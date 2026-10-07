//! What the engine knows by name: kind tables, marker names, the Austrian
//! cutoff and holding period, categories, entry types, and quarantine
//! reasons.
//!
//! Python spells the same names where it still needs them
//! (`transaction_kinds.INBOUND_KIND_TO_RP2_TYPE`, `journal_markers`,
//! `austrian.AT_NEU_CUTOFF` and `AT_CATEGORY_TO_KENNZAHL`,
//! `tax_policy.AUSTRIAN_LONG_TERM_DAYS`, `exit_tax.RECOGNIZED_ENTRY_TYPES`,
//! `quarantine_catalog`).
//! [`manifest`] collects them into one document so tests can compare those
//! copies with the engine's. The tables port `transaction_kinds.py:8-23`,
//! `engines/rp2.py` (the kind tokens `:84-104`, the default inbound type
//! `:391-393`, the Austrian zero-cost kinds `:1079`, the rail reason `:71`),
//! `journal_markers.py:16-31`, and `austrian.py:55`.

use serde::ser::Serializer;
use serde::Serialize;

use crate::austria::{AtCategory, DEFAULT_POOL, NEU_CUTOFF_US};
use crate::engine::Country;
use crate::entry::TransactionType;
use crate::ENGINE_VERSION;

/// `INBOUND_KIND_TO_RP2_TYPE`: the type of an inbound row by its normalized
/// kind. A plain deposit is not listed: it defaults to `BUY` without that
/// proving an external origin.
pub const INBOUND_KIND_TYPES: [(&str, TransactionType); 12] = [
    ("buy", TransactionType::Buy),
    ("airdrop", TransactionType::Airdrop),
    ("hardfork", TransactionType::Hardfork),
    ("hard_fork", TransactionType::Hardfork),
    ("income", TransactionType::Income),
    ("interest", TransactionType::Interest),
    ("lending_interest", TransactionType::Interest),
    ("mining", TransactionType::Mining),
    ("mining_reward", TransactionType::Mining),
    ("routing_income", TransactionType::Income),
    ("staking", TransactionType::Staking),
    // Compensation is acquisition basis; employment-income reporting stays
    // outside Kassiber.
    ("wages", TransactionType::Buy),
];

/// The type of an inbound kind [`INBOUND_KIND_TYPES`] does not list.
pub const DEFAULT_INBOUND_TYPE: TransactionType = TransactionType::Buy;

/// Inbound kinds containing one of these tokens look like income; when the
/// kind table does not list them they are quarantined
/// (`unclassified_income_kind`) rather than booked as a purchase.
pub const INCOME_LIKE_KIND_TOKENS: [&str; 7] = [
    "reward", "referral", "bonus", "cashback", "rebate", "dividend", "yield",
];

/// Outbound kinds containing one of these tokens are dispositions but not
/// market sales; they are quarantined (`non_sale_disposal_kind`) rather than
/// taxed at market value.
pub const NON_SALE_DISPOSAL_KIND_TOKENS: [&str; 6] =
    ["gift", "donat", "lost", "stolen", "theft", "non_taxable"];

/// Inbound kinds an Austrian book quarantines
/// (`acquisition_valuation_unsupported`): their zero-cost valuation is not
/// supported.
pub const AT_ZERO_COST_KINDS: [&str; 3] = ["airdrop", "hardfork", "hard_fork"];

/// `journal_markers.MARKER_REGIME`: the regime of an event or fee slice.
pub const MARKER_REGIME: &str = "at_regime";
/// `journal_markers.MARKER_REGIME_BASIS`: why a disposal has its regime.
pub const MARKER_REGIME_BASIS: &str = "at_regime_basis";
/// `journal_markers.REGIME_BASIS_ELECTION`: the wallet held Alt and Neu, and
/// Kassiber exercised the designation right (Neu first).
pub const REGIME_BASIS_ELECTION: &str = "wahlrecht";
/// `journal_markers.MARKER_POOL`: the cost-basis pool.
pub const MARKER_POOL: &str = "at_pool";
/// `journal_markers.MARKER_SWAP_LINK`: the swap pair of a Neu swap leg.
pub const MARKER_SWAP_LINK: &str = "at_swap_link";
/// `journal_markers.MARKER_ALT_OUT`: the Alt msat that left a transfer's
/// source.
pub const MARKER_ALT_OUT: &str = "at_alt_out";
/// `journal_markers.MARKER_ALT_IN`: the Alt msat a transfer's destination
/// received.
pub const MARKER_ALT_IN: &str = "at_alt_in";

