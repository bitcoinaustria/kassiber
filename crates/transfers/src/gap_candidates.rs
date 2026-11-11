//! Current gap candidates are always derived from the book's raw worklist.
use super::component_rows::s;
use super::*;
use kassiber_custody::{codec, gaps};
use kassiber_envelope::truthy;
use kassiber_store::rusqlite::{params, params_from_iter};
use std::collections::BTreeSet;

const SELECT: &str = "SELECT t.id,t.profile_id,t.wallet_id,w.label AS wallet_label,w.kind AS wallet_kind,t.occurred_at,t.direction,t.asset,t.amount,t.fee,t.amount_includes_fee,t.excluded,t.kind,t.kind_override,t.privacy_boundary,t.external_id,t.raw_json,w.config_json AS wallet_config_json FROM transactions t JOIN wallets w ON w.id=t.wallet_id";
const STRUCTURED: &str = "LOWER(COALESCE(t.privacy_boundary,'')) IN ('coinjoin','payjoin','whirlpool') OR LOWER(COALESCE(w.kind,'')) IN ('samourai','samourai-whirlpool','whirlpool') OR LOWER(COALESCE(t.kind,'')) IN ('badbank','coinjoin','postmix','premix','samourai_badbank','samourai_deposit','samourai_postmix','samourai_premix','samourai_tx0','whirlpool_mix','whirlpool_tx0') OR LOWER(COALESCE(w.config_json,'')) LIKE '%\"samourai\"%'";

fn internal(message: impl Into<String>) -> AppError {
    AppError::new("internal_error", message)
}

fn search_error(worklist: &[serde_json::Value], message: String) -> AppError {
    if let Some((row, code)) = gaps::normalization_failure(worklist) {
        if code == "OverflowError" {
            return internal("date value out of range");
        }
        if code == "ValueError" {
            for key in ["amount", "fee"] {
                if let Ok(value) = codec::from_json(&row[key]) {
                    if let Err(error) = integer(&value) {
                        return error;
                    }
                }
            }
        }
    }
    internal(message)
}

fn worklist(c: &Connection, profile: &str, count: i64) -> Result<(Vec<serde_json::Value>, bool)> {
    let large = count > 50_000;
    let selected = if !large {
        rows(c, &format!("{SELECT} WHERE t.profile_id=? AND t.excluded=0 ORDER BY t.occurred_at,t.created_at,t.id"), [profile])?
    } else {
        let mut sources = rows(c, &format!("{SELECT} WHERE t.profile_id=? AND t.excluded=0 AND t.direction='outbound' AND ({STRUCTURED}) ORDER BY t.occurred_at,t.created_at,t.id LIMIT 256"), [profile])?;
        let budget = 64.min(256 - sources.len());
        if budget > 0 {
            let mut ordinary = Vec::new();
            for asset in rows(c, "SELECT DISTINCT asset FROM transactions WHERE profile_id=? AND excluded=0 AND direction='outbound' ORDER BY asset", [profile])? {
                ordinary.extend(rows(c, &format!("{SELECT} WHERE t.profile_id=? AND t.excluded=0 AND t.direction='outbound' AND t.asset=? AND NOT ({STRUCTURED}) ORDER BY t.amount DESC,t.occurred_at DESC,t.created_at DESC,t.id DESC LIMIT ?"), params![profile,sql::bind(&value(&asset,"asset"),2)?,budget as i64])?);
            }
            let mut ordinary = ordinary
                .into_iter()
                .map(|row| {
                    let amount = integer(
                        get(&row, "amount")
                            .filter(|v| truthy(v))
                            .unwrap_or(&Value::Int(0.into())),
                    )?;
                    Ok(((-amount, text(&row, "occurred_at"), text(&row, "id")), row))
                })
                .collect::<Result<Vec<_>>>()?;
            ordinary.sort_by(|a, b| a.0.cmp(&b.0));
            sources.extend(ordinary.into_iter().take(budget).map(|(_, row)| row));
        }
        let source_json: Vec<_> = sources.iter().map(codec::to_json).collect();
        let ranges = gaps::worklist_amount_ranges(&source_json)
            .map_err(|message| search_error(&source_json, message))?;
        if !ranges.is_empty() {
            let mut bound = vec![s(profile)];
            for (asset, low, high) in &ranges {
                bound.extend([s(asset), Value::Int(low.clone()), Value::Int(high.clone())]);
            }
            let bound = bound
                .iter()
                .enumerate()
                .map(|(i, v)| sql::bind(v, i + 1))
                .collect::<Result<Vec<_>>>()?;
            let ranges_sql =
                vec!["(t.asset=? AND t.amount BETWEEN ? AND ?)"; ranges.len()].join(" OR ");
            sources.extend(rows(c,&format!("{SELECT} WHERE t.profile_id=? AND t.excluded=0 AND t.direction='inbound' AND ({ranges_sql}) ORDER BY t.occurred_at,t.created_at,t.id LIMIT 512"),params_from_iter(bound))?);
        }
        sources.sort_by_key(|r| (text(r, "occurred_at"), text(r, "id")));
        sources
    };
    Ok((selected.iter().map(codec::to_json).collect(), large))
}

pub(super) fn profile(c: &Connection, scope: &Scope<'_>) -> Result<Value> {
    Ok(rows(c,"SELECT p.*,(SELECT COUNT(*) FROM transactions t WHERE t.profile_id=p.id AND t.excluded=0) AS active_count FROM profiles p WHERE p.id=?",[scope.profile_id])?.remove(0))
}

