//! One asset's computation (RP2's `compute_tax`).
//!
//! [`PreparedAsset`] holds the visible, ordered entry sets; [`compute_asset`]
//! runs the taxable-event pass and derives the outputs in the order RP2's
//! `ComputedData` does, so the first failure is RP2's first failure.

use std::borrow::Cow;
use std::collections::HashSet;

use crate::austria;
use crate::cursor::{Cursor, Fragment};
use crate::decimal::Decimal;
use crate::entry::Entry;
use crate::error::{EngineError, EngineResult};
use crate::manifest::kind_has_token;
use crate::methods::{lot_method, LotView};
use crate::model::{
    AssetInput, AssetOutput, CountrySpec, Dec, EntryKind, ErrorBody, FeeUnits, Holding, MethodName,
    PoolScope, RulesSpec, SwapValues,
};
use crate::model::{GiftLosses, Given, Received};
use crate::num::{lt13, zero};
use crate::outputs::{
    balances, fragment_values, gain_loss_out, open_positions, sold_percentages, yearly,
};
use crate::par;
use crate::time::{elapsed_days, CivilDate, Timestamp};
use kassiber_tax_countries::calendar::{self, Zone};

/// The country rules the lot engine consults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Country {
    /// Kassiber's generic country.
    Generic {
        /// Whole days a lot must be held for a long-term gain.
        long_term_days: i64,
    },
    /// Austria. Holding-period and Alt/Neu semantics live in its
    /// classification hooks, not in the lot engine.
    Austria,
    /// The United States: long-term once held more than one year by UTC
    /// calendar date.
    UnitedStates,
    /// A registry country's rules.
    Rules(Rules),
}

/// A registry country's rules, read from its [`RulesSpec`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rules {
    /// The zone whose local dates count the holding period.
    pub zone: Zone,
    pub holding: Holding,
    /// The tax year's first day (month, day).
    pub tax_year_start: (u8, u8),
    /// The only pool scope the country allows, if it allows one.
    pub pool_scope: Option<PoolScope>,
    pub fee_units: FeeUnits,
    pub swap_values: SwapValues,
    pub gifts: Gifts,
}

/// What gifts and donations are.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Gifts {
    pub given: Given,
    pub donated: Given,
    pub received: Received,
    pub losses: GiftLosses,
}

impl Gifts {
    /// Whether an outbound row of `kind` is a disposal at its value rather
    /// than a quarantined disposition (lost, stolen and other kinds always
    /// are).
    pub fn books_outbound(&self, kind: &str) -> bool {
        if kind_has_token(kind, &["gift"]) {
            self.given == Given::MarketValue
        } else if kind_has_token(kind, &["donat"]) {
            self.donated == Given::MarketValue
        } else {
            false
        }
    }

    /// Whether an inbound row of `kind` is a gift the country refuses to
    /// book without the donor's data.
    pub fn refuses_inbound(&self, kind: &str) -> bool {
        self.received == Received::Quarantine && kind_has_token(kind, &["gift"])
    }
}

impl Rules {
    /// Reads the request's rules; a zone the registry has no calendar for
    /// is a request error.
    pub fn from_spec(spec: &RulesSpec) -> EngineResult<Self> {
        let zone = calendar::zone(&spec.timezone).ok_or_else(|| {
            EngineError::request(format!(
                "the country rules name the time zone {:?}, which has no calendar",
                spec.timezone
            ))
        })?;
        let tax_year_start = match spec.tax_year_start.as_deref() {
            None => (1, 1),
            Some(text) => calendar::Date::parse(&format!("2000-{text}"))
                .filter(|_| text.len() == 5)
                .map(|date| (date.month as u8, date.day as u8))
                .ok_or_else(|| {
                    EngineError::request(format!(
                        "the country rules' tax year starts on {text:?}, which is not a MM-DD day"
                    ))
                })?,
        };
        Ok(Rules {
            zone,
            holding: spec.holding,
            tax_year_start,
            pool_scope: spec.pool_scope,
            fee_units: spec.fee_units,
            swap_values: spec.swap_values,
            gifts: Gifts {
                given: spec.gifts_given,
                donated: spec.donations_given,
                received: spec.gifts_received,
                losses: spec.gift_losses,
            },
        })
    }