/// The Austrian disposal categories, in `AT_CATEGORY_TO_KENNZAHL` order.
pub(crate) const AT_CATEGORIES: [AtCategory; 7] = [
    AtCategory::IncomeGeneral,
    AtCategory::IncomeCapitalYield,
    AtCategory::NeuGain,
    AtCategory::NeuLoss,
    AtCategory::NeuSwap,
    AtCategory::AltSpekulation,
    AtCategory::AltTaxfree,
];

/// A reason the ledger quarantines a row for. These are the reasons of the
/// adapter's live paths; the normalizer's dead branches are not ported.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum QuarantineReason {
    /// An event or a fee-bearing transfer without a price.
    MissingSpotPrice,
    /// Coarse pricing on a profile that requires its review.
    PricingReviewRequired,
    /// A privacy hop outside a reviewed coinjoin pair.
    PrivacyHopUnresolved,
    /// A spend that conflicts with another (a custody hold).
    ConflictingSpend,
    /// An unconfirmed on-chain spend (a custody hold).
    PendingOnchainConfirmation,
    /// A transfer leg blocked by another leg of its atomic group.
    DerivedTransferGroupBlocked,
    /// More is spent from a wallet than it holds.
    InsufficientLots,
    /// The wallet holds the coins, but not enough of them are priced.
    MissingCostBasis,
    /// A disposal after a quarantined row that taints the lot state.
    BasisProvenanceIncomplete,
    /// An inbound kind that looks like income but is not classified.
    UnclassifiedIncomeKind,
    /// An outbound kind that is a disposition but not a sale.
    NonSaleDisposalKind,
    /// An Austrian airdrop or hard fork (zero-cost valuation).
    AcquisitionValuationUnsupported,
    /// `austrian.AT_SWAP_QUARANTINE_REASON`: a Neu swap that cannot carry
    /// its basis.
    AtSwapBasisCarryUnresolved,
    /// `GENERIC_BITCOIN_RAIL_QUARANTINE_REASON`: a carrying-value Bitcoin
    /// rail pair without a carried basis.
    BitcoinRailCarryBasisUnresolved,
}

impl QuarantineReason {
    /// Every reason: normalize, then the walk's gates, then the carries.
    pub const ALL: [QuarantineReason; 14] = [
        QuarantineReason::MissingSpotPrice,
        QuarantineReason::PricingReviewRequired,
        QuarantineReason::PrivacyHopUnresolved,
        QuarantineReason::ConflictingSpend,
        QuarantineReason::PendingOnchainConfirmation,
        QuarantineReason::DerivedTransferGroupBlocked,
        QuarantineReason::InsufficientLots,
        QuarantineReason::MissingCostBasis,
        QuarantineReason::BasisProvenanceIncomplete,
        QuarantineReason::UnclassifiedIncomeKind,
        QuarantineReason::NonSaleDisposalKind,
        QuarantineReason::AcquisitionValuationUnsupported,
        QuarantineReason::AtSwapBasisCarryUnresolved,
        QuarantineReason::BitcoinRailCarryBasisUnresolved,
    ];

