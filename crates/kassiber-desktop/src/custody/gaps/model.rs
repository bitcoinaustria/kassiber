//! The custody-gap model (`routes/custodyGapsModel.ts`): gaps and their pages, the action a gap
//! offers, the review plan's arguments (bound to the plan's input version), coverage, and the
//! exact amounts and dates the tabs write. Amounts stay exact (`i128` msat), never floats.

use serde_json::{Value, json};

use crate::custody::model::{exact, opt, s};

/// `CustodyGap` (the fields the tab uses, plus the raw record).
#[derive(Clone, Debug, Default)]
pub struct Gap {
    pub gap_id: String,
    pub status: String,
    pub status_reason: Option<String>,
    pub asset: String,
    pub source_wallet_label: String,
    pub destination_wallet_labels: Vec<String>,
    pub source_total_msat: Value,
    pub source_fee_msat: Value,
    pub source_debit_msat: Value,
    pub return_total_msat: Value,
    pub residual_msat: Value,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub confidence: String,
    pub promotion_eligible: bool,
    pub reason_codes: Vec<String>,
    /// `residual_classification`: (classification, amount_msat).
    pub residual_classification: Option<(String, Value)>,
    pub affected_disposals: i64,
    pub affected_years: Vec<i64>,
}

impl Gap {
    pub fn parse(v: &Value) -> Gap {
        let strings = |k: &str| {
            v[k].as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default()
        };
        let rc = &v["residual_classification"];
        Gap {
            gap_id: s(v, "gap_id"),
            status: s(v, "status"),
            status_reason: opt(v, "status_reason"),
            asset: s(v, "asset"),
            source_wallet_label: s(v, "source_wallet_label"),
            destination_wallet_labels: strings("destination_wallet_labels"),
            source_total_msat: v["source_total_msat"].clone(),
            source_fee_msat: v["source_fee_msat"].clone(),
            source_debit_msat: v["source_debit_msat"].clone(),
            return_total_msat: v["return_total_msat"].clone(),
            residual_msat: v["residual_msat"].clone(),
            started_at: opt(v, "started_at"),
            ended_at: opt(v, "ended_at"),
            confidence: s(v, "confidence"),
            promotion_eligible: v["promotion_eligible"].as_bool().unwrap_or(false),
            reason_codes: strings("reason_codes"),
            residual_classification: rc
                .is_object()
                .then(|| (s(rc, "classification"), rc["amount_msat"].clone())),
            affected_disposals: v["downstream"]["affected_disposals"].as_i64().unwrap_or(0),
            affected_years: v["downstream"]["affected_years"]
                .as_array()
                .map(|a| a.iter().filter_map(Value::as_i64).collect())
                .unwrap_or_default(),
        }
    }

    /// `gapDestinations`.
    pub fn destinations(&self) -> String {
        if self.destination_wallet_labels.is_empty() {
            "—".into()
        } else {
            self.destination_wallet_labels.join(", ")
        }
    }

    /// Whether it sits in the review queue (needs review or conflicting evidence).
    pub fn in_queue(&self) -> bool {
        self.status == "needs_review" || self.status == "conflicting"
    }

    /// `hasResidual`: a residual that is not exactly zero (unreadable counts as one).
    pub fn has_residual(&self) -> bool {
        exact(&self.residual_msat).is_none_or(|m| m != 0)
    }
}

