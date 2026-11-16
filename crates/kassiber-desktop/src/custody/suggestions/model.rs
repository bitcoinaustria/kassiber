//! Transfer candidates as the suggestions card decides them (`components/review/transferModel.ts`,
//! `suggestionReason.ts`) and the review artifacts the bulk step previews (`reviewWorkflow.ts`).

use serde_json::Value;

use crate::i18n::{t, tc, tr};

pub fn s(v: &Value, k: &str) -> String {
    match v.get(k) {
        Some(Value::String(x)) => x.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn num(v: &Value, k: &str) -> f64 {
    v[k].as_f64().unwrap_or(0.0)
}

pub fn candidate_key(c: &Value) -> String {
    format!("{}->{}", s(c, "out_id"), s(c, "in_id"))
}

/// `candidateTerms`: a matching rule's kind and treatment, else the matcher's default.
pub fn candidate_terms(c: &Value) -> (String, String) {
    let rule = &c["rule_match"];
    let pick = |rk: &str, dk: &str| {
        rule[rk]
            .as_str()
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| s(c, dk))
    };
    (
        pick("kind", "default_kind"),
        pick("policy", "default_policy"),
    )
}

pub fn conflict_size(c: &Value) -> u64 {
    c["conflict_size"].as_u64().unwrap_or(1)
}

/// `needsReview`: not exact, or competing for one of its sides.
pub fn needs_review(c: &Value) -> bool {
    s(c, "confidence") != "exact" || conflict_size(c) > 1
}

/// `bulkPairable`: exact, no competitor, no ownership proof, tax-neutral; at most 50.
pub fn bulk_pairable(cands: &[Value]) -> Vec<Value> {
    cands
        .iter()
        .filter(|c| {
            s(c, "confidence") == "exact"
                && conflict_size(c) <= 1
                && s(c, "method") != "ownership_graph"
                && candidate_terms(c).1 == "carrying-value"
        })
        .take(50)
        .cloned()
        .collect()
}

const BITCOIN_SWAP_KINDS: [&str; 6] = [
    "chain-swap",
    "peg-in",
    "peg-out",
    "reverse-submarine-swap",
    "submarine-swap",
    "swap-refund",
];
pub const BITCOIN_ASSETS: [&str; 5] = ["BTC", "XBT", "XXBT", "LBTC", "L-BTC"];

pub fn is_bitcoin(asset: &str) -> bool {
    BITCOIN_ASSETS.contains(&asset.to_uppercase().as_str())
}

/// `moveType`: `transfer`, `bitcoinSwap` or `swap`.
pub fn move_type(c: &Value) -> &'static str {
    if s(c, "out_asset").to_uppercase() == s(c, "in_asset").to_uppercase() {
        return "transfer";
    }
    if s(c, "candidate_type") == "transfer"
        || BITCOIN_SWAP_KINDS.contains(&s(c, "default_kind").as_str())
    {
        return "bitcoinSwap";
    }
    "swap"
}

/// `formatBtc`: `₿ 0.25000000`.
fn format_btc(q: f64) -> String {
    format!("₿ {:.8}", q.abs())
}

/// `formatUiNumber(q, {maximumFractionDigits: 8})`.
fn ui_number(q: f64) -> String {
    let txt = format!("{q:.8}");
    let txt = txt.trim_end_matches('0').trim_end_matches('.').to_owned();
    let (int, frac) = txt
        .split_once('.')
        .map(|(a, b)| (a.to_owned(), Some(b.to_owned())))
        .unwrap_or((txt.clone(), None));
    let de = crate::i18n::lang() == crate::i18n::Lang::De;
    let mut g = String::new();
    for (i, ch) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            g.push(if de { '\u{a0}' } else { ',' });
        }
        g.push(ch);
    }
    match frac {
        Some(f) => format!("{g}{}{f}", if de { ',' } else { '.' }),
        None => g,
    }
}

/// `legAmount`: ₿ for bitcoin on any rail, the asset's own unit otherwise (msat scale).
pub fn leg_amount(msat: f64, asset: &str) -> String {
    let q = msat.abs() / 100_000_000_000.0;
    if is_bitcoin(asset) {
        format_btc(q)
    } else {
        format!("{} {asset}", ui_number(q))
    }
}

/// `sats`: whole sats in the UI locale.
pub fn sats(msat: f64) -> String {
    format!(
        "{} sats",
        crate::format::count((msat.abs() / 1000.0).round() as usize)
    )
}

/// `formatShortDate`: `2024-03-01 10:00`.
pub fn short_date(v: &str) -> String {
    if v.is_empty() {
        return "—".into();
    }
    let n = v.replace('T', " ");
    let n = n.strip_suffix('Z').unwrap_or(&n).to_owned();
    n.chars().take(16).collect()
}

fn epoch(v: &str) -> Option<f64> {
    crate::format::parse_ts(v)
}