    /// The stored reason name.
    pub fn as_str(self) -> &'static str {
        match self {
            QuarantineReason::MissingSpotPrice => "missing_spot_price",
            QuarantineReason::PricingReviewRequired => "pricing_review_required",
            QuarantineReason::PrivacyHopUnresolved => "privacy_hop_unresolved",
            QuarantineReason::ConflictingSpend => "conflicting_spend",
            QuarantineReason::PendingOnchainConfirmation => "pending_onchain_confirmation",
            QuarantineReason::DerivedTransferGroupBlocked => "derived_transfer_group_blocked",
            QuarantineReason::InsufficientLots => "insufficient_lots",
            QuarantineReason::MissingCostBasis => "missing_cost_basis",
            QuarantineReason::BasisProvenanceIncomplete => "basis_provenance_incomplete",
            QuarantineReason::UnclassifiedIncomeKind => "unclassified_income_kind",
            QuarantineReason::NonSaleDisposalKind => "non_sale_disposal_kind",
            QuarantineReason::AcquisitionValuationUnsupported => {
                "acquisition_valuation_unsupported"
            }
            QuarantineReason::AtSwapBasisCarryUnresolved => "at_swap_basis_carry_unresolved",
            QuarantineReason::BitcoinRailCarryBasisUnresolved => {
                "bitcoin_rail_carry_basis_unresolved"
            }
        }
    }
}

impl Serialize for QuarantineReason {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

/// `GENERIC_BITCOIN_RAIL_QUARANTINE_REASON` (`engines/rp2.py:71`).
pub const RAIL_QUARANTINE_REASON: QuarantineReason =
    QuarantineReason::BitcoinRailCarryBasisUnresolved;

/// `AT_SWAP_QUARANTINE_REASON` (`austrian.py:55`).
pub const SWAP_QUARANTINE_REASON: QuarantineReason = QuarantineReason::AtSwapBasisCarryUnresolved;

/// Which template built a quarantine's detail. Python adds the row facts a
/// template needs by this kind, never by the reason, because one reason can
/// come from two templates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DetailKind {
    /// `_pricing_review_detail`: Python adds the row's pricing fields.
    PricingReview,
    /// `_append_privacy_hop_quarantine`: Python adds the privacy evidence.
    PrivacyHop,
    /// A custody hold: Python adds the row's external id.
    Hold,
    /// `_quarantine_partner_leg`: the other leg of a transfer; nothing added.
    PartnerLeg,
    /// Every other template; nothing added.
    Plain,
}

impl DetailKind {
    /// Every detail kind.
    pub const ALL: [DetailKind; 5] = [
        DetailKind::PricingReview,
        DetailKind::PrivacyHop,
        DetailKind::Hold,
        DetailKind::PartnerLeg,
        DetailKind::Plain,
    ];

    /// The name the response uses.
    pub fn as_str(self) -> &'static str {
        match self {
            DetailKind::PricingReview => "pricing_review",
            DetailKind::PrivacyHop => "privacy_hop",
            DetailKind::Hold => "hold",
            DetailKind::PartnerLeg => "partner_leg",
            DetailKind::Plain => "plain",
        }
    }
}

impl Serialize for DetailKind {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

/// The type of a journal entry (`exit_tax.RECOGNIZED_ENTRY_TYPES`, which
/// `_append_rp2_journal_entries` emits, `engines/rp2.py:1383-1598`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EntryType {
    /// A lot: a purchase or another non-earn receipt.
    Acquisition,
    /// An earn receipt (income at receipt).
    Income,
    /// A taxable disposal.
    Disposal,
    /// A fee-only spend.
    Fee,
    /// The destination leg of a transfer.
    TransferIn,
    /// The source leg of a transfer.
    TransferOut,
    /// The fee of a transfer.
    TransferFee,
}

impl EntryType {
    /// Every entry type, in `RECOGNIZED_ENTRY_TYPES` source order.
    pub const ALL: [EntryType; 7] = [
        EntryType::Acquisition,
        EntryType::Income,
        EntryType::Disposal,
        EntryType::Fee,
        EntryType::TransferIn,
        EntryType::TransferOut,
        EntryType::TransferFee,
    ];

    /// The stored `entry_type`.
    pub fn as_str(self) -> &'static str {
        match self {
            EntryType::Acquisition => "acquisition",
            EntryType::Income => "income",
            EntryType::Disposal => "disposal",
            EntryType::Fee => "fee",
            EntryType::TransferIn => "transfer_in",
            EntryType::TransferOut => "transfer_out",
            EntryType::TransferFee => "transfer_fee",
        }
    }
}

