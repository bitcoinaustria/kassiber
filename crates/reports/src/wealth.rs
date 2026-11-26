//! Stichtag wealth statement (`reports.wealth`, `ui.reports.wealth_preview`,
//! and PDF / XLSX / CSV exports).
//!
//! Computes exact per-wallet and per-asset holdings strictly before the next
//! local midnight after `as_of`, with optional Austrian Altbestand/Neubestand
//! classification and optional Stichtag valuation + cost-basis / lower-of-cost
//! accounting comparison.
//!
//! Unlike final tax-filing exports, a wealth statement never hard-blocks on
//! source freshness, stale journals, quarantines or custody gaps: it degrades
//! fail-closed into a clearly marked `draft` (with diagonal watermark in PDF)
//! and withholds any derived cost basis or Alt/Neu classification that cannot
//! be proven up to `as_of`.
use super::*;
use crate::artifact_csv::{File, Prepared};
use kassiber_pyjson::BigInt;
use std::collections::{BTreeMap, BTreeSet};

fn parse_bool_default(args: &Value, key: &str, default: bool) -> Result<bool> {
    match get(args, key) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Bool(v)) => Ok(*v),
        _ => Err(AppError::new(
            "validation",
            format!("{key} must be a boolean"),
        )),
    }
}

fn resolve_wallets(
    conn: &Connection,
    profile_id: &str,
    args: &Value,
) -> Result<Option<Vec<kassiber_wallets::Wallet>>> {
    let raw = match value(args, "wallets") {
        Value::Null => {
            if truthy(value(args, "wallet")) {
                Some(vec![value(args, "wallet").clone()])
            } else {
                None
            }
        }
        Value::List(items) => {
            if items.is_empty() {
                return Err(AppError::new(
                    "validation",
                    "Select at least one wallet for the wealth statement",
                ));
            }
            Some(items.clone())
        }
        _ => {
            return Err(AppError::new(
                "validation",
                "wallets must be an array of strings",
            ))
        }
    };
    let Some(refs) = raw else {
        return Ok(None);
    };
    let mut resolved = Vec::new();
    for reference in refs {
        let wallet = kassiber_wallets::resolve(conn, profile_id, &py_str(&reference))?;
        if !resolved
            .iter()
            .any(|existing: &kassiber_wallets::Wallet| existing.id == wallet.id)
        {
            resolved.push(wallet);
        }
    }
    Ok(Some(resolved))
}

fn decimal_exact_str(d: &Decimal) -> String {
    d.to_fixed_string().unwrap_or_else(|_| d.to_canonical())
}

fn msat_to_decimal(msat: i128) -> Result<Decimal> {
    Decimal::from_parts(msat < 0, msat.unsigned_abs().into(), -11).map_err(arithmetic)
}