/// `suggestionReason`: the shared evidence, otherwise how close the amounts and times are.
pub fn reason(c: &Value) -> String {
    match s(c, "method").as_str() {
        "payment_hash" => t("review:inbox.transfers.reason.paymentHash"),
        "provider_swap_id" => t("review:inbox.transfers.reason.provider"),
        "htlc_refund" => t("review:inbox.transfers.reason.refund"),
        "ownership_graph" => t("review:inbox.transfers.reason.ownership"),
        _ => {
            let minutes = match (
                epoch(&s(c, "out_occurred_at")),
                epoch(&s(c, "in_occurred_at")),
            ) {
                (Some(a), Some(b)) => Some(((b - a).abs() / 60.0).round() as usize),
                _ => None,
            };
            let time = match minutes {
                None => String::new(),
                Some(m) if m < 90 => tc("review:inbox.transfers.reason.minutes", m, &[]),
                Some(m) if m < 48 * 60 => tc(
                    "review:inbox.transfers.reason.hours",
                    ((m as f64) / 60.0).round() as usize,
                    &[],
                ),
                Some(m) => tc(
                    "review:inbox.transfers.reason.days",
                    ((m as f64) / 1440.0).round() as usize,
                    &[],
                ),
            };
            let fee = num(c, "swap_fee_msat");
            let key = if fee == 0.0 {
                "sameAmount"
            } else if fee > 0.0 {
                "closeAmount"
            } else {
                "similarAmount"
            };
            tr(
                &format!("review:inbox.transfers.reason.{key}"),
                &[("time", &time)],
            )
        }
    }
}

// ---- Review artifacts (reviewWorkflow.ts) ------------------------------------------------------

fn effects_ok(v: &Value) -> bool {
    let c = |k: &str| v[k].as_u64().is_some();
    v.is_object()
        && c("entries_count")
        && c("quarantine_count")
        && v["report_ready"].is_boolean()
        && v["quarantines"].is_array()
}

/// `reviewArtifact`: a schema-1 preview the dialog may confirm.
pub fn artifact_ok(v: &Value) -> bool {
    let digest = v["digest"].as_str().unwrap_or("");
    v["schema_version"] == 1
        && v["workspace_id"].is_string()
        && v["profile_id"].is_string()
        && v["base_input_version"].as_u64().is_some()
        && digest.len() == 64
        && digest
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        && v["operations"].as_array().is_some_and(|o| {
            !o.is_empty()
                && o.len() <= 50
                && o.iter().all(|x| x.is_object() && x["type"].is_string())
        })
        && effects_ok(&v["before"])
        && effects_ok(&v["after"])
}

/// `reviewTaxDelta`: the tax context and changed years, or None for an older artifact.
pub fn tax_delta(a: &Value) -> Option<(Value, Vec<Value>)> {
    let after = if a["after"].is_object() {
        &a["after"]
    } else {
        &a["verification"]
    };
    let tax = &after["tax"];
    let years = a["delta"]["years"].as_array()?;
    if !tax["fiat_currency"].is_string() || !tax["years"].is_array() {
        return None;
    }
    if !years.iter().all(|y| {
        y["year"].as_i64().is_some()
            && y["realized_gain_loss"].is_string()
            && y["income"].is_string()
    }) {
        return None;
    }
    Some((tax.clone(), years.clone()))
}

/// A random v4-style UUID for idempotency keys (`crypto.randomUUID()`).
pub fn uuid() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0x9e37_79b9_7f4a_7c15);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut x = nanos
        ^ N.fetch_add(0x9e37_79b9_7f4a_7c15, Ordering::Relaxed)
        ^ (std::process::id() as u64).rotate_left(32);
    let mut next = || {
        x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = x;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    };
    let (a, b) = (next(), next());
    format!(
        "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        a >> 32,
        (a >> 16) & 0xffff,
        a & 0xfff,
        (b >> 48) & 0x3fff | 0x8000,
        b & 0xffff_ffff_ffff
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cand(conf: &str, conflict: u64, method: &str, policy: &str) -> Value {
        json!({"out_id":"o","in_id":"i","confidence":conf,"conflict_size":conflict,"method":method,"default_kind":"manual","default_policy":policy,"out_asset":"BTC","in_asset":"BTC","swap_fee_msat":0})
    }

    #[test]
    fn bulk_takes_only_exact_neutral_unique() {
        let c = vec![
            cand("exact", 1, "provider_swap_id", "carrying-value"),
            cand("strong", 1, "heuristic", "carrying-value"),
            cand("exact", 2, "payment_hash", "carrying-value"),
            cand("exact", 1, "ownership_graph", "carrying-value"),
            cand("exact", 1, "payment_hash", "taxable"),
        ];
        assert_eq!(bulk_pairable(&c).len(), 1);
        assert!(!needs_review(&c[0]));
        assert!(needs_review(&c[1]));
        assert!(needs_review(&c[2]));
    }

    #[test]
    fn rule_terms_win() {
        let mut c = cand("exact", 1, "heuristic", "carrying-value");
        c["rule_match"] =
            json!({"rule_id":"r","rule_name":null,"kind":"peg-in","policy":"taxable"});
        assert_eq!(candidate_terms(&c), ("peg-in".into(), "taxable".into()));
        assert!(bulk_pairable(&[c]).is_empty());
    }

    #[test]
    fn amounts_and_dates() {
        assert_eq!(leg_amount(25_000_000_000.0, "BTC"), "₿ 0.25000000");
        assert_eq!(leg_amount(12_050_000_000_000.0, "USDT"), "120.5 USDT");
        assert_eq!(short_date("2024-03-01T10:00:00Z"), "2024-03-01 10:00");
    }

    #[test]
    fn reasons_read_the_times() {
        crate::i18n::set_lang(crate::i18n::Lang::En);
        let mut c = cand("strong", 1, "heuristic", "carrying-value");
        c["out_occurred_at"] = json!("2024-03-01T10:00:00Z");
        c["in_occurred_at"] = json!("2024-03-01T10:20:00Z");
        assert_eq!(reason(&c), "Same amount, 20 min apart");
        c["swap_fee_msat"] = json!(1000);
        c["in_occurred_at"] = json!("2024-03-03T10:00:00Z");
        assert_eq!(reason(&c), "Same amount less a fee, 2 days apart");
    }
}