/// `collectCustodyGapPages`: the pages' gaps once each, in order.
pub fn collect_pages(pages: &[Value]) -> Vec<Gap> {
    let mut seen = std::collections::HashSet::new();
    let mut out = vec![];
    for p in pages {
        for g in p["gaps"].as_array().into_iter().flatten() {
            let gap = Gap::parse(g);
            if seen.insert(gap.gap_id.clone()) {
                out.push(gap);
            }
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionMode {
    Create,
    Reopen,
    Revise,
    None,
}

/// `custodyGapActionMode`.
pub fn action_mode(g: &Gap) -> ActionMode {
    match g.status.as_str() {
        "resolved" => ActionMode::Reopen,
        "needs_review" if g.status_reason.as_deref() == Some("bridge_reopened") => {
            ActionMode::Revise
        }
        "needs_review" | "conflicting" => ActionMode::Create,
        _ => ActionMode::None,
    }
}

/// `shouldOfferResidualClassification`.
pub fn offer_residual(g: &Gap) -> bool {
    g.status == "resolved"
        && g.residual_classification.is_none()
        && exact(&g.residual_msat).is_some_and(|m| m > 0)
}

/// `canShowNoKnownCustodyGaps`.
pub fn can_show_clear(summary: &Value, review_count: usize) -> bool {
    review_count == 0
        && summary["needs_review"].as_i64().unwrap_or(0) == 0
        && summary["conflicting"].as_i64().unwrap_or(0) == 0
        && summary["derived_state_current"].as_bool() == Some(true)
        && summary["search_complete"].as_bool() != Some(false)
        && summary["canonical_issue_count"].as_i64().unwrap_or(0) == 0
}

pub const RESIDUAL_CLASSIFICATIONS: [&str; 6] = [
    "external_payment",
    "external_disposal",
    "external_gift",
    "external_loss",
    "retained_custody",
    "suspense_continuation",
];

/// `formatCustodyMsat`: the exact quantity and its asset code (`0.05905 BTC`).
pub fn fmt_msat(v: &Value, asset: &str) -> String {
    // BigInt() of a fractional number throws in the web: unreadable, like text that is no integer.
    let msat = match v {
        Value::Number(n) if n.is_f64() && n.as_f64().is_some_and(|f| f.fract() != 0.0) => None,
        _ => exact(v),
    };
    let Some(m) = msat else {
        return format!("— {asset}");
    };
    let sign = if m < 0 { "−" } else { "" };
    let a = m.unsigned_abs();
    let whole = a / 100_000_000_000;
    let frac = format!("{:011}", a % 100_000_000_000);
    let frac = frac.trim_end_matches('0');
    if frac.is_empty() {
        format!("{sign}{whole} {asset}")
    } else {
        format!("{sign}{whole}.{frac} {asset}")
    }
}

const MONTHS_EN: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
/// `Intl.DateTimeFormat("de", {month: "short"})`.
const MONTHS_DE: [&str; 12] = [
    "Jan.", "Feb.", "März", "Apr.", "Mai", "Juni", "Juli", "Aug.", "Sept.", "Okt.", "Nov.", "Dez.",
];

/// `formatDate(value, i18n.language)`: `Aug 30, 2022` / `30. Aug. 2022`, or a dash.
pub fn fmt_date(v: Option<&str>) -> String {
    let Some(s) = v.filter(|s| !s.is_empty()) else {
        return "—".into();
    };
    let Some(d) = crate::format::Day::parse(s) else {
        return s.to_owned();
    };
    match crate::i18n::lang() {
        crate::i18n::Lang::En => format!("{} {}, {}", MONTHS_EN[d.m as usize - 1], d.d, d.y),
        crate::i18n::Lang::De => format!("{}. {} {}", d.d, MONTHS_DE[d.m as usize - 1], d.y),
    }
}

/// The plan request for an action (`bridgePreviewArgs`, `reopenPreviewArgs`, …).
pub fn plan_args(action: &str, gap_id: &str, reason: &str, classification: Option<&str>) -> Value {
    let mut a = json!({"action": action, "gap_id": gap_id});
    if let Some(c) = classification {
        a["classification"] = json!(c);
    }
    let r = reason.trim();
    if !r.is_empty() && action != "create" && action != "dismiss" {
        a["reason"] = json!(r);
    }
    a
}

/// The apply request bound to the plan's input version (`bridgeCreateArgs`, `reopenConfirmArgs`,
/// `reviseConfirmArgs`, `residualConfirmArgs`, the dismissal).
pub fn apply_args(action: &str, preview: &Value, reason: &str) -> Value {
    let mut a = json!({"gap_id": preview["gap_id"], "action": action, "expected_input_version": preview["input_version"]});
    if action == "classify_residual" {
        a["classification"] = preview["classification"].clone();
    }
    let r = reason.trim();
    if !r.is_empty() && matches!(action, "reopen" | "revise" | "classify_residual") {
        a["reason"] = json!(r);
    }
    a
}

/// Whether a preview may be confirmed: a bridge or a revision or residual decision only when
/// the core says it is activatable; a reopen always (after the explicit confirmation).
pub fn confirmable(action: &str, preview: &Value) -> bool {
    match action {
        "reopen" => true,
        _ => preview["activatable"].as_bool() == Some(true),
    }
}

/// A count as the web interpolates it.
pub fn num(v: &Value) -> String {
    match v {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        _ => "—".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gap(v: Value) -> Gap {
        Gap::parse(&v)
    }

    #[test]
    fn formats_exact_quantities() {
        assert_eq!(fmt_msat(&json!(5905070000u64), "BTC"), "0.0590507 BTC");
        assert_eq!(fmt_msat(&json!("100000000000"), "LBTC"), "1 LBTC");
        assert_eq!(fmt_msat(&json!(-1500), "BTC"), "−0.000000015 BTC");
        assert_eq!(fmt_msat(&json!("x"), "BTC"), "— BTC");
        assert_eq!(fmt_msat(&json!(1.5), "BTC"), "— BTC");
    }

    #[test]
    fn action_modes() {
        assert_eq!(
            action_mode(&gap(json!({"status": "resolved"}))),
            ActionMode::Reopen
        );
        assert_eq!(
            action_mode(&gap(
                json!({"status": "needs_review", "status_reason": "bridge_reopened"})
            )),
            ActionMode::Revise
        );
        assert_eq!(
            action_mode(&gap(json!({"status": "conflicting"}))),
            ActionMode::Create
        );
        assert_eq!(
            action_mode(&gap(json!({"status": "dismissed"}))),
            ActionMode::None
        );
    }

    #[test]
    fn residual_offer() {
        assert!(offer_residual(&gap(
            json!({"status": "resolved", "residual_msat": "10"})
        )));
        assert!(!offer_residual(&gap(
            json!({"status": "resolved", "residual_msat": 0})
        )));
        assert!(!offer_residual(&gap(
            json!({"status": "resolved", "residual_msat": 5, "residual_classification": {"classification": "external_payment", "amount_msat": 5}})
        )));
        assert!(!offer_residual(&gap(
            json!({"status": "needs_review", "residual_msat": 5})
        )));
    }

    #[test]
    fn clear_needs_a_complete_current_search() {
        let base = json!({"needs_review": 0, "conflicting": 0, "derived_state_current": true, "canonical_issue_count": 0});
        assert!(can_show_clear(&base, 0));
        let mut s = base.clone();
        s["search_complete"] = json!(false);
        assert!(!can_show_clear(&s, 0));
        let mut s = base.clone();
        s["derived_state_current"] = json!(false);
        assert!(!can_show_clear(&s, 0));
        assert!(!can_show_clear(&base, 1));
    }

    #[test]
    fn plans_bind_the_input_version() {
        assert_eq!(
            plan_args("create", "g", " x ", None),
            json!({"action": "create", "gap_id": "g"})
        );
        assert_eq!(
            plan_args("reopen", "g", "  why ", None),
            json!({"action": "reopen", "gap_id": "g", "reason": "why"})
        );
        assert_eq!(
            plan_args("classify_residual", "g", "", Some("external_gift")),
            json!({"action": "classify_residual", "gap_id": "g", "classification": "external_gift"})
        );
        let p = json!({"gap_id": "g", "input_version": 45, "classification": "external_loss"});
        assert_eq!(
            apply_args("classify_residual", &p, "n"),
            json!({"gap_id": "g", "action": "classify_residual", "classification": "external_loss", "expected_input_version": 45, "reason": "n"})
        );
        assert_eq!(
            apply_args("create", &p, "ignored"),
            json!({"gap_id": "g", "action": "create", "expected_input_version": 45})
        );
    }

    #[test]
    fn collects_pages_once() {
        let pages = vec![
            json!({"gaps": [{"gap_id": "a"}, {"gap_id": "b"}]}),
            json!({"gaps": [{"gap_id": "b"}, {"gap_id": "c"}]}),
        ];
        let ids: Vec<String> = collect_pages(&pages)
            .into_iter()
            .map(|g| g.gap_id)
            .collect();
        assert_eq!(ids, ["a", "b", "c"]);
    }
}