    /// The local date of `ts` in the rules' zone. An instant before the
    /// zone's calendar starts fails rather than take a guessed offset.
    pub fn local_date(&self, ts: &Timestamp) -> EngineResult<CivilDate> {
        self.zone
            .local_date(ts.us)
            .map_err(|error| EngineError::value(format!("{error}: {}", ts.raw)))
    }
}

impl Country {
    /// Reads the request's country.
    pub fn from_spec(spec: &CountrySpec) -> EngineResult<Self> {
        Ok(match spec {
            // Any period above i64::MAX days behaves identically.
            CountrySpec::Generic { long_term_days } => Country::Generic {
                long_term_days: i64::try_from(*long_term_days).unwrap_or(i64::MAX),
            },
            CountrySpec::At {} => Country::Austria,
            CountrySpec::Us {} => Country::UnitedStates,
            CountrySpec::Rules(spec) => Country::Rules(Rules::from_spec(spec)?),
        })
    }

    /// RP2's `get_long_term_capital_gain_period()`; Austria's is
    /// `sys.maxsize`, so it is never reached.
    pub fn long_term_days(self) -> i64 {
        match self {
            Country::Generic { long_term_days } => long_term_days,
            Country::Austria
            | Country::Rules(Rules {
                holding: Holding::Never,
                ..
            }) => i64::MAX,
            // Only as a day count for callers that need one; the rule itself
            // is `is_long_term`'s calendar test.
            Country::UnitedStates
            | Country::Rules(Rules {
                holding: Holding::MoreThanOneYear,
                ..
            }) => 365,
        }
    }

    /// The rules the calendar countries share: the United States' are a
    /// UTC calendar's "more than one year".
    fn calendar(self) -> Option<Rules> {
        match self {
            Country::UnitedStates => Some(Rules {
                zone: calendar::UTC,
                holding: Holding::MoreThanOneYear,
                tax_year_start: (1, 1),
                pool_scope: None,
                fee_units: FeeUnits::Basis,
                swap_values: SwapValues::Own,
                gifts: Gifts::default(),
            }),
            Country::Rules(rules) => Some(rules),
            Country::Generic { .. } | Country::Austria => None,
        }
    }

    /// The tax year of a taxable event: a rules country's, in its zone from
    /// its start day; every other country's is the year in the timestamp's
    /// own offset, as RP2 reads it.
    pub fn tax_year(self, ts: &Timestamp) -> EngineResult<i32> {
        match self {
            Country::Rules(rules) => Ok(kassiber_tax_countries::years::tax_year(
                rules.local_date(ts)?,
                rules.tax_year_start,
            )),
            _ => Ok(ts.year()),
        }
    }

    /// What fee units paid in the asset are; the generic rule but for a
    /// rules country that changes it.
    pub fn fee_units(self) -> FeeUnits {
        match self {
            Country::Rules(rules) => rules.fee_units,
            _ => FeeUnits::Basis,
        }
    }

    /// The values of a taxable cross-asset exchange; each leg's own but for
    /// a rules country that changes it.
    pub fn swap_values(self) -> SwapValues {
        match self {
            Country::Rules(rules) => rules.swap_values,
            _ => SwapValues::Own,
        }
    }

    /// What gifts and donations are; quarantined outbound, acquisitions
    /// inbound, but for a rules country that changes it.
    pub fn gifts(self) -> Gifts {
        match self {
            Country::Rules(rules) => rules.gifts,
            _ => Gifts::default(),
        }
    }

    /// Whether a lot acquired at `acquired` and disposed of at `disposed`
    /// is long-term. RP2 counts whole elapsed days against
    /// `long_term_days()`; a calendar country counts local dates in its
    /// zone (`calendar::held_more_than_one_year`), so a disposal on the
    /// anniversary is still short-term where RP2's US plugin says
    /// long-term.
    pub fn is_long_term(self, disposed: &Timestamp, acquired: &Timestamp) -> EngineResult<bool> {
        match self.calendar() {
            Some(Rules {
                holding: Holding::Never,
                ..
            }) => Ok(false),
            Some(rules) => Ok(calendar::held_more_than_one_year(
                rules.local_date(acquired)?,
                rules.local_date(disposed)?,
            )),
            None => Ok(elapsed_days(disposed, acquired) >= i128::from(self.long_term_days())),
        }
    }
}

