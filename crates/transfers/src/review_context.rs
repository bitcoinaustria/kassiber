//! `build_swap_review_context_payload`: a deterministic, redacted review
//! bundle around suggested transfer/swap candidates. Every suggested action
//! still requires the user's consent; nothing here pairs or reprices.
use std::collections::BTreeSet;

use super::*;
use kassiber_envelope::redaction::redact_secret_text;
use kassiber_envelope::{s, truthy};
use kassiber_store::rusqlite::params_from_iter;

const KEYWORDS: &[&str] = &[
    "aqua",
    "boltz",
    "federation",
    "lbtc",
    "lightning",
    "liquid",
    "ln",
    "peg",
    "phoenix",
    "submarine",
    "swap",
    "tausch",
    "übertrag",
    "uebertrag",
];
const CANDIDATE_KEYS: &[&str] = &[
    "out_id",
    "in_id",
    "out_asset",
    "in_asset",
    "out_amount_msat",
    "out_amount",
    "in_amount_msat",
    "in_amount",
    "out_occurred_at",
    "in_occurred_at",
    "default_kind",
    "default_policy",
    "candidate_type",
];
const FILTER_KEYS: &[&str] = &[
    "confidence",
    "method",
    "asset_pair",
    "route_pair",
    "candidate_type",
    "time_window_seconds",
    "fee_pct_max",
    "fee_sats_min",
];

fn int(v: &Value) -> i64 {
    match v {
        Value::Int(n) => i64::try_from(n.clone()).unwrap_or_default(),
        Value::Bool(b) => i64::from(*b),
        Value::Float(n) => n.trunc() as i64,
        _ => 0,
    }
}
fn btc(msat: i64) -> Value {
    Value::Float(kassiber_pytext::parse_float(&format!("{msat}e-11")).unwrap())
}
/// `_safe_review_text`: redacted, stripped and bounded by code points.
fn safe(value: &Value, limit: usize) -> String {
    let raw = if truthy(value) {
        py_str(value)
    } else {
        String::new()
    };
    let text = redact_secret_text(kassiber_pytext::strip(&raw));
    if text.chars().count() <= limit {
        return text;
    }
    let head: String = text.chars().take(limit).collect();
    format!(
        "{}...<truncated>",
        head.trim_end_matches(|c: char| kassiber_pytext::is_space(u32::from(c)))
    )
}
fn mentions(fields: &[(&str, Vec<String>)]) -> Vec<Value> {
    let mut output = Vec::new();
    for (field, values) in fields {
        for value in values {
            let lowered = kassiber_pytext::lower(value);
            if let Some(keyword) = KEYWORDS.iter().find(|k| lowered.contains(**k)) {
                output.push(dict([
                    ("field", s(*field)),
                    ("keyword", s(*keyword)),
                    ("excerpt", s(safe(&s(value.as_str()), 160))),
                ]));
            }
        }
    }
    output
}
fn string(v: &Value) -> Option<&str> {
    match v {
        Value::Str(t) => t.as_str(),
        _ => None,
    }
}
fn placeholders(n: usize) -> String {
    vec!["?"; n].join(",")
}