pub(super) struct Loaded {
    pub result: Value,
    pub display_rows: Vec<serde_json::Value>,
    pub journal_status: String,
}

pub(super) fn load(c: &Connection, scope: &Scope<'_>) -> Result<Loaded> {
    load_with(c, scope, true)
}

/// `load_gap_search_result(..., include_journal_claims=...)`: without journal
/// claims neither the stored gap inputs nor journal transfer entries suppress
/// a boundary, whatever the journal's freshness.
pub(super) fn load_with(c: &Connection, scope: &Scope<'_>, journal_claims: bool) -> Result<Loaded> {
    let profile = profile(c, scope)?;
    let count = integer(&value(&profile, "active_count"))?;
    let count = i64::try_from(count).map_err(|_| internal("transaction count overflow"))?;
    let version = |key| {
        integer(
            get(&profile, key)
                .filter(|v| truthy(v))
                .unwrap_or(&Value::Int(0.into())),
        )
    };
    let current = kassiber_store::semantics::compatible(
        i64::try_from(version("required_journal_semantics_version")?)
            .map_err(|_| internal("semantics overflow"))?,
        i64::try_from(version("journal_semantics_version")?)
            .map_err(|_| internal("semantics overflow"))?,
    ) && count > 0
        && truthy(&value(&profile, "last_processed_at"))
        && version("last_processed_tx_count")? == count.into()
        && version("journal_input_version")? == version("last_processed_input_version")?;
    let (worklist, large) = worklist(c, scope.profile_id, count)?;
    let mut ignored = BTreeSet::new();
    let mut accounting = BTreeSet::new();
    let claims = current && journal_claims;
    let stored = if claims {
        rows(
            c,
            "SELECT * FROM journal_custody_gap_inputs WHERE profile_id=? AND input_version=?",
            params![
                scope.profile_id,
                sql::bind(&value(&profile, "journal_input_version"), 2)?
            ],
        )?
        .into_iter()
        .next()
    } else {
        None
    };
    let stored = stored.and_then(|r| {
        let ids = parse_json(&text(&r, "ignored_ids_json")).ok()?;
        let accounting = parse_json(&text(&r, "accounting_ignored_ids_json")).ok()?;
        match (ids, accounting) {
            (Value::List(a), Value::List(b)) => Some((a, b)),
            _ => None,
        }
    });
    if let Some((a, b)) = stored {
        ignored.extend(a.iter().filter(|v| truthy(v)).map(py_str));
        accounting.extend(b.iter().filter(|v| truthy(v)).map(py_str));
    } else {
        // Active authored memberships include all modern pair/payout terms.
        // Raw active anchors also suppress contradictory/incomplete components.
        for row in rows(c,"SELECT COALESCE(l.transaction_id,l.anchor_transaction_id) AS transaction_id FROM custody_component_legs l JOIN custody_components c ON c.id=l.component_id WHERE c.profile_id=? AND (c.state='active' OR c.id IN (SELECT component_id FROM custody_component_transaction_memberships WHERE profile_id=?))",[scope.profile_id,scope.profile_id])? {
            if truthy(&value(&row,"transaction_id")) {ignored.insert(text(&row,"transaction_id"));}
        }
        for row in rows(c,"SELECT COALESCE(source.anchor_transaction_id,source.transaction_id) AS source_id,CASE WHEN term.term_kind='transaction_pair' THEN COALESCE(target.anchor_transaction_id,target.transaction_id) ELSE NULL END AS target_id FROM custody_component_economic_terms term JOIN custody_component_legs source ON source.id=term.source_leg_id JOIN custody_component_legs target ON target.id=term.target_leg_id WHERE term.profile_id=? AND term.component_id IN (SELECT component_id FROM custody_component_transaction_memberships WHERE profile_id=?)",[scope.profile_id,scope.profile_id])? {
            for key in ["source_id","target_id"] {if truthy(&value(&row,key)) {ignored.insert(text(&row,key));}}
        }
        if claims {
            for row in rows(c,"SELECT DISTINCT transaction_id FROM journal_entries WHERE profile_id=? AND entry_type IN ('transfer_out','transfer_in')",[scope.profile_id])? {
                ignored.insert(text(&row,"transaction_id"));
            }
        }
        accounting.clone_from(&ignored);
    }
    let result = gaps::compute(&worklist, large, &ignored, &accounting, 250)
        .map_err(|message| search_error(&worklist, message))?;
    let journal_status = if count == 0 {
        "no_transactions"
    } else if !truthy(&value(&profile, "last_processed_at")) {
        "not_processed"
    } else if current {
        "current"
    } else {
        "stale"
    }
    .into();
    let display_rows = worklist
        .into_iter()
        .filter(|r| !ignored.contains(kassiber_pytext::strip(r["id"].as_str().unwrap_or_default())))
        .collect();
    Ok(Loaded {
        result: codec::from_json(&result).map_err(internal)?,
        display_rows,
        journal_status,
    })
}

pub(super) fn find(c: &Connection, scope: &Scope<'_>, gap: &str) -> Result<Value> {
    let loaded = load(c, scope)?;
    let candidates = get(&loaded.result, "candidates");
    let list = if let Some(Value::List(v)) = candidates {
        v.as_slice()
    } else {
        &[]
    };
    list.iter()
        .find(|v| text(v, "gap_id") == gap)
        .cloned()
        .ok_or_else(|| {
            AppError::new(
                "custody_gap_not_found",
                "Custody gap not found in current evidence",
            )
            .with_hint("Reload the review queue; the evidence may have changed.")
        })
}