/// One asset's constructed entries: what the adapter holds after building
/// its transaction sets, before any computation.
pub(crate) struct ParsedAsset<'e> {
    pub asset: String,
    /// Every entry, in insertion order: constructed here, or borrowed from
    /// the ledger's walk, which constructed them.
    pub entries: Cow<'e, [Entry]>,
}

impl ParsedAsset<'static> {
    /// Runs every constructor and the per-set duplicate-row check, failing
    /// as building RP2's transaction sets does.
    pub fn new(input: &AssetInput) -> EngineResult<Self> {
        let entries = input
            .entries
            .iter()
            .map(|entry| Entry::from_input(entry, None))
            .collect::<EngineResult<Vec<_>>>()?;
        let parsed = ParsedAsset {
            asset: input.asset.clone(),
            entries: Cow::Owned(entries),
        };
        parsed.check()?;
        Ok(parsed)
    }
}

impl ParsedAsset<'_> {
    /// The checks building RP2's transaction sets runs once every
    /// constructor has: each entry is of the set's asset, and no set gets a
    /// row twice.
    pub fn check(&self) -> EngineResult<()> {
        for entry in self.entries.iter() {
            if entry.asset != self.asset {
                return Err(EngineError::value(format!(
                    "Attempting to add a {} entry to a {} set",
                    entry.asset, self.asset
                )));
            }
        }
        // Each TransactionSet rejects a repeated row as it is added.
        for kind in [EntryKind::In, EntryKind::Out, EntryKind::Intra] {
            reject_duplicates(self.entries.iter().filter(|e| e.kind == kind))?;
        }
        Ok(())
    }

    /// Indices of the entries of `kind` RP2's set iteration yields, in its
    /// order.
    pub fn visible(&self, kind: EntryKind) -> Vec<usize> {
        visible_sorted(&self.entries, kind)
    }
}

/// One asset's validated entries and RP2's views of them.
pub(crate) struct PreparedAsset<'e> {
    pub asset: String,
    /// Every entry, in insertion order.
    pub entries: Cow<'e, [Entry]>,
    /// Visible IN, OUT, and INTRA entries, each stably sorted by instant.
    pub ins: Vec<usize>,
    pub outs: Vec<usize>,
    pub intras: Vec<usize>,
    /// The lot list (visible IN entries in order).
    pub lots: Vec<LotView>,
    /// Taxable events in processing order.
    pub events: Vec<usize>,
}

/// Indices of the visible entries of `kind`, stably sorted by instant.
fn visible_sorted(entries: &[Entry], kind: EntryKind) -> Vec<usize> {
    let mut indices: Vec<usize> = (0..entries.len())
        .filter(|&i| entries[i].kind == kind && entries[i].ts.is_visible())
        .collect();
    indices.sort_by_key(|&i| entries[i].ts.us);
    indices
}

/// RP2's "Entry already added" check for a sequence of additions.
fn reject_duplicates<'a>(entries: impl IntoIterator<Item = &'a Entry>) -> EngineResult<()> {
    let mut seen = HashSet::new();
    for entry in entries {
        if !seen.insert(entry.row) {
            return Err(EngineError::value(format!(
                "Entry already added: {}",
                entry.text
            )));
        }
    }
    Ok(())
}