fn summaries(conn: &Connection, ids: &[String]) -> Result<Vec<(String, Value)>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let found = rows(
        conn,
        &format!("SELECT t.id,t.external_id,t.occurred_at,t.confirmed_at,t.direction,t.asset,t.amount,t.fee,t.fiat_currency,t.fiat_rate,t.fiat_value,t.kind,t.description,t.counterparty,t.note,t.excluded,t.payment_hash,t.payment_hash_source,w.label AS wallet_label,w.kind AS wallet_kind FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE t.id IN ({})", placeholders(ids.len())),
        params_from_iter(ids),
    )?;
    let mut output: Vec<(String, Value)> = Vec::new();
    for row in found {
        let amount = int(&value(&row, "amount"));
        let fee = int(&value(&row, "fee"));
        let external = value(&row, "external_id");
        let summary = dict([
            ("transaction_id", value(&row, "id")),
            (
                "external_id",
                if truthy(&external) { external } else { s("") },
            ),
            ("occurred_at", value(&row, "occurred_at")),
            ("confirmed_at", value(&row, "confirmed_at")),
            ("direction", value(&row, "direction")),
            ("asset", value(&row, "asset")),
            ("amount_msat", Value::from(amount)),
            ("amount", btc(amount)),
            ("fee_msat", Value::from(fee)),
            ("fee", btc(fee)),
            ("fiat_currency", value(&row, "fiat_currency")),
            ("fiat_rate", value(&row, "fiat_rate")),
            ("fiat_value", value(&row, "fiat_value")),
            ("kind", value(&row, "kind")),
            ("description", s(safe(&value(&row, "description"), 240))),
            ("counterparty", s(safe(&value(&row, "counterparty"), 240))),
            ("note", s(safe(&value(&row, "note"), 240))),
            ("excluded", Value::Bool(truthy(&value(&row, "excluded")))),
            (
                "payment_hash_present",
                Value::Bool(truthy(&value(&row, "payment_hash"))),
            ),
            ("payment_hash_source", value(&row, "payment_hash_source")),
            (
                "wallet",
                dict([
                    ("label", value(&row, "wallet_label")),
                    ("kind", value(&row, "wallet_kind")),
                ]),
            ),
            ("tags", Value::List(vec![])),
            ("metadata_mentions", Value::List(vec![])),
        ]);
        let id = text(&row, "id");
        match output.iter_mut().find(|(k, _)| *k == id) {
            Some(slot) => slot.1 = summary,
            None => output.push((id, summary)),
        }
    }
    let tags = rows(
        conn,
        &format!("SELECT tt.transaction_id,tags.code,tags.label FROM transaction_tags tt JOIN tags ON tags.id=tt.tag_id WHERE tt.transaction_id IN ({}) ORDER BY tt.transaction_id ASC,tags.code ASC", placeholders(ids.len())),
        params_from_iter(ids),
    )?;
    for tag in tags {
        let id = text(&tag, "transaction_id");
        if let Some((_, summary)) = output.iter_mut().find(|(k, _)| *k == id) {
            if let Value::Dict(fields) = summary {
                if let Some((_, Value::List(list))) =
                    fields.iter_mut().find(|(k, _)| *k == "tags".into())
                {
                    list.push(dict([
                        ("code", value(&tag, "code")),
                        ("label", value(&tag, "label")),
                    ]));
                }
            }
        }
    }
    for (_, summary) in output.iter_mut() {
        let field = |key: &str| {
            let v = value(summary, key);
            if truthy(&v) {
                vec![py_str(&v)]
            } else {
                vec![]
            }
        };
        let tag_texts: Vec<String> = match value(summary, "tags") {
            Value::List(tags) => tags
                .iter()
                .map(|t| {
                    let label = value(t, "label");
                    if truthy(&label) {
                        label
                    } else {
                        value(t, "code")
                    }
                })
                .filter(truthy)
                .map(|v| py_str(&v))
                .collect(),
            _ => vec![],
        };
        let found = mentions(&[
            ("description", field("description")),
            ("counterparty", field("counterparty")),
            ("note", field("note")),
            ("tags", tag_texts),
        ]);
        kassiber_envelope::set(summary, "metadata_mentions", Value::List(found));
    }
    Ok(output)
}