/// A row's quantity fields. Every crypto asset shares the 10^-11 storage unit,
/// but only Bitcoin (BTC, Liquid BTC) quantities are BTC and msat: other
/// assets get their native quantity and unit with null BTC/msat fields, so no
/// consumer can add them to a Bitcoin total (docs/plan/24-crypto-assets.md).
fn quantity_fields(asset: &str, qty_msat: i128) -> Result<[(&'static str, Value); 4]> {
    let native = s(decimal_exact_str(&msat_to_decimal(qty_msat)?));
    let bitcoin = kassiber_store::assets::is_bitcoin(asset);
    Ok([
        ("quantity", native.clone()),
        (
            "quantityUnit",
            s(if bitcoin {
                "BTC".to_owned()
            } else {
                asset.trim().to_ascii_uppercase()
            }),
        ),
        (
            "quantityMsat",
            if bitcoin {
                Value::Int(BigInt::from(qty_msat))
            } else {
                Value::Null
            },
        ),
        ("quantityBtc", if bitcoin { native } else { Value::Null }),
    ])
}

/// `entries` with the quantity fields inserted after the leading `head` keys.
fn with_quantity(
    head: Vec<(&'static str, Value)>,
    asset: &str,
    qty_msat: i128,
    tail: Vec<(&'static str, Value)>,
) -> Result<Value> {
    let mut entries = head;
    entries.extend(quantity_fields(asset, qty_msat)?);
    entries.extend(tail);
    Ok(dict(entries))
}

fn opt_money(v: Option<&Decimal>) -> Result<Value> {
    v.map(|d| Ok(s(decimal_exact_str(d))))
        .transpose()
        .map(|x| x.unwrap_or(Value::Null))
}

fn add_opt(a: Option<Decimal>, b: Option<&Decimal>) -> Result<Option<Decimal>> {
    match (a, b) {
        (Some(x), Some(y)) => x.add(y, &DECIMAL).map(Some).map_err(arithmetic),
        _ => Ok(None),
    }
}

pub(crate) fn custody_class(kind: &str) -> &'static str {
    match kind.trim().to_ascii_lowercase().as_str() {
        "exchange" | "exchange_api" | "exchange_csv" | "broker" | "custodial" | "platform"
        | "kraken" | "coinbase" | "bitpanda" | "binance" | "bitstamp" | "bitvavo" | "bitfinex"
        | "bitcoinde" | "strike" | "river" | "bullbitcoin" | "relai" | "pocket"
        | "pocketbitcoin" | "21bitcoin" | "coinfinity" => "platform",
        "lightning" | "lnd" | "cln" | "coreln" | "nwc" | "phoenix" | "zeus" | "alby" | "breez" => {
            "self_custody_lightning"
        }
        "liquid" | "elements" => "self_custody_liquid",
        _ => "self_custody",
    }
}

/// Map every wallet label in `profile_id` to its coarse custody class
/// (`"platform"` or `"self_custody"`) for minimal-disclosure report redaction.
pub(crate) fn wallet_custody_map(
    conn: &Connection,
    profile_id: &str,
) -> Result<BTreeMap<String, &'static str>> {
    let wallet_rows = rows(
        conn,
        "SELECT label, kind FROM wallets WHERE profile_id = ?",
        [profile_id],
    )?;
    let mut map = BTreeMap::new();
    for w in &wallet_rows {
        let label = text(w, "label");
        if !label.is_empty() {
            let group = if custody_class(&text(w, "kind")) == "platform" {
                "platform"
            } else {
                "self_custody"
            };
            map.insert(label, group);
        }
    }
    Ok(map)
}

fn wealth_fmv(
    conn: &Connection,
    asset: &str,
    fiat: &str,
    cutoff: i64,
    fallback: &BTreeMap<String, (Decimal, i64)>,
) -> Result<(Option<Decimal>, Value)> {
    let pair = kassiber_rates::transaction_rate_pair(asset, fiat);
    let mut quotes: Vec<(Decimal, i64, &'static str, String)> = Vec::new();
    if let Some(pair) = &pair {
        let cached = sql::optional_rows(
            conn,
            "SELECT timestamp,rate,rate_exact,source FROM rates_cache WHERE pair=? \
             ORDER BY CASE WHEN source='manual' THEN 0 ELSE 1 END ASC,fetched_at DESC",
            [pair],
        )?;
        for row in &cached {
            let Some(at) = kassiber_rates::time::parse(&text(row, "timestamp")) else {
                continue;
            };
            if at >= cutoff {
                continue;
            }
            if let Some(rate) = exit_tax::exact(row, "rate")? {
                quotes.push((rate, at, "cache", text(row, "source")));
            }
        }
    }
    if let Some((rate, at)) = fallback.get(asset) {
        quotes.push((rate.clone(), *at, "transaction", "transaction".into()));
    }
    quotes.retain(|(rate, at, _, _)| {
        *rate > Decimal::zero() && 0 < cutoff - at && cutoff - at <= 86_400_000_000
    });
    // Manual cutoff quotes within 24h take priority over cached market data and
    // transaction rates; ties break by newest timestamp.
    quotes.sort_by_key(|(_, at, _, provider)| (provider != "manual", std::cmp::Reverse(*at)));
    let (found, as_of, source, provider) = match quotes.into_iter().next() {
        Some((rate, at, source, provider)) => (
            Some(rate),
            s(kassiber_rates::time::format(at).unwrap()),
            source,
            provider,
        ),
        None => (None, Value::Null, "missing", String::new()),
    };
    let row = dict([
        ("asset", s(asset)),
        (
            "rate",
            found
                .as_ref()
                .map(float)
                .transpose()?
                .unwrap_or(Value::Null),
        ),
        (
            "rateExact",
            found
                .as_ref()
                .map(|r| s(decimal_exact_str(r)))
                .unwrap_or(Value::Null),
        ),
        (
            "pair",
            s(pair.unwrap_or_else(|| format!("{asset}-{}", fiat.to_uppercase()))),
        ),
        ("asOf", as_of),
        ("source", s(source)),
        ("provider", s(provider)),
    ]);
    Ok((found, row))
}

/// Collect human-readable freshness & scan blockers without aborting export.
pub(crate) fn freshness_reasons(conn: &Connection, profile_id: &str) -> Result<Vec<String>> {
    let mut reasons = Vec::new();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM freshness_source_states WHERE profile_id=? AND blocking_reports=1",
            [profile_id],
            |r| r.get(0),
        )
        .map_err(sql_error)?;
    if count > 0 {
        reasons.push(format!(
            "{count} source refresh failure(s) are still unresolved."
        ));
    }
    let scans = crate::scan_readiness::silent_payment_blockers(conn, profile_id)?;
    if !scans.is_empty() {
        reasons.push(format!(
            "{} Silent Payments wallet scan(s) are incomplete.",
            scans.len()
        ));
    }
    Ok(reasons)
}

/// Collect draft-mode admission reasons without failing on stale journals,
/// freshness blockers, or custody issues. Hard boundaries (mixed networks,
/// unknown countries, non-Austrian E 1kv) still fail closed.
pub(crate) fn draft_admission_reasons(
    conn: &Connection,
    profile: &Value,
    austrian: bool,
) -> Result<Vec<String>> {
    let id = text(profile, "id");
    kassiber_setup::networks::require_accounting(conn, &id)?;
    let country = kassiber_pytext::strip(&text(profile, "tax_country")).to_lowercase();
    if !country::known(&country) {
        return Err(AppError::new(
            "internal_error",
            format!("Unsupported tax country '{}'", text(profile, "tax_country")),
        ));
    }
    if austrian {
        crate::austrian::validate(profile)?;
    }
    let mut reasons = freshness_reasons(conn, &id)?;
    let blockers = components::blockers(conn, profile)?;
    if !blockers.is_empty() {
        reasons.push(format!(
            "{} custody component(s) are incomplete or conflicting.",
            blockers.len()
        ));
    }
    let issues = quantity_issues(conn, &id)?;
    if !issues.is_empty() {
        reasons.push(format!(
            "{} unresolved custody quantity issue(s) block final cost basis.",
            issues.len()
        ));
    }
    let active: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transactions WHERE profile_id=? AND excluded=0",
            [&id],
            |r| r.get(0),
        )
        .map_err(sql_error)?;
    if !semantics_current(profile)
        || !truthy(value(profile, "last_processed_at"))
        || int(profile, "last_processed_tx_count") != active
        || int(profile, "journal_input_version") != int(profile, "last_processed_input_version")
    {
        reasons.push("Journals are stale and have not been reprocessed.".into());
    }
    Ok(reasons)
}

fn cutoff_day(
    conn: &Connection,
    raw: &Value,
    sources: &dyn kassiber_setup::Sources,
    zone: Option<&str>,
) -> Result<(String, i64)> {
    let raw = if truthy(raw) {
        py_str(raw)
    } else {
        sources.now(conn)?
    };
    let stripped = kassiber_pytext::strip(&raw);
    let hint =
        "Use a date (YYYY-MM-DD) or a date-time with an offset, e.g. 2025-01-01T12:00:00+01:00.";
    if stripped.chars().count() != 10
        && !stripped.to_uppercase().contains('T')
        && !stripped.contains(' ')
    {
        return Err(AppError::new("validation", "Invalid wealth as-of date").with_hint(hint));
    }
    let input = stripped.replace('Z', "+00:00");
    let (instant, has_offset) = kassiber_rates::time::parse_with_offset(&input)
        .ok_or_else(|| AppError::new("validation", "Invalid wealth as-of date").with_hint(hint))?;
    let civil_date = stripped.chars().count() == 10;
    if !civil_date && !has_offset {
        return Err(
            AppError::new("validation", "Wealth as-of time requires an offset").with_hint(hint),
        );
    }
    let zone = export_range::book_zone(zone, &[])?;
    let epoch = kassiber_rates::time::UNIX;
    let offset = zone
        .utc_offset((instant - epoch).div_euclid(1_000_000))
        .unwrap();
    let overflow = || AppError::new("internal_error", "date value out of range");
    let local = kassiber_rates::time::format(if civil_date {
        instant
    } else {
        instant + offset * 1_000_000
    })
    .ok_or_else(overflow)?;
    let day = local[..10].to_owned();
    let next = kassiber_rates::time::parse(&day).unwrap() + 86_400_000_000;
    kassiber_rates::time::format(next).ok_or_else(overflow)?;
    let offset = zone
        .local_offset((next - epoch).div_euclid(1_000_000))
        .unwrap();
    Ok((day, next - offset * 1_000_000))
}