impl Serialize for EntryType {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

/// `_rp2_in_transaction_type`: the type an inbound row of `kind` books as.
pub fn inbound_transaction_type(kind: &str) -> TransactionType {
    INBOUND_KIND_TYPES
        .iter()
        .find(|(name, _)| *name == kind)
        .map_or(DEFAULT_INBOUND_TYPE, |&(_, transaction_type)| {
            transaction_type
        })
}

/// `_kind_has_token`: whether a non-empty `kind` contains one of `tokens`.
pub fn kind_has_token(kind: &str, tokens: &[&str]) -> bool {
    !kind.is_empty() && tokens.iter().any(|token| kind.contains(token))
}

/// RP2's name of a transaction type (`TransactionType.BUY.name`).
fn type_name(transaction_type: TransactionType) -> String {
    transaction_type.value().to_ascii_uppercase()
}

/// The marker names of journal descriptions.
#[derive(Clone, Debug, Serialize)]
pub struct Markers {
    pub regime: &'static str,
    pub regime_basis: &'static str,
    pub pool: &'static str,
    pub swap_link: &'static str,
    pub alt_out: &'static str,
    pub alt_in: &'static str,
}

/// The `manifest` document.
#[derive(Clone, Debug, Serialize)]
pub struct Manifest {
    pub engine_version: &'static str,
    /// [`INBOUND_KIND_TYPES`] as an object, in table order.
    #[serde(serialize_with = "ordered_object")]
    pub inbound_kind_types: Vec<(&'static str, String)>,
    pub default_inbound_type: String,
    pub income_like_kind_tokens: &'static [&'static str],
    pub non_sale_disposal_kind_tokens: &'static [&'static str],
    pub at_zero_cost_kinds: &'static [&'static str],
    pub markers: Markers,
    pub regime_basis_election: &'static str,
    /// `AT_NEU_CUTOFF` in microseconds since the Unix epoch.
    pub neu_cutoff_us: i64,
    /// Austria's holding period in days (`sys.maxsize`: never long-term).
    pub at_long_term_days: i64,
    /// The Austrian pool id written for Kassiber's global pool.
    pub default_pool: &'static str,
    pub at_categories: Vec<&'static str>,
    pub entry_types: &'static [EntryType],
    pub quarantine_reasons: &'static [QuarantineReason],
    pub detail_kinds: &'static [DetailKind],
    pub rail_quarantine_reason: QuarantineReason,
    pub swap_quarantine_reason: QuarantineReason,
}