type Journals = (Vec<(String, Vec<Value>)>, Vec<(String, Value)>);
fn journals(conn: &Connection, ids: &[String]) -> Result<Journals> {
    if ids.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let mut by_tx: Vec<(String, Vec<Value>)> =
        ids.iter().map(|id| (id.clone(), Vec::new())).collect();
    let found = rows(
        conn,
        &format!("SELECT id,transaction_id,occurred_at,entry_type,asset,quantity,fiat_value,cost_basis,proceeds,gain_loss,at_category,at_kennzahl,description FROM journal_entries WHERE transaction_id IN ({}) ORDER BY occurred_at ASC,created_at ASC,id ASC", placeholders(ids.len())),
        params_from_iter(ids),
    )?;
    for row in found {
        let quantity = int(&value(&row, "quantity"));
        let entry = dict([
            ("id", value(&row, "id")),
            ("occurred_at", value(&row, "occurred_at")),
            ("entry_type", value(&row, "entry_type")),
            ("asset", value(&row, "asset")),
            ("quantity_msat", Value::from(quantity)),
            ("quantity", btc(quantity)),
            ("fiat_value", value(&row, "fiat_value")),
            ("cost_basis", value(&row, "cost_basis")),
            ("proceeds", value(&row, "proceeds")),
            ("gain_loss", value(&row, "gain_loss")),
            ("at_category", value(&row, "at_category")),
            ("at_kennzahl", value(&row, "at_kennzahl")),
            ("description", s(safe(&value(&row, "description"), 240))),
        ]);
        let id = text(&row, "transaction_id");
        match by_tx.iter_mut().find(|(k, _)| *k == id) {
            Some((_, list)) => list.push(entry),
            None => by_tx.push((id, vec![entry])),
        }
    }
    let mut quarantines: Vec<(String, Value)> = Vec::new();
    for row in rows(
        conn,
        &format!("SELECT transaction_id,reason,detail_json,created_at FROM journal_quarantines WHERE transaction_id IN ({})", placeholders(ids.len())),
        params_from_iter(ids),
    )? {
        let raw = value(&row, "detail_json");
        let detail = match kassiber_pytext::loads(&if truthy(&raw) { py_str(&raw) } else { "{}".into() }) {
            Ok(Value::Dict(d)) => Value::Dict(d),
            _ => dict::<&str>([]),
        };
        let entry = dict([
            ("reason", value(&row, "reason")),
            ("detail", detail),
            ("created_at", value(&row, "created_at")),
        ]);
        let id = text(&row, "transaction_id");
        match quarantines.iter_mut().find(|(k, _)| *k == id) {
            Some(slot) => slot.1 = entry,
            None => quarantines.push((id, entry)),
        }
    }
    Ok((by_tx, quarantines))
}

fn confidence(candidate: &Value) -> Value {
    let method = value(candidate, "method");
    let level = value(candidate, "confidence");
    let (reason, needs) = match string(&method) {
        Some("payment_hash") => ("both legs share a Lightning payment_hash".to_owned(), false),
        Some("htlc_refund") => ("the inbound refund spends the outbound's on-chain HTLC funding output (deterministic link, same-wallet safe)".to_owned(), false),
        Some("provider_swap_id") => {
            let evidence = value(candidate, "evidence");
            let provider = value(&evidence, "provider");
            let provider = if truthy(&provider) { py_str(&provider) } else { "provider".into() };
            (format!("both legs share redacted {provider} swap metadata"), false)
        }
        _ => ("legs match by direction, amount delta, wallets, and time window".to_owned(), true),
    };
    dict([
        ("confidence", level),
        ("method", method),
        ("reason", s(reason)),
        ("needs_human_confirmation", Value::Bool(needs)),
    ])
}
fn fee(candidate: &Value, fee_pct_max: f64, fee_sats_min: i64) -> Value {
    let fee = int(&value(candidate, "swap_fee_msat"));
    let out = int(&value(candidate, "out_amount_msat")).abs();
    let threshold = ((out as f64 * fee_pct_max).trunc() as i64).max(fee_sats_min * 1000);
    let pct = if out != 0 {
        Value::Float(fee as f64 / out as f64)
    } else {
        Value::Null
    };
    let assessment = if fee < 0 {
        "anomaly_inbound_exceeds_outbound"
    } else if fee == 0 {
        "no_fee_detected"
    } else if threshold != 0 && fee > threshold {
        "above_default_heuristic_threshold"
    } else {
        "normal"
    };
    dict([
        ("swap_fee_msat", Value::from(fee)),
        ("swap_fee", btc(fee)),
        ("swap_fee_kind", value(candidate, "swap_fee_kind")),
        ("pct_of_out", pct),
        ("default_threshold_msat", Value::from(threshold)),
        ("assessment", s(assessment)),
    ])
}
fn suggested(candidate: &Value, conflict_size: i64, assessment: &str) -> Value {
    let arguments = dict([
        ("tx_out", value(candidate, "out_id")),
        ("tx_in", value(candidate, "in_id")),
        ("kind", value(candidate, "default_kind")),
        ("policy", value(candidate, "default_policy")),
        ("confidence_at_pair", value(candidate, "confidence")),
    ]);
    let plain = |action: &str, reason: &str| {
        dict([
            ("action", s(action)),
            ("requires_consent", Value::Bool(false)),
            ("reason", s(reason)),
        ])
    };
    let pair = |action: &str, reason: &str| {
        dict([
            ("action", s(action)),
            ("daemon_kind", s("ui.transfers.pair")),
            ("arguments", arguments.clone()),
            ("requires_consent", Value::Bool(true)),
            ("reason", s(reason)),
        ])
    };
    if conflict_size > 1 {
        return plain(
            "resolve_conflict",
            "candidate shares a leg with other candidates",
        );
    }
    if assessment == "anomaly_inbound_exceeds_outbound" {
        return plain(
            "inspect_before_pairing",
            "computed fee is negative, so the legs may be reversed or mismatched",
        );
    }
    if value(candidate, "default_policy") == s("taxable") {
        return pair(
            "pair_taxable_after_confirmation",
            "this profile keeps the cross-asset swap as SELL + BUY while recording the audit link",
        );
    }
    if value(candidate, "confidence") == s("exact") {
        let reason = match string(&value(candidate, "method")) {
            Some("htlc_refund") => "the inbound refund deterministically spends the outbound's HTLC funding output, and is non-conflicted",
            Some("provider_swap_id") => "provider swap metadata is exact and non-conflicted",
            _ => "payment_hash identity is exact and non-conflicted",
        };
        return pair("pair_exact_candidate", reason);
    }
    pair(
        "ask_user_to_confirm_then_pair",
        "heuristic candidates need human confirmation before pairing",
    )
}
fn impact(
    out_entries: &[Value],
    in_entries: &[Value],
    quarantines: &[Value],
    policy: &str,
) -> Value {
    let types: BTreeSet<String> = out_entries
        .iter()
        .chain(in_entries)
        .map(|e| value(e, "entry_type"))
        .filter(truthy)
        .map(|v| py_str(&v))
        .collect();
    let (status, summary) = if !quarantines.is_empty() {
        (
            "blocked_by_quarantine",
            "one or both legs are quarantined, so report impact is not trustworthy yet",
        )
    } else if out_entries.is_empty() && in_entries.is_empty() {
        (
            "no_current_journal_entries",
            "journals do not currently expose entries for these legs",
        )
    } else if policy == "carrying-value" {
        ("would_remain_separate_until_paired", "leaving this unpaired keeps the legs as separate journal events; pairing preserves principal and surfaces the swap fee")
    } else {
        (
            "audit_link_only_by_default",
            "default policy records a reviewed link but leaves SELL + BUY tax treatment in place",
        )
    };
    dict([
        ("status", s(status)),
        ("summary", s(summary)),
        (
            "entry_types",
            Value::List(types.into_iter().map(s).collect()),
        ),
        ("out_entry_count", Value::from(out_entries.len() as i64)),
        ("in_entry_count", Value::from(in_entries.len() as i64)),
        ("quarantine_count", Value::from(quarantines.len() as i64)),
    ])
}