pub fn report(
    conn: &Connection,
    workspace: &Value,
    profile: &Value,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Value> {
    let profile_id = text(profile, "id");
    // Hard guard: mixed Bitcoin networks can never be summed into one balance.
    kassiber_setup::networks::require_accounting(conn, &profile_id)?;
    let country = kassiber_pytext::strip(&text(profile, "tax_country")).to_lowercase();
    if !country::known(&country) {
        return Err(AppError::new(
            "internal_error",
            format!("Unsupported tax country '{}'", text(profile, "tax_country")),
        ));
    }
    let is_at = country::austrian(&country);
    let fiat = if truthy(value(profile, "fiat_currency")) {
        text(profile, "fiat_currency").to_uppercase()
    } else {
        "EUR".into()
    };
    let book_calendar = crate::tax_years::book_cut(conn, profile)?;
    let zone = book_calendar
        .as_ref()
        .map(|cal| cal.timezone)
        .or_else(|| country::zone(&country));
    let tz_label = zone.unwrap_or("UTC");
    let (as_of_day, cutoff) = cutoff_day(conn, value(args, "as_of"), sources, zone)?;

    let selected_wallets = resolve_wallets(conn, &profile_id, args)?;
    let wallet_filter: Option<BTreeSet<String>> = selected_wallets
        .as_ref()
        .map(|ws| ws.iter().map(|w| w.id.clone()).collect());
    let include_regime = parse_bool_default(args, "include_regime", is_at)?;
    let include_valuation = parse_bool_default(args, "include_valuation", true)?;
    let include_wallets = parse_bool_default(args, "include_wallets", true)?;
    let privacy = parse_bool_default(args, "privacy_mode", false)?;
    let explicit_draft = parse_bool_default(args, "draft", false)?;

    let pool_scope = {
        let raw = kassiber_pytext::lower(kassiber_pytext::strip(&text(
            profile,
            "cost_basis_pool_scope",
        )));
        if raw.is_empty() {
            "global".to_owned()
        } else {
            raw
        }
    };
    let per_wallet_basis = pool_scope == "wallet"
        || (pool_scope == "global_then_wallet" && as_of_day.as_str() >= "2025-01-01");

    // Inspect wallet catalog for metadata (label, kind, chain).
    let wallet_rows = rows(
        conn,
        "SELECT id,label,kind,config_json FROM wallets WHERE profile_id=? ORDER BY label ASC, id ASC",
        [&profile_id],
    )?;
    let mut wallet_meta: BTreeMap<String, (String, String, String)> = BTreeMap::new();
    for w in &wallet_rows {
        let chain = kassiber_pytext::loads(&text(w, "config_json"))
            .ok()
            .and_then(|cfg| {
                let raw = text(&cfg, "chain");
                if raw.is_empty() {
                    None
                } else {
                    Some(raw)
                }
            })
            .unwrap_or_default();
        wallet_meta.insert(text(w, "id"), (text(w, "label"), text(w, "kind"), chain));
    }

    // Check readiness conditions up to `cutoff`.
    let active_tx_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transactions WHERE profile_id=? AND excluded=0",
            [&profile_id],
            |r| r.get(0),
        )
        .map_err(sql_error)?;
    let journals_current = semantics_current(profile)
        && truthy(value(profile, "last_processed_at"))
        && int(profile, "last_processed_tx_count") == active_tx_count
        && int(profile, "journal_input_version") == int(profile, "last_processed_input_version");

    let mut draft_reasons = Vec::new();
    let mut draft_reason_codes: Vec<Value> = Vec::new();
    let freshness_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM freshness_source_states WHERE profile_id=? AND blocking_reports=1",
            [&profile_id],
            |r| r.get(0),
        )
        .map_err(sql_error)?;
    if freshness_count > 0 {
        draft_reasons.push(format!(
            "{freshness_count} source refresh failure(s) are still unresolved."
        ));
        draft_reason_codes.push(dict([
            ("code", s("freshness")),
            ("count", Value::Int(freshness_count.into())),
        ]));
    }
    let sp_scans = crate::scan_readiness::silent_payment_blockers(conn, &profile_id)?;
    if !sp_scans.is_empty() {
        draft_reasons.push(format!(
            "{} Silent Payments wallet scan(s) are incomplete.",
            sp_scans.len()
        ));
        draft_reason_codes.push(dict([
            ("code", s("silent_payments")),
            ("count", Value::Int(sp_scans.len().into())),
        ]));
    }
    let comp_blockers = components::blockers(conn, profile)?;
    if !comp_blockers.is_empty() {
        draft_reasons.push(format!(
            "{} custody component(s) are incomplete or conflicting.",
            comp_blockers.len()
        ));
        draft_reason_codes.push(dict([
            ("code", s("custody_components")),
            ("count", Value::Int(comp_blockers.len().into())),
        ]));
    }
    let all_qty_issues = quantity_issues(conn, &profile_id)?;
    let qty_issues_before_cutoff: Vec<Value> = all_qty_issues
        .into_iter()
        .filter(|issue| {
            let at = if truthy(value(issue, "blocks_from")) {
                text(issue, "blocks_from")
            } else {
                text(issue, "occurred_at")
            };
            if at.is_empty() {
                true
            } else {
                kassiber_rates::time::parse(&at).is_none_or(|ts| ts < cutoff)
            }
        })
        .collect();
    if !qty_issues_before_cutoff.is_empty() {
        draft_reasons.push(format!(
            "{} unresolved custody quantity issue(s) affect balances on or before {as_of_day}.",
            qty_issues_before_cutoff.len()
        ));
        draft_reason_codes.push(dict([
            ("code", s("quantity_issues")),
            ("count", Value::Int(qty_issues_before_cutoff.len().into())),
            ("as_of", s(as_of_day.as_str())),
        ]));
    }
    if !journals_current && active_tx_count > 0 {
        draft_reasons
            .push("Journals are not up to date with the latest transactions or edits.".into());
        draft_reason_codes.push(dict([("code", s("stale_journals"))]));
    }

    // Count quarantines dated strictly before `cutoff` (or undated).
    // On any book with a shared/pooled basis (`global` or `global_then_wallet`),
    // a quarantine in an unselected wallet still corrupts the shared pool, so
    // check book-wide quarantines independently of the display wallet filter.
    let quarantine_rows = rows(
        conn,
        "SELECT jq.transaction_id, jq.reason, t.occurred_at, t.wallet_id \
         FROM journal_quarantines jq \
         JOIN transactions t ON t.id = jq.transaction_id \
         WHERE jq.profile_id = ?",
        [&profile_id],
    )?;
    let before_cutoff = |row: &&Value| {
        let at = text(row, "occurred_at");
        at.is_empty() || kassiber_rates::time::parse(&at).is_none_or(|ts| ts < cutoff)
    };
    let book_quarantines_before_cutoff: usize =
        quarantine_rows.iter().filter(before_cutoff).count();
    let scoped_quarantines_before_cutoff: usize = quarantine_rows
        .iter()
        .filter(|row| {
            wallet_filter
                .as_ref()
                .is_none_or(|set| set.contains(&text(row, "wallet_id")))
                && before_cutoff(row)
        })
        .count();
    let quarantines_before_cutoff = if pool_scope == "wallet" {
        scoped_quarantines_before_cutoff
    } else {
        book_quarantines_before_cutoff
    };
    if quarantines_before_cutoff > 0 {
        draft_reasons.push(format!(
            "{quarantines_before_cutoff} quarantined transaction(s) fall on or before {as_of_day}."
        ));
        draft_reason_codes.push(dict([
            ("code", s("quarantines")),
            ("count", Value::Int(quarantines_before_cutoff.into())),
            ("as_of", s(as_of_day.as_str())),
        ]));
    }

    // Can we trust journal-derived basis & Alt/Neu classification?
    // Quarantined transactions are absent from `journal_entries`, so any
    // quarantine affecting the active pool on or before cutoff forces
    // observed-quantity fallback instead of dropping coins or guessing basis.
    let journals_usable = journals_current
        && comp_blockers.is_empty()
        && qty_issues_before_cutoff.is_empty()
        && quarantines_before_cutoff == 0;

    // Per-(wallet_id, asset) exact quantities (msat) and optional wallet-level basis.
    let mut wallet_qty: BTreeMap<(String, String), i128> = BTreeMap::new();
    let mut wallet_basis: BTreeMap<(String, String), Decimal> = BTreeMap::new();
    // Engine-persisted per-wallet holdings basis (transition- and transfer-aware
    // whenever no journal entries exist at or after `cutoff` for an asset).
    let mut stored_wallet_basis: BTreeMap<(String, String), Decimal> = BTreeMap::new();
    let mut stored_wallet_assets: BTreeSet<String> = BTreeSet::new();
    // Assets with entries at/after cutoff, pre-2025 entries, or self-transfers.
    let mut asset_has_post_cutoff: BTreeSet<String> = BTreeSet::new();
    let mut asset_has_pre_2025: BTreeSet<String> = BTreeSet::new();
    let mut asset_has_transfers: BTreeSet<String> = BTreeSet::new();
    // Global per-asset total quantity & basis across ALL wallets (needed for global pool avg cost).
    let mut global_asset_qty: BTreeMap<String, i128> = BTreeMap::new();
    let mut global_asset_basis: BTreeMap<String, Decimal> = BTreeMap::new();
    // Per-(asset, regime_idx [0=neu, 1=alt]) exact msat & cost basis (all wallets or filtered).
    let mut regime_buckets: BTreeMap<String, [(i128, Decimal); 2]> = BTreeMap::new();
    let mut regime_resolved = false;

    let data_source = if journals_usable {
        if per_wallet_basis {
            let wh_rows = rows(
                conn,
                "SELECT wallet_id, asset, cost_basis FROM journal_wallet_holdings WHERE profile_id = ?",
                [&profile_id],
            )?;
            for row in &wh_rows {
                let wid = text(row, "wallet_id");
                let asset = text(row, "asset");
                if wid.is_empty() || asset.is_empty() {
                    continue;
                }
                if let Some(cb) = exit_tax::exact(row, "cost_basis")? {
                    stored_wallet_assets.insert(asset.clone());
                    let entry = stored_wallet_basis
                        .entry((wid, asset))
                        .or_insert_with(Decimal::zero);
                    *entry = entry.add(&cb, &DECIMAL).map_err(arithmetic)?;
                }
            }
        }
        let entries = rows(
            conn,
            "SELECT e.*, w.label AS wallet_label FROM journal_entries e \
             LEFT JOIN wallets w ON w.id = e.wallet_id \
             WHERE e.profile_id = ? \
             ORDER BY e.occurred_at ASC, e.created_at ASC, e.id ASC",
            [&profile_id],
        )?;
        for row in &entries {
            let at = text(row, "occurred_at");
            let Some(instant) = kassiber_rates::time::parse(&at) else {
                continue;
            };
            let asset = text(row, "asset");
            if asset.is_empty() {
                continue;
            }
            if instant >= cutoff {
                asset_has_post_cutoff.insert(asset);
                continue;
            }
            if at.as_str() < "2025-01-01T00:00:00Z" {
                asset_has_pre_2025.insert(asset.clone());
            }
            let kind = text(row, "entry_type");
            let qty_msat = i128::from(int(row, "quantity"));
            let wallet_id = text(row, "wallet_id");
            let basis_opt = exit_tax::exact(row, "cost_basis")?;
            let fiat_opt = exit_tax::exact(row, "fiat_value")?;
            let basis_delta = if kind == "income" {
                Decimal::zero()
            } else if qty_msat >= 0 {
                basis_opt.or(fiat_opt).unwrap_or_else(Decimal::zero)
            } else {
                basis_opt
                    .unwrap_or_else(Decimal::zero)
                    .neg(&DECIMAL)
                    .map_err(arithmetic)?
            };

            if !wallet_id.is_empty() && !["income", "transfer_fee"].contains(&kind.as_str()) {
                *wallet_qty
                    .entry((wallet_id.clone(), asset.clone()))
                    .or_insert(0) += qty_msat;
            }
            if !wallet_id.is_empty() && kind != "income" {
                let entry = wallet_basis
                    .entry((wallet_id.clone(), asset.clone()))
                    .or_insert_with(Decimal::zero);
                *entry = entry.add(&basis_delta, &DECIMAL).map_err(arithmetic)?;
            }

            if ["transfer_in", "transfer_out"].contains(&kind.as_str()) {
                asset_has_transfers.insert(asset);
                continue;
            }
            if kind == "income" {
                continue;
            }
            *global_asset_qty.entry(asset.clone()).or_insert(0) += qty_msat;
            let gb = global_asset_basis
                .entry(asset.clone())
                .or_insert_with(Decimal::zero);
            *gb = gb.add(&basis_delta, &DECIMAL).map_err(arithmetic)?;

            let regime_idx = usize::from(is_at && exit_tax::alt(row)?);
            let bucket = &mut regime_buckets
                .entry(asset)
                .or_insert_with(|| [(0, Decimal::zero()), (0, Decimal::zero())])[regime_idx];
            bucket.0 += qty_msat;
            bucket.1 = bucket.1.add(&basis_delta, &DECIMAL).map_err(arithmetic)?;
        }
        // `regime_buckets` aggregates across the whole book; only mark Alt/Neu
        // resolved when no wallet subset filter is active.
        regime_resolved = is_at && include_regime && wallet_filter.is_none();
        "journals"
    } else {
        // Fallback: reconstruct exact observed wallet quantities strictly before cutoff.
        let tx_rows = rows(
            conn,
            "SELECT wallet_id, asset, direction, amount, fee, amount_includes_fee, occurred_at \
             FROM transactions WHERE profile_id = ? AND excluded = 0 \
             ORDER BY occurred_at ASC, created_at ASC, id ASC",
            [&profile_id],
        )?;
        for row in &tx_rows {
            let at = text(row, "occurred_at");
            let Some(instant) = kassiber_rates::time::parse(&at) else {
                continue;
            };
            if instant >= cutoff {
                continue;
            }
            let asset = text(row, "asset");
            let wallet_id = text(row, "wallet_id");
            if asset.is_empty() || wallet_id.is_empty() {
                continue;
            }
            let amount = i128::from(int(row, "amount"));
            let fee = i128::from(int(row, "fee"));
            let includes_fee = int(row, "amount_includes_fee") != 0;
            let direction = text(row, "direction").trim().to_ascii_lowercase();
            let delta = match direction.as_str() {
                "inbound" => amount,
                "outbound" => {
                    if includes_fee {
                        -amount
                    } else {
                        -amount - fee
                    }
                }
                _ => 0,
            };
            *wallet_qty.entry((wallet_id, asset.clone())).or_insert(0) += delta;
            *global_asset_qty.entry(asset).or_insert(0) += delta;
        }
        "observed"
    };

    // Apply optional wallet filter to wallet holdings.
    let filtered_wallet_qty: BTreeMap<(String, String), i128> = wallet_qty
        .iter()
        .filter(|((wid, _), qty)| {
            **qty > 0 && wallet_filter.as_ref().is_none_or(|set| set.contains(wid))
        })
        .map(|(k, v)| (k.clone(), *v))
        .collect();

    // Determine held assets in scope.
    let mut held_assets: BTreeSet<String> = filtered_wallet_qty
        .keys()
        .map(|(_, asset)| asset.clone())
        .collect();
    if wallet_filter.is_none() {
        for (asset, qty) in &global_asset_qty {
            if *qty > 0 {
                held_assets.insert(asset.clone());
            }
        }
    }

    // Look up local as-of FMV rates when valuation is enabled.
    let fallback = if include_valuation {
        exit_tax::fallback_rates(conn, profile, cutoff)?
    } else {
        BTreeMap::new()
    };
    let mut rate_map: BTreeMap<String, Option<Decimal>> = BTreeMap::new();
    let mut valuation_sources = Vec::new();
    let mut missing_fmv_assets = Vec::new();
    if include_valuation {
        for asset in &held_assets {
            let (rate, source_row) = wealth_fmv(conn, asset, &fiat, cutoff, &fallback)?;
            if rate.is_none() {
                missing_fmv_assets.push(s(asset.as_str()));
            }
            rate_map.insert(asset.clone(), rate);
            valuation_sources.push(source_row);
        }
    }

    // Build asset summary rows (with Alt/Neu split when requested for Austrian books).
    let mut asset_rows = Vec::new();
    let mut total_qty_msat: i128 = 0;
    let mut alt_qty_msat: i128 = 0;
    let mut neu_qty_msat: i128 = 0;
    let mut non_bitcoin_assets: BTreeSet<String> = BTreeSet::new();
    let mut total_basis: Option<Decimal> = if journals_usable && include_valuation {
        Some(Decimal::zero())
    } else {
        None
    };
    let mut total_market: Option<Decimal> = if include_valuation && missing_fmv_assets.is_empty() {
        Some(Decimal::zero())
    } else {
        None
    };
    let mut total_lower: Option<Decimal> =
        if journals_usable && include_valuation && missing_fmv_assets.is_empty() {
            Some(Decimal::zero())
        } else {
            None
        };
    let mut total_unrealized: Option<Decimal> =
        if journals_usable && include_valuation && missing_fmv_assets.is_empty() {
            Some(Decimal::zero())
        } else {
            None
        };
    let mut total_impairment: Option<Decimal> =
        if journals_usable && include_valuation && missing_fmv_assets.is_empty() {
            Some(Decimal::zero())
        } else {
            None
        };

    // Helper: sum filtered wallet quantity per asset.
    let mut scope_asset_qty: BTreeMap<String, i128> = BTreeMap::new();
    for ((_, asset), qty) in &filtered_wallet_qty {
        *scope_asset_qty.entry(asset.clone()).or_insert(0) += *qty;
    }

    for asset in &held_assets {
        let rate = rate_map.get(asset).and_then(Option::as_ref);
        // Quantity totals are Bitcoin totals: other assets' quantities are
        // in their own units and never add to them (fiat totals still do).
        let bitcoin = kassiber_store::assets::is_bitcoin(asset);
        if !bitcoin {
            non_bitcoin_assets.insert(asset.trim().to_ascii_uppercase());
        }
        type AssetValuation = (
            Value,
            Option<Decimal>,
            Option<Decimal>,
            Option<Decimal>,
            Option<Decimal>,
            Option<Decimal>,
        );
        let emit_row = |regime_code: &str,
                        qty_msat: i128,
                        basis_opt: Option<Decimal>|
         -> Result<AssetValuation> {
            let qty_dec = msat_to_decimal(qty_msat)?;
            let market = if include_valuation {
                rate.map(|r| qty_dec.mul(r, &DECIMAL).map_err(arithmetic))
                    .transpose()?
            } else {
                None
            };
            let basis = if include_valuation {
                basis_opt.map(|b| b.max(Decimal::zero()))
            } else {
                None
            };
            let avg_cost = match &basis {
                Some(b) if qty_dec > Decimal::zero() => {
                    Some(b.div(&qty_dec, &DECIMAL).map_err(arithmetic)?)
                }
                _ => None,
            };
            let (lower, unrealized, impairment) = match (&basis, &market) {
                (Some(b), Some(m)) => {
                    let lower = if b <= m { b.clone() } else { m.clone() };
                    let diff = m.sub(b, &DECIMAL).map_err(arithmetic)?;
                    let imp = if b > m {
                        b.sub(m, &DECIMAL).map_err(arithmetic)?
                    } else {
                        Decimal::zero()
                    };
                    (Some(lower), Some(diff), Some(imp))
                }
                _ => (None, None, None),
            };
            let row = with_quantity(
                vec![("asset", s(asset.as_str())), ("regime", s(regime_code))],
                asset,
                qty_msat,
                vec![
                    ("unitRate", opt_money(rate)?),
                    ("avgCost", opt_money(avg_cost.as_ref())?),
                    ("costBasis", opt_money(basis.as_ref())?),
                    ("marketValue", opt_money(market.as_ref())?),
                    ("lowerOfCostOrMarket", opt_money(lower.as_ref())?),
                    ("unrealizedPnl", opt_money(unrealized.as_ref())?),
                    ("impairmentIndicator", opt_money(impairment.as_ref())?),
                ],
            )?;
            Ok((row, basis, market, lower, unrealized, impairment))
        };

        if regime_resolved {
            // Emit Altbestand first, then Neubestand (only positive buckets).
            let buckets = regime_buckets
                .get(asset)
                .cloned()
                .unwrap_or([(0, Decimal::zero()), (0, Decimal::zero())]);
            for (idx, code) in [(1usize, "alt"), (0usize, "neu")] {
                let (qty_msat, ref basis) = buckets[idx];
                if qty_msat <= 0 {
                    continue;
                }
                if bitcoin {
                    if idx == 1 {
                        alt_qty_msat += qty_msat;
                    } else {
                        neu_qty_msat += qty_msat;
                    }
                    total_qty_msat += qty_msat;
                }
                let (row, b, m, l, u, imp) = emit_row(code, qty_msat, Some(basis.clone()))?;
                total_basis = add_opt(total_basis, b.as_ref())?;
                total_market = add_opt(total_market, m.as_ref())?;
                total_lower = add_opt(total_lower, l.as_ref())?;
                total_unrealized = add_opt(total_unrealized, u.as_ref())?;
                total_impairment = add_opt(total_impairment, imp.as_ref())?;
                asset_rows.push(row);
            }
        } else {
            let qty_msat = if wallet_filter.is_some() {
                *scope_asset_qty.get(asset).unwrap_or(&0)
            } else {
                *global_asset_qty
                    .get(asset)
                    .or_else(|| scope_asset_qty.get(asset))
                    .unwrap_or(&0)
            };
            if qty_msat <= 0 {
                continue;
            }
            if bitcoin {
                total_qty_msat += qty_msat;
            }
            let exact_wallet_basis = |wid: &str, asset: &str| -> Option<Decimal> {
                if !per_wallet_basis {
                    return None;
                }
                if !asset_has_post_cutoff.contains(asset) && stored_wallet_assets.contains(asset) {
                    return Some(
                        stored_wallet_basis
                            .get(&(wid.to_owned(), asset.to_owned()))
                            .cloned()
                            .unwrap_or_else(Decimal::zero)
                            .max(Decimal::zero()),
                    );
                }
                let entries_exact = (pool_scope == "wallet"
                    && !asset_has_transfers.contains(asset))
                    || (pool_scope == "global_then_wallet"
                        && as_of_day.as_str() >= "2025-01-01"
                        && !asset_has_pre_2025.contains(asset)
                        && !asset_has_transfers.contains(asset));
                if entries_exact {
                    Some(
                        wallet_basis
                            .get(&(wid.to_owned(), asset.to_owned()))
                            .cloned()
                            .unwrap_or_else(Decimal::zero)
                            .max(Decimal::zero()),
                    )
                } else {
                    None
                }
            };
            let basis_opt = if !journals_usable {
                None
            } else if wallet_filter.is_none() {
                Some(
                    global_asset_basis
                        .get(asset)
                        .cloned()
                        .unwrap_or_else(Decimal::zero),
                )
            } else if let Some(exact_sum) = filtered_wallet_qty
                .keys()
                .filter(|(_, a)| a == asset)
                .map(|(wid, a)| exact_wallet_basis(wid, a))
                .collect::<Option<Vec<Decimal>>>()
            {
                let mut sum = Decimal::zero();
                for wb in exact_sum {
                    sum = sum.add(&wb, &DECIMAL).map_err(arithmetic)?;
                }
                Some(sum)
            } else {
                // Global pool (or historical mid-history cutoff with transfers / pre-2025
                // transition carryover): allocate proportionally from total asset basis.
                let gq = *global_asset_qty.get(asset).unwrap_or(&0);
                let gb = global_asset_basis
                    .get(asset)
                    .cloned()
                    .unwrap_or_else(Decimal::zero);
                if gq > 0 {
                    let ratio = Decimal::from(qty_msat)
                        .div(&Decimal::from(gq), &DECIMAL)
                        .map_err(arithmetic)?;
                    Some(gb.mul(&ratio, &DECIMAL).map_err(arithmetic)?)
                } else {
                    Some(Decimal::zero())
                }
            };
            let regime_code = if is_at && include_regime {
                "unresolved"
            } else {
                "all"
            };
            let (row, b, m, l, u, imp) = emit_row(regime_code, qty_msat, basis_opt)?;
            total_basis = add_opt(total_basis, b.as_ref())?;
            total_market = add_opt(total_market, m.as_ref())?;
            total_lower = add_opt(total_lower, l.as_ref())?;
            total_unrealized = add_opt(total_unrealized, u.as_ref())?;
            total_impairment = add_opt(total_impairment, imp.as_ref())?;
            asset_rows.push(row);
        }
    }

    // Build per-wallet (or grouped custody-class in privacy mode) holdings rows.
    let exact_wallet_basis = |wid: &str, asset: &str| -> Option<Decimal> {
        if !per_wallet_basis {
            return None;
        }
        if !asset_has_post_cutoff.contains(asset) && stored_wallet_assets.contains(asset) {
            return Some(
                stored_wallet_basis
                    .get(&(wid.to_owned(), asset.to_owned()))
                    .cloned()
                    .unwrap_or_else(Decimal::zero)
                    .max(Decimal::zero()),
            );
        }
        let entries_exact = (pool_scope == "wallet" && !asset_has_transfers.contains(asset))
            || (pool_scope == "global_then_wallet"
                && as_of_day.as_str() >= "2025-01-01"
                && !asset_has_pre_2025.contains(asset)
                && !asset_has_transfers.contains(asset));
        if entries_exact {
            Some(
                wallet_basis
                    .get(&(wid.to_owned(), asset.to_owned()))
                    .cloned()
                    .unwrap_or_else(Decimal::zero)
                    .max(Decimal::zero()),
            )
        } else {
            None
        }
    };
    let mut any_wallet_basis_allocated = false;
    let mut wallet_holdings = Vec::new();
    if privacy {
        // Merge self-custody wallets together and platform wallets together so
        // individual hardware/software wallet names and counts stay private.
        type CustodyBucket = (i128, Option<Decimal>, Option<Decimal>, bool);
        let mut grouped: BTreeMap<(&'static str, String), CustodyBucket> = BTreeMap::new();
        for ((wid, asset), qty_msat) in &filtered_wallet_qty {
            let (_, kind, _) = wallet_meta
                .get(wid)
                .cloned()
                .unwrap_or_else(|| (wid.clone(), String::new(), String::new()));
            let group = if custody_class(&kind) == "platform" {
                "platform"
            } else {
                "self_custody"
            };
            let qty_dec = msat_to_decimal(*qty_msat)?;
            let rate = rate_map.get(asset).and_then(Option::as_ref);
            let market = if include_valuation {
                rate.map(|r| qty_dec.mul(r, &DECIMAL).map_err(arithmetic))
                    .transpose()?
            } else {
                None
            };
            let (basis_opt, allocated) = if !include_valuation || !journals_usable {
                (None, false)
            } else if let Some(b) = exact_wallet_basis(wid, asset) {
                (Some(b), false)
            } else {
                any_wallet_basis_allocated = true;
                let gq = *global_asset_qty.get(asset).unwrap_or(&0);
                let gb = global_asset_basis
                    .get(asset)
                    .cloned()
                    .unwrap_or_else(Decimal::zero)
                    .max(Decimal::zero());
                if gq > 0 {
                    let ratio = Decimal::from(*qty_msat)
                        .div(&Decimal::from(gq), &DECIMAL)
                        .map_err(arithmetic)?;
                    (Some(gb.mul(&ratio, &DECIMAL).map_err(arithmetic)?), true)
                } else {
                    (Some(Decimal::zero()), true)
                }
            };
            let entry = grouped.entry((group, asset.clone())).or_insert_with(|| {
                (
                    0,
                    basis_opt.as_ref().map(|_| Decimal::zero()),
                    market.as_ref().map(|_| Decimal::zero()),
                    allocated,
                )
            });
            entry.0 += *qty_msat;
            entry.1 = add_opt(entry.1.clone(), basis_opt.as_ref())?;
            entry.2 = add_opt(entry.2.clone(), market.as_ref())?;
            entry.3 = entry.3 || allocated;
        }
        for ((group, asset), (qty_msat, basis_opt, market, allocated)) in grouped {
            let asset_total = *scope_asset_qty.get(&asset).unwrap_or(&0);
            let share_pct = if asset_total > 0 {
                let pct = Decimal::from(qty_msat)
                    .mul(&Decimal::from(100), &DECIMAL)
                    .map_err(arithmetic)?
                    .div(&Decimal::from(asset_total), &DECIMAL)
                    .map_err(arithmetic)?;
                s(decimal_exact_str(&pct))
            } else {
                Value::Null
            };
            wallet_holdings.push(with_quantity(
                vec![
                    ("walletId", s("")),
                    ("wallet", s("")),
                    ("walletKind", s("")),
                    ("custodyClass", s(group)),
                    ("chain", s("")),
                    ("asset", s(asset.as_str())),
                ],
                &asset,
                qty_msat,
                vec![
                    ("sharePercent", share_pct),
                    ("costBasis", opt_money(basis_opt.as_ref())?),
                    ("costBasisAllocated", Value::Bool(allocated)),
                    ("marketValue", opt_money(market.as_ref())?),
                ],
            )?);
        }
    } else {
        for ((wid, asset), qty_msat) in &filtered_wallet_qty {
            let (label, kind, chain) = wallet_meta
                .get(wid)
                .cloned()
                .unwrap_or_else(|| (wid.clone(), String::new(), String::new()));
            let c_class = custody_class(&kind);
            let qty_dec = msat_to_decimal(*qty_msat)?;
            let asset_total = *scope_asset_qty.get(asset).unwrap_or(&0);
            let share_pct = if asset_total > 0 {
                let pct = Decimal::from(*qty_msat)
                    .mul(&Decimal::from(100), &DECIMAL)
                    .map_err(arithmetic)?
                    .div(&Decimal::from(asset_total), &DECIMAL)
                    .map_err(arithmetic)?;
                s(decimal_exact_str(&pct))
            } else {
                Value::Null
            };
            let rate = rate_map.get(asset).and_then(Option::as_ref);
            let market = if include_valuation {
                rate.map(|r| qty_dec.mul(r, &DECIMAL).map_err(arithmetic))
                    .transpose()?
            } else {
                None
            };
            let (basis_opt, allocated) = if !include_valuation || !journals_usable {
                (None, false)
            } else if let Some(b) = exact_wallet_basis(wid, asset) {
                (Some(b), false)
            } else {
                any_wallet_basis_allocated = true;
                let gq = *global_asset_qty.get(asset).unwrap_or(&0);
                let gb = global_asset_basis
                    .get(asset)
                    .cloned()
                    .unwrap_or_else(Decimal::zero)
                    .max(Decimal::zero());
                if gq > 0 {
                    let ratio = Decimal::from(*qty_msat)
                        .div(&Decimal::from(gq), &DECIMAL)
                        .map_err(arithmetic)?;
                    (Some(gb.mul(&ratio, &DECIMAL).map_err(arithmetic)?), true)
                } else {
                    (Some(Decimal::zero()), true)
                }
            };
            wallet_holdings.push(with_quantity(
                vec![
                    ("walletId", s(wid.as_str())),
                    ("wallet", s(label)),
                    ("walletKind", s(kind)),
                    ("custodyClass", s(c_class)),
                    ("chain", s(chain)),
                    ("asset", s(asset.as_str())),
                ],
                asset,
                *qty_msat,
                vec![
                    ("sharePercent", share_pct),
                    ("costBasis", opt_money(basis_opt.as_ref())?),
                    ("costBasisAllocated", Value::Bool(allocated)),
                    ("marketValue", opt_money(market.as_ref())?),
                ],
            )?);
        }
        wallet_holdings.sort_by_key(|r| (text(r, "asset"), text(r, "wallet")));
    }

    let is_draft = explicit_draft
        || !draft_reasons.is_empty()
        || (include_valuation && !missing_fmv_assets.is_empty());

    let wallet_scope_labels: Vec<Value> = if privacy {
        Vec::new()
    } else {
        match &selected_wallets {
            Some(ws) => ws.iter().map(|w| s(w.label.as_str())).collect(),
            None => Vec::new(),
        }
    };

    let method = kassiber_pytext::strip(&if truthy(value(profile, "gains_algorithm")) {
        text(profile, "gains_algorithm")
    } else if is_at {
        "moving_average_at".into()
    } else {
        "fifo".into()
    })
    .to_lowercase();

    let total_qty_dec = msat_to_decimal(total_qty_msat)?;
    let alt_qty_dec = msat_to_decimal(alt_qty_msat)?;
    let neu_qty_dec = msat_to_decimal(neu_qty_msat)?;

    Ok(dict([
        (
            "workspace",
            if truthy(value(workspace, "label")) {
                value(workspace, "label").clone()
            } else {
                value(profile, "workspace_id").clone()
            },
        ),
        (
            "profile",
            if truthy(value(profile, "label")) {
                value(profile, "label").clone()
            } else {
                value(profile, "id").clone()
            },
        ),
        ("jurisdictionCode", s(country.to_uppercase())),
        ("taxCountry", s(country)),
        ("fiatCurrency", s(fiat)),
        ("asOfDate", s(as_of_day)),
        ("asOfTimezone", s(tz_label)),
        ("method", s(method)),
        ("costBasisPoolScope", s(pool_scope)),
        (
            "perWalletBasis",
            Value::Bool(per_wallet_basis && !any_wallet_basis_allocated),
        ),
        ("includeRegime", Value::Bool(include_regime && is_at)),
        ("regimeResolved", Value::Bool(regime_resolved)),
        ("includeValuation", Value::Bool(include_valuation)),
        ("includeWallets", Value::Bool(include_wallets)),
        ("privacyMode", Value::Bool(privacy)),
        ("walletScope", Value::List(wallet_scope_labels)),
        (
            "journalInputVersion",
            if journals_usable {
                value(profile, "journal_input_version").clone()
            } else {
                Value::Null
            },
        ),
        ("dataSource", s(data_source)),
        ("status", s(if is_draft { "draft" } else { "final" })),
        ("isDraft", Value::Bool(is_draft)),
        (
            "draftReasons",
            Value::List(draft_reasons.into_iter().map(s).collect()),
        ),
        ("draftReasonCodes", Value::List(draft_reason_codes)),
        (
            "quarantinesBeforeAsOf",
            Value::Int(quarantines_before_cutoff.into()),
        ),
        ("missingFmvAssets", Value::List(missing_fmv_assets)),
        ("valuationSources", Value::List(valuation_sources)),
        ("assets", Value::List(asset_rows)),
        (
            "walletHoldings",
            Value::List(if include_wallets {
                wallet_holdings
            } else {
                Vec::new()
            }),
        ),
        (
            "totals",
            dict([
                ("quantityMsat", Value::Int(BigInt::from(total_qty_msat))),
                ("quantityBtc", s(decimal_exact_str(&total_qty_dec))),
                ("altQuantityMsat", Value::Int(BigInt::from(alt_qty_msat))),
                ("altQuantityBtc", s(decimal_exact_str(&alt_qty_dec))),
                ("neuQuantityMsat", Value::Int(BigInt::from(neu_qty_msat))),
                ("neuQuantityBtc", s(decimal_exact_str(&neu_qty_dec))),
                (
                    "excludedQuantityAssets",
                    Value::List(non_bitcoin_assets.into_iter().map(s).collect()),
                ),
                ("costBasis", opt_money(total_basis.as_ref())?),
                ("marketValue", opt_money(total_market.as_ref())?),
                ("lowerOfCostOrMarket", opt_money(total_lower.as_ref())?),
                ("unrealizedPnl", opt_money(total_unrealized.as_ref())?),
                ("impairmentIndicator", opt_money(total_impairment.as_ref())?),
            ]),
        ),
    ]))
}