fn ordered_object<S: Serializer>(
    pairs: &[(&'static str, String)],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.collect_map(pairs.iter().map(|(key, value)| (key, value)))
}

/// The engine's names and tables.
pub fn manifest() -> Manifest {
    Manifest {
        engine_version: ENGINE_VERSION,
        inbound_kind_types: INBOUND_KIND_TYPES
            .iter()
            .map(|&(kind, transaction_type)| (kind, type_name(transaction_type)))
            .collect(),
        default_inbound_type: type_name(DEFAULT_INBOUND_TYPE),
        income_like_kind_tokens: &INCOME_LIKE_KIND_TOKENS,
        non_sale_disposal_kind_tokens: &NON_SALE_DISPOSAL_KIND_TOKENS,
        at_zero_cost_kinds: &AT_ZERO_COST_KINDS,
        markers: Markers {
            regime: MARKER_REGIME,
            regime_basis: MARKER_REGIME_BASIS,
            pool: MARKER_POOL,
            swap_link: MARKER_SWAP_LINK,
            alt_out: MARKER_ALT_OUT,
            alt_in: MARKER_ALT_IN,
        },
        regime_basis_election: REGIME_BASIS_ELECTION,
        neu_cutoff_us: NEU_CUTOFF_US,
        at_long_term_days: Country::Austria.long_term_days(),
        default_pool: DEFAULT_POOL,
        at_categories: AT_CATEGORIES
            .iter()
            .map(|category| category.as_str())
            .collect(),
        entry_types: &EntryType::ALL,
        quarantine_reasons: &QuarantineReason::ALL,
        detail_kinds: &DetailKind::ALL,
        rail_quarantine_reason: RAIL_QUARANTINE_REASON,
        swap_quarantine_reason: SWAP_QUARANTINE_REASON,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::austria::{has_swap_link, pool_id, regime_from_notes, swap_link_id, Regime};
    use crate::entry::Entry;
    use crate::model::EntryInput;
    use crate::time::CivilDate;

    fn in_entry(transaction_type: &str) -> EntryInput {
        let json = format!(
            r#"{{"kind": "in", "row": 1, "asset": "BTC", "transaction_type": "{transaction_type}",
                "exchange": "A", "holder": "H", "spot_price": "100E0", "crypto_in": "1E0",
                "ts": {{"raw": "2024-01-01T00:00:00Z", "us": 1704067200000000, "offset_s": 0,
                        "date": "2024-01-01", "year": 2024, "vienna_date": "2024-01-01",
                        "display": "2024-01-01 00:00:00+00:00",
                        "iso": "2024-01-01T00:00:00+00:00"}}}}"#
        );
        serde_json::from_str(&json).expect("entry input")
    }

    fn is_snake_case(name: &str) -> bool {
        !name.is_empty() && name.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
    }

    #[test]
    fn inbound_kinds_book_as_types_an_acquisition_accepts() {
        let kinds: BTreeSet<&str> = INBOUND_KIND_TYPES.iter().map(|(kind, _)| *kind).collect();
        assert_eq!(kinds.len(), INBOUND_KIND_TYPES.len(), "duplicate kind");
        for (kind, transaction_type) in INBOUND_KIND_TYPES {
            // Keys are what `normalized_transaction_kind` returns.
            assert!(is_snake_case(kind), "{kind}");
            let input = in_entry(&type_name(transaction_type));
            let entry =
                Entry::from_input(&input, None).unwrap_or_else(|e| panic!("{kind}: {}", e.message));
            assert_eq!(entry.transaction_type, transaction_type, "{kind}");
            assert_eq!(inbound_transaction_type(kind), transaction_type, "{kind}");
        }
        let input = in_entry(&type_name(DEFAULT_INBOUND_TYPE));
        assert!(Entry::from_input(&input, None).is_ok());
        // Python's `.get(kind, "BUY")` matches the whole kind exactly: a
        // kind that starts with, ends with, or contains a key, or spells it
        // in another case, is not that key.
        for kind in [
            "deposit",
            "",
            "BUY",
            "hard-fork",
            "reward",
            "staking_reward",
            "income_tax",
            "buy_back",
            "mining_pool_payout",
            "liquid_staking",
            "xstakingx",
            "STAKING",
        ] {
            assert_eq!(
                inbound_transaction_type(kind),
                TransactionType::Buy,
                "{kind}"
            );
        }
    }

    #[test]
    fn kind_tokens_match_substrings_of_non_empty_kinds() {
        assert!(kind_has_token("referral_reward", &INCOME_LIKE_KIND_TOKENS));
        assert!(kind_has_token("donation", &NON_SALE_DISPOSAL_KIND_TOKENS));
        assert!(kind_has_token(
            "non_taxable_out",
            &NON_SALE_DISPOSAL_KIND_TOKENS
        ));
        assert!(!kind_has_token(
            "withdrawal",
            &NON_SALE_DISPOSAL_KIND_TOKENS
        ));
        assert!(!kind_has_token("", &[""]));
        assert!(kind_has_token("x", &[""]));
        for token in INCOME_LIKE_KIND_TOKENS
            .iter()
            .chain(&NON_SALE_DISPOSAL_KIND_TOKENS)
            .chain(&AT_ZERO_COST_KINDS)
        {
            assert!(is_snake_case(token), "{token}");
        }
        // Every zero-cost kind books as an earn type when the book is not
        // Austrian.
        for kind in AT_ZERO_COST_KINDS {
            assert!(inbound_transaction_type(kind).is_earn(), "{kind}");
        }
    }

    #[test]
    fn marker_names_are_the_ones_the_lot_engine_reads() {
        let notes = format!(
            "{MARKER_REGIME}=neu {MARKER_REGIME_BASIS}={REGIME_BASIS_ELECTION} \
             {MARKER_POOL}=p1 {MARKER_SWAP_LINK}=s1 text"
        );
        assert_eq!(regime_from_notes(&notes).ok(), Some(Some(Regime::Neu)));
        assert_eq!(pool_id(&notes).ok().as_deref(), Some("p1"));
        assert_eq!(swap_link_id(&notes).ok(), Some(Some("s1".to_owned())));
        // The audit markers are written for Kassiber and never read by RP2's
        // rules: alone they set no regime, pool, or swap.
        let audit = format!(
            "{MARKER_REGIME_BASIS}={REGIME_BASIS_ELECTION} {MARKER_ALT_OUT}=5 {MARKER_ALT_IN}=4"
        );
        assert_eq!(regime_from_notes(&audit).ok(), Some(None));
        assert_eq!(pool_id(&audit).ok().as_deref(), Some(DEFAULT_POOL));
        assert!(!has_swap_link(&audit));
    }

    #[test]
    fn cutoff_is_march_first_2021_in_vienna() {
        let midnight = CivilDate {
            year: 2021,
            month: 3,
            day: 1,
        };
        // Europe/Vienna is UTC+1 (CET) in March before the switch.
        let cutoff = midnight.to_days() * 86_400_000_000 - 3_600_000_000;
        assert_eq!(manifest().neu_cutoff_us, cutoff);
    }

    #[test]
    fn categories_and_reasons_are_complete_and_distinct() {
        // Each match is exhaustive, so a new variant fails to compile here
        // until it is listed.
        fn category_position(category: AtCategory) -> usize {
            match category {
                AtCategory::IncomeGeneral => 0,
                AtCategory::IncomeCapitalYield => 1,
                AtCategory::NeuGain => 2,
                AtCategory::NeuLoss => 3,
                AtCategory::NeuSwap => 4,
                AtCategory::AltSpekulation => 5,
                AtCategory::AltTaxfree => 6,
            }
        }
        fn reason_listed(reason: QuarantineReason) -> bool {
            match reason {
                QuarantineReason::MissingSpotPrice
                | QuarantineReason::PricingReviewRequired
                | QuarantineReason::PrivacyHopUnresolved
                | QuarantineReason::ConflictingSpend
                | QuarantineReason::PendingOnchainConfirmation
                | QuarantineReason::DerivedTransferGroupBlocked
                | QuarantineReason::InsufficientLots
                | QuarantineReason::MissingCostBasis
                | QuarantineReason::BasisProvenanceIncomplete
                | QuarantineReason::UnclassifiedIncomeKind
                | QuarantineReason::NonSaleDisposalKind
                | QuarantineReason::AcquisitionValuationUnsupported
                | QuarantineReason::AtSwapBasisCarryUnresolved
                | QuarantineReason::BitcoinRailCarryBasisUnresolved => {
                    QuarantineReason::ALL.contains(&reason)
                }
            }
        }
        fn entry_type_listed(entry_type: EntryType) -> bool {
            match entry_type {
                EntryType::Acquisition
                | EntryType::Income
                | EntryType::Disposal
                | EntryType::Fee
                | EntryType::TransferIn
                | EntryType::TransferOut
                | EntryType::TransferFee => EntryType::ALL.contains(&entry_type),
            }
        }
        fn detail_kind_listed(kind: DetailKind) -> bool {
            match kind {
                DetailKind::PricingReview
                | DetailKind::PrivacyHop
                | DetailKind::Hold
                | DetailKind::PartnerLeg
                | DetailKind::Plain => DetailKind::ALL.contains(&kind),
            }
        }
        for (index, category) in AT_CATEGORIES.iter().enumerate() {
            assert_eq!(category_position(*category), index, "{}", category.as_str());
        }
        let reasons: BTreeSet<&str> = QuarantineReason::ALL.iter().map(|r| r.as_str()).collect();
        assert_eq!(reasons.len(), QuarantineReason::ALL.len());
        assert!(QuarantineReason::ALL.iter().all(|r| reason_listed(*r)));
        assert!(reasons.iter().all(|name| is_snake_case(name)));
        let entry_types: BTreeSet<&str> = EntryType::ALL.iter().map(|t| t.as_str()).collect();
        assert_eq!(entry_types.len(), EntryType::ALL.len());
        assert!(EntryType::ALL.iter().all(|t| entry_type_listed(*t)));
        assert!(entry_types.iter().all(|name| is_snake_case(name)));
        let kinds: BTreeSet<&str> = DetailKind::ALL.iter().map(|k| k.as_str()).collect();
        assert_eq!(kinds.len(), DetailKind::ALL.len());
        assert!(DetailKind::ALL.iter().all(|k| detail_kind_listed(*k)));
        assert!(kinds.iter().all(|name| is_snake_case(name)));
    }

    #[test]
    fn manifest_serializes_every_table() {
        let text = serde_json::to_string(&manifest()).expect("manifest serializes");
        // The kind table keeps its order, as Python's dict does.
        let kinds = concat!(
            r#""inbound_kind_types":{"buy":"BUY","airdrop":"AIRDROP","hardfork":"HARDFORK","#,
            r#""hard_fork":"HARDFORK","income":"INCOME","interest":"INTEREST","#,
            r#""lending_interest":"INTEREST","mining":"MINING","mining_reward":"MINING","#,
            r#""routing_income":"INCOME","staking":"STAKING","wages":"BUY"}"#,
        );
        assert!(text.contains(kinds), "{text}");
        let value: serde_json::Value = serde_json::from_str(&text).expect("manifest is JSON");
        let keys: Vec<&str> = value
            .as_object()
            .expect("manifest is an object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            [
                "at_categories",
                "at_long_term_days",
                "at_zero_cost_kinds",
                "default_inbound_type",
                "default_pool",
                "detail_kinds",
                "engine_version",
                "entry_types",
                "inbound_kind_types",
                "income_like_kind_tokens",
                "markers",
                "neu_cutoff_us",
                "non_sale_disposal_kind_tokens",
                "quarantine_reasons",
                "rail_quarantine_reason",
                "regime_basis_election",
                "swap_quarantine_reason",
            ]
        );
        assert_eq!(value["engine_version"], ENGINE_VERSION);
        assert_eq!(value["default_inbound_type"], "BUY");
        assert_eq!(value["default_pool"], "default");
        assert_eq!(value["neu_cutoff_us"], 1_614_553_200_000_000_i64);
        assert_eq!(value["at_long_term_days"], i64::MAX);
        assert_eq!(
            value["markers"],
            serde_json::json!({
                "regime": "at_regime",
                "regime_basis": "at_regime_basis",
                "pool": "at_pool",
                "swap_link": "at_swap_link",
                "alt_out": "at_alt_out",
                "alt_in": "at_alt_in",
            })
        );
        assert_eq!(value["regime_basis_election"], "wahlrecht");
        assert_eq!(
            value["at_categories"],
            serde_json::json!([
                "income_general",
                "income_capital_yield",
                "neu_gain",
                "neu_loss",
                "neu_swap",
                "alt_spekulation",
                "alt_taxfree",
            ])
        );
        assert_eq!(
            value["entry_types"],
            serde_json::json!([
                "acquisition",
                "income",
                "disposal",
                "fee",
                "transfer_in",
                "transfer_out",
                "transfer_fee",
            ])
        );
        assert_eq!(
            value["rail_quarantine_reason"],
            "bitcoin_rail_carry_basis_unresolved"
        );
        assert_eq!(
            value["swap_quarantine_reason"],
            "at_swap_basis_carry_unresolved"
        );
        assert_eq!(
            value["quarantine_reasons"].as_array().map(Vec::len),
            Some(14)
        );
        assert_eq!(
            value["detail_kinds"],
            serde_json::json!([
                "pricing_review",
                "privacy_hop",
                "hold",
                "partner_leg",
                "plain"
            ])
        );
    }
}