/// `payload` carries the transport's validated arguments and the shared
/// suggestion and saved-view reads, which the book adapter computed.
pub(crate) fn execute(conn: &Connection, scope: &Scope<'_>, payload: &Value) -> Result<Value> {
    let args = value(payload, "args");
    let suggestion = value(payload, "suggestion");
    let limit = int(&value(payload, "limit")) as usize;
    let fee_pct_max = match value(payload, "fee_pct_max") {
        Value::Float(n) => n,
        other => int(&other) as f64,
    };
    let fee_sats_min = int(&value(payload, "fee_sats_min"));
    let candidates = match value(&suggestion, "candidates") {
        Value::List(items) => items,
        _ => Vec::new(),
    };
    let limited: Vec<&Value> = candidates.iter().take(limit).collect();
    let mut ids: Vec<String> = Vec::new();
    for candidate in &limited {
        for key in ["out_id", "in_id"] {
            if let Some(id) = string(&value(candidate, key)).filter(|v| !v.is_empty()) {
                if !ids.iter().any(|i| i == id) {
                    ids.push(id.to_owned());
                }
            }
        }
    }
    let tx_summaries = summaries(conn, &ids)?;
    let (journal_by_tx, quarantines_by_tx) = journals(conn, &ids)?;
    let leg = |candidate: &Value, key: &str| {
        let v = value(candidate, key);
        if truthy(&v) {
            py_str(&v)
        } else {
            String::new()
        }
    };
    let mut items = Vec::new();
    for candidate in &limited {
        let out_id = leg(candidate, "out_id");
        let in_id = leg(candidate, "in_id");
        let conflict_size = int(&value(candidate, "conflict_size"));
        let fee = fee(candidate, fee_pct_max, fee_sats_min);
        let entries = |id: &str| {
            journal_by_tx
                .iter()
                .find(|(k, _)| k == id)
                .map(|(_, v)| v.clone())
                .unwrap_or_default()
        };
        let (out_entries, in_entries) = (entries(&out_id), entries(&in_id));
        let quarantines: Vec<Value> = [&out_id, &in_id]
            .into_iter()
            .filter_map(|id| {
                quarantines_by_tx
                    .iter()
                    .find(|(k, _)| k == id)
                    .map(|(_, v)| v.clone())
            })
            .filter(truthy)
            .collect();
        let summary = |id: &str| {
            tx_summaries
                .iter()
                .find(|(k, _)| k == id)
                .map(|(_, v)| v.clone())
                .unwrap_or_else(|| dict([("transaction_id", s(id))]))
        };
        let (out_summary, in_summary) = (summary(&out_id), summary(&in_id));
        let mut metadata = Vec::new();
        for side in [&out_summary, &in_summary] {
            if let Value::List(list) = value(side, "metadata_mentions") {
                metadata.extend(list);
            }
        }
        let assessment = py_str(&value(&fee, "assessment"));
        items.push(dict([
            (
                "candidate",
                dict(CANDIDATE_KEYS.iter().map(|k| (*k, value(candidate, k)))),
            ),
            ("out", out_summary),
            ("in", in_summary),
            ("confidence", confidence(candidate)),
            ("fee", fee),
            (
                "conflict",
                dict([
                    ("set_id", s(leg(candidate, "conflict_set_id"))),
                    ("candidate_count", Value::from(conflict_size)),
                    ("requires_manual_resolution", Value::Bool(conflict_size > 1)),
                ]),
            ),
            ("rule_match", value(candidate, "rule_match")),
            ("metadata_mentions", Value::List(metadata)),
            (
                "current_journal",
                dict([
                    ("out_entries", Value::List(out_entries.clone())),
                    ("in_entries", Value::List(in_entries.clone())),
                    ("quarantines", Value::List(quarantines.clone())),
                ]),
            ),
            (
                "report_impact_if_left_unpaired",
                impact(
                    &out_entries,
                    &in_entries,
                    &quarantines,
                    &leg(candidate, "default_policy"),
                ),
            ),
            (
                "suggested_action",
                suggested(candidate, conflict_size, &assessment),
            ),
        ]));
    }
    let list = |v: Value| match v {
        Value::List(items) => items,
        _ => Vec::new(),
    };
    let pairs = list(records::pairs(conn, scope)?);
    let rules = list(rules::list(conn, scope)?);
    let views = list(value(payload, "saved_views"));
    let profile = rows(
        conn,
        "SELECT w.label AS workspace,p.label AS profile,p.tax_country FROM profiles p JOIN workspaces w ON w.id=? WHERE p.id=?",
        [scope.workspace_id, scope.profile_id],
    )?
    .remove(0);
    let counts = value(&suggestion, "counts");
    let count = |key: &str| Value::from(int(&value(&counts, key)));
    let filters: Vec<(&str, Value)> = FILTER_KEYS
        .iter()
        .filter_map(|k| {
            get(&args, k)
                .filter(|v| **v != Value::Null)
                .map(|v| (*k, v.clone()))
        })
        .collect();
    let bounded = |items: &[Value]| Value::List(items.iter().take(limit).cloned().collect());
    Ok(dict([
        (
            "summary",
            dict([
                ("workspace", value(&profile, "workspace")),
                ("profile", value(&profile, "profile")),
                ("tax_country", value(&profile, "tax_country")),
                ("candidate_count", count("total")),
                ("exact_candidates", count("exact")),
                ("strong_candidates", count("strong")),
                ("conflict_clusters", count("conflicts")),
                ("rule_matches", count("rule_matches")),
                ("review_items", Value::from(items.len() as i64)),
                ("active_pairs", Value::from(pairs.len() as i64)),
                ("rules", Value::from(rules.len() as i64)),
                ("saved_views", Value::from(views.len() as i64)),
                ("limit", Value::from(limit as i64)),
            ]),
        ),
        ("filters", dict(filters)),
        (
            "counts",
            if counts == Value::Null {
                dict::<&str>([])
            } else {
                counts
            },
        ),
        ("review_items", Value::List(items)),
        ("active_pairs", bounded(&pairs)),
        ("rules", bounded(&rules)),
        ("saved_views", bounded(&views)),
    ]))
}