pub fn model(
    conn: &Connection,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Value> {
    let (workspace, profile) = kassiber_setup::references::scope(conn, args)?;
    let report = report(conn, &workspace, &profile, args, sources)?;
    Ok(dict([
        ("report", report),
        ("generated_at", s(sources.now(conn)?)),
    ]))
}

pub(super) fn prepare_pdf(
    conn: &Connection,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Prepared> {
    let model = model(conn, args, sources)?;
    let mut doc = report_design::wealth(&model);
    if privacy_mode(args) {
        let (_, profile) = kassiber_setup::references::scope(conn, args)?;
        doc.apply_privacy_mode_for(&wallet_custody_map(conn, &text(&profile, "id"))?);
    }
    let (bytes, pages) = report_design::pdf(&doc)?;
    let report = value(&model, "report");
    let title = format!(
        "Wealth statement — {} ({})",
        text(report, "profile"),
        text(report, "asOfDate")
    );
    Ok(Prepared {
        files: vec![File {
            name: "wealth-statement.pdf".into(),
            sheet: String::new(),
            bytes,
            rows: 0,
        }],
        metadata: dict([
            ("pages", Value::Int(pages.into())),
            ("title", s(title)),
            ("scope", s("wealth")),
            ("format", s("pdf")),
            ("as_of", value(report, "asOfDate").clone()),
            ("status", value(report, "status").clone()),
            ("is_draft", value(report, "isDraft").clone()),
            ("draft_reasons", value(report, "draftReasons").clone()),
        ]),
        completion: None,
    })
}

pub(super) fn prepare_xlsx(
    conn: &Connection,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Prepared> {
    let model = model(conn, args, sources)?;
    let mut doc = report_design::wealth(&model);
    if privacy_mode(args) {
        let (_, profile) = kassiber_setup::references::scope(conn, args)?;
        doc.apply_privacy_mode_for(&wallet_custody_map(conn, &text(&profile, "id"))?);
    }
    let workbook = report_design::xlsx_document(&doc);
    let bytes = kassiber_xlsx::render(&workbook)
        .map_err(|e| AppError::new("internal_error", e.to_string()))?;
    let report = value(&model, "report");
    let row_count = artifact_csv::list(value(report, "assets")).len()
        + artifact_csv::list(value(report, "walletHoldings")).len();
    Ok(Prepared {
        files: vec![File {
            name: "wealth-statement.xlsx".into(),
            sheet: String::new(),
            bytes,
            rows: row_count,
        }],
        metadata: dict([
            ("scope", s("wealth")),
            ("format", s("xlsx")),
            ("as_of", value(report, "asOfDate").clone()),
            ("status", value(report, "status").clone()),
            ("is_draft", value(report, "isDraft").clone()),
            ("draft_reasons", value(report, "draftReasons").clone()),
            (
                "sheets",
                Value::List(
                    workbook
                        .sheets
                        .iter()
                        .map(|sheet| s(sheet.name.as_str()))
                        .collect(),
                ),
            ),
            ("rows", Value::Int(row_count.into())),
        ]),
        completion: None,
    })
}

pub(super) fn prepare_csv(
    conn: &Connection,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Prepared> {
    let model = model(conn, args, sources)?;
    let mut doc = report_design::wealth(&model);
    if privacy_mode(args) {
        let (_, profile) = kassiber_setup::references::scope(conn, args)?;
        doc.apply_privacy_mode_for(&wallet_custody_map(conn, &text(&profile, "id"))?);
    }
    let bytes = report_design::csv_zip(&doc)?;
    let report = value(&model, "report");
    let row_count = artifact_csv::list(value(report, "assets")).len()
        + artifact_csv::list(value(report, "walletHoldings")).len();
    let sections: Vec<Value> = doc
        .sections
        .iter()
        .flat_map(|s| &s.blocks)
        .filter_map(|b| match b {
            report_design::Block::Table(t) => Some(s(t.id)),
            _ => None,
        })
        .collect();
    Ok(Prepared {
        files: vec![File {
            name: "wealth-statement.zip".into(),
            sheet: String::new(),
            bytes,
            rows: row_count,
        }],
        metadata: dict([
            ("scope", s("wealth")),
            ("format", s("csv")),
            ("as_of", value(report, "asOfDate").clone()),
            ("status", value(report, "status").clone()),
            ("is_draft", value(report, "isDraft").clone()),
            ("draft_reasons", value(report, "draftReasons").clone()),
            ("sections", Value::List(sections)),
            ("rows", Value::Int(row_count.into())),
        ]),
        completion: None,
    })
}