impl<'e> PreparedAsset<'e> {
    /// Builds the views `compute_tax` iterates from constructed entries.
    /// Fails as RP2 does before the first taxable event: on a duplicate
    /// row or no visible lot.
    pub fn from_parsed(parsed: ParsedAsset<'e>) -> EngineResult<Self> {
        let ParsedAsset { asset, entries } = parsed;
        let ins = visible_sorted(&entries, EntryKind::In);
        let outs = visible_sorted(&entries, EntryKind::Out);
        let intras = visible_sorted(&entries, EntryKind::Intra);

        // The combined set adds IN, then OUT, then INTRA entries and rejects
        // a row seen in another set; its stable sort by instant puts earn
        // receipts before disposals before move fees at one instant.
        let mut all: Vec<usize> = ins.iter().chain(&outs).chain(&intras).copied().collect();
        reject_duplicates(all.iter().map(|&i| &entries[i]))?;
        all.sort_by_key(|&i| entries[i].ts.us);
        let mut events = Vec::new();
        for index in all {
            if entries[index].is_taxable()? {
                events.push(index);
            }
        }

        let lots = ins
            .iter()
            .map(|&index| {
                let entry = &entries[index];
                let (Some(crypto_in), Some(fiat_in_with_fee)) =
                    (entry.crypto_in(), entry.fiat_in_with_fee())
                else {
                    return Err(EngineError::runtime(
                        "Internal error: lot is not an InTransaction",
                    ));
                };
                Ok(LotView {
                    entry: index,
                    row: entry.row,
                    us: entry.ts.us,
                    posix: entry.ts.posix_seconds()?,
                    crypto_in: crypto_in.clone(),
                    fiat_in_with_fee: fiat_in_with_fee.clone(),
                })
            })
            .collect::<EngineResult<Vec<_>>>()?;
        if lots.is_empty() {
            return Err(EngineError::runtime(
                "Internal error: AVL tree has no root node",
            ));
        }
        Ok(PreparedAsset {
            asset,
            entries,
            ins,
            outs,
            intras,
            lots,
            events,
        })
    }
}

/// RP2's `compute_tax` for one asset.
pub(crate) fn compute_asset(
    input: &AssetInput,
    country: Country,
    method: MethodName,
    scope: PoolScope,
) -> EngineResult<AssetOutput> {
    compute_parsed(ParsedAsset::new(input)?, country, method, scope)
}

/// The first day the United States identifies units per wallet or account
/// (Treas. Reg. § 1.1012-1(j)), in UTC days since 1970-01-01.
const US_WALLET_SCOPE_FROM: i64 = 20_089;

/// A US book on the global scope may not dispose of anything from 2025:
/// the move to per-wallet identification (Rev. Proc. 2024-28) is not
/// modelled, so it fails closed rather than carry universal lots across.
fn check_us_global_scope(
    prepared: &PreparedAsset,
    country: Country,
    scope: PoolScope,
) -> EngineResult<()> {
    if country != Country::UnitedStates || scope != PoolScope::Global {
        return Ok(());
    }
    for &index in &prepared.events {
        let event = &prepared.entries[index];
        if !event.is_earning()
            && crate::time::utc_date(event.ts.us).to_days() >= US_WALLET_SCOPE_FROM
        {
            return Err(EngineError::value(format!(
                "US books identify units per wallet from 2025 (Treas. Reg. § 1.1012-1(j)); the global pool scope cannot dispose on or after 2025-01-01: {}",
                event.text
            )));
        }
    }
    Ok(())
}

/// A rules country that allows one pool scope refuses the other, so a
/// book never gets the wrong pool's figures (Germany's global pool would
/// take another wallet's lots, BMF 6 March 2025 Rn. 62).
fn check_rules_scope(country: Country, scope: PoolScope) -> EngineResult<()> {
    match country {
        Country::Rules(Rules {
            pool_scope: Some(required),
            ..
        }) if required != scope => Err(EngineError::value(format!(
            "This country's rules identify units {}; the {} pool scope is not allowed",
            if required == PoolScope::Wallet {
                "per wallet"
            } else {
                "across all wallets"
            },
            if scope == PoolScope::Wallet {
                "wallet"
            } else {
                "global"
            }
        ))),
        _ => Ok(()),
    }
}

/// RP2's `compute_tax` for one asset whose entries are constructed and
/// checked.
pub(crate) fn compute_parsed(
    parsed: ParsedAsset<'_>,
    country: Country,
    method: MethodName,
    scope: PoolScope,
) -> EngineResult<AssetOutput> {
    let prepared = PreparedAsset::from_parsed(parsed)?;
    check_us_global_scope(&prepared, country, scope)?;
    check_rules_scope(country, scope)?;
    if let Some(matching) = crate::matching::method(method) {
        // A country method computes the asset as a whole; each lot's own
        // basis is its acquisition's and open position's.
        let matched = matching.compute(&prepared, country, scope)?;
        let basis: Vec<Decimal> = prepared
            .lots
            .iter()
            .map(|lot| lot.fiat_in_with_fee.clone())
            .collect();
        let open_basis = matched.open_basis.as_ref().unwrap_or(&basis);
        let mut output = finish(&prepared, &matched.fragments, &basis, open_basis, country)?;
        output.wallet_holdings = matched.wallet_holdings;
        return Ok(output);
    }
    if scope == PoolScope::Wallet {
        // Lot methods only: each lot's basis is its own, so the effective
        // and open-position bases are the lots' (RP2's lot-method views).
        let (fragments, holdings) = crate::wallets::fragments(
            &prepared.entries,
            &prepared.lots,
            &prepared.ins,
            &prepared.events,
            method,
        )?;
        let basis: Vec<Decimal> = prepared
            .lots
            .iter()
            .map(|lot| lot.fiat_in_with_fee.clone())
            .collect();
        let mut output = finish(&prepared, &fragments, &basis, &basis, country)?;
        output.wallet_holdings = Some(holdings);
        return Ok(output);
    }
    let mut cursor = Cursor::new(
        &prepared.entries,
        &prepared.lots,
        &prepared.events,
        lot_method(method, &prepared.entries, &prepared.lots),
    );
    let mut fragments = Vec::new();
    cursor.run(&mut fragments)?;
    // RP2 replays the whole pass to snapshot open-position basis. Without
    // carried basis (a single asset has none) the replay starts from the
    // same state and repeats this pass exactly, for lot and pool methods
    // alike, so the snapshot is taken from this cursor.
    let open_basis = cursor.open_position_basis()?.unwrap_or_default();
    let effective_basis = cursor.effective_basis();
    finish(
        &prepared,
        &fragments,
        &effective_basis,
        &open_basis,
        country,
    )
}

/// Derives the `ComputedData` views, in RP2's order.
pub(crate) fn finish(
    prepared: &PreparedAsset,
    fragments: &[Fragment],
    effective_basis: &[Decimal],
    open_basis: &[Decimal],
    country: Country,
) -> EngineResult<AssetOutput> {
    let entries = &prepared.entries;
    let lots = &prepared.lots;
    // Each fragment's values are its own, so they are derived concurrently
    // and taken in order, up to the first failure.
    let values = par::map(fragments, 256, |fragment| {
        fragment_values(fragment, entries, lots, country)
    })
    .into_iter()
    .map(par::unwrap)
    .collect::<EngineResult<Vec<_>>>()?;
    let yearly = yearly(&prepared.asset, fragments, &values, entries, country)?;
    for basis in open_basis {
        if lt13(basis, &zero())? {
            return Err(EngineError::value(format!(
                "Parameter 'open_position_basis' has non-positive value {basis}"
            )));
        }
    }
    let balances = balances(entries, &prepared.ins, &prepared.outs, &prepared.intras)?;
    let sold = sold_percentages(fragments, lots)?;
    let mut gain_losses: Vec<_> = fragments
        .iter()
        .zip(&values)
        .map(|(fragment, value)| gain_loss_out(fragment, value, entries, lots))
        .collect();
    if country == Country::Austria {
        // Kassiber classifies every row of an Austrian book, whatever the
        // method; a failure aborts the report only when Kassiber reaches it.
        for ((out, fragment), value) in gain_losses.iter_mut().zip(fragments).zip(&values) {
            match austria::classify(fragment, &value.fiat_gain, entries, lots) {
                Ok(category) => out.at_category = Some(category.as_str().to_owned()),
                Err(error) => {
                    out.at_category_error = Some(ErrorBody {
                        class: error.class,
                        message: error.message,
                    })
                }
            }
        }
    }
    Ok(AssetOutput {
        asset: prepared.asset.clone(),
        in_transactions: lots.iter().map(|lot| lot.row).collect(),
        in_fiat_in_with_fee: effective_basis.iter().cloned().map(Dec).collect(),
        gain_losses,
        yearly,
        open_positions: open_positions(lots, &sold, open_basis),
        balances,
        wallet_holdings: None,
    })
}
