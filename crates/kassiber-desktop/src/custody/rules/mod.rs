//! Linking rules and saved views of the suggestions (`routes/transfers/RulesSheet.tsx`,
//! `rulesModel.ts`), in a sheet off the page's menu. A rule marks the suggestions it matches
//! and sets the kind and tax treatment their link records; the owner still confirms each link.
//! A saved view filters the suggestions; new ones are not made here.

mod view;

use rune::{Cx, El, TextEdit, Tx};
use serde_json::{Value, json};

use crate::app::{App, Reply};
use crate::i18n::{t, tr};
use crate::theme::Tk;

pub const PAIR_KIND_OPTIONS: [&str; 9] = [
    "manual",
    "coinjoin",
    "whirlpool",
    "chain-swap",
    "peg-in",
    "peg-out",
    "reverse-submarine-swap",
    "submarine-swap",
    "swap-refund",
];
pub const PAIR_POLICY_OPTIONS: [&str; 2] = ["carrying-value", "taxable"];
pub const WALLET_KIND_OPTIONS: [&str; 8] = [
    "phoenix",
    "coreln",
    "lnd",
    "nwc",
    "descriptor",
    "xpub",
    "address",
    "custom",
];

/// The "Create linking rule" form (`CreateRuleDialog`).
pub struct Form {
    pub name: TextEdit,
    pub out_asset: String,
    pub in_asset: String,
    pub out_kind: String,
    pub in_kind: String,
    pub max_fee: TextEdit,
    pub min_confidence: String,
    pub kind: String,
    pub policy: String,
}

impl Default for Form {
    fn default() -> Self {
        Form {
            name: TextEdit::new(""),
            out_asset: "any".into(),
            in_asset: "any".into(),
            out_kind: "any".into(),
            in_kind: "any".into(),
            max_fee: TextEdit::new(""),
            min_confidence: "strong".into(),
            kind: "submarine-swap".into(),
            policy: "carrying-value".into(),
        }
    }
}

#[derive(Default)]
pub struct State {
    pub open: bool,
    pub rules: Option<Vec<Value>>,
    pub rules_loading: bool,
    /// Saved views of both surfaces (transfer candidates, then swap candidates).
    pub transfer_views: Vec<Value>,
    pub swap_views: Vec<Value>,
    pub create_open: bool,
    pub form: Form,
    pub creating: bool,
    pub confirm_delete: Option<String>,
    pub error: Option<String>,
    /// The open select of the form, by its key.
    pub select: Option<String>,
}

fn st(app: &mut App) -> &mut State {
    &mut app.custody.rules
}

/// Opens the sheet; its reads run while it is open (`enabled: open`).
pub fn open(app: &mut App) {
    st(app).open = true;
    st(app).error = None;
    read_rules(app);
    read_views(app);
}

pub fn close(app: &mut App) {
    let s = st(app);
    s.open = false;
    s.confirm_delete = None;
    s.select = None;
}

fn read_rules(app: &mut App) {
    if st(app).rules.is_none() {
        st(app).rules_loading = true;
    }
    app.send("ui.transfers.rules.list", Value::Null, "custody.rules.list");
}

fn read_views(app: &mut App) {
    app.send(
        "ui.saved_views.list",
        json!({"surface": "transfer_candidates"}),
        "custody.rules.views_transfer",
    );
    app.send(
        "ui.saved_views.list",
        json!({"surface": "swap_candidates"}),
        "custody.rules.views_swap",
    );
}

/// Mounts the sheet (and its dialog) into `layer` while open.
pub fn sheet(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) {
    view::sheet(app, cx, tk, layer);
}

pub fn poll(_app: &mut App, _tx: &mut Tx) -> bool {
    false
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    let failed = || {
        r.error
            .clone()
            .filter(|e| !e.is_empty())
            .unwrap_or_else(|| t("review:swap.rules.failed"))
    };
    match tag {
        "custody.rules.list" => {
            let s = st(app);
            s.rules_loading = false;
            if r.error.is_none() {
                s.rules = Some(r.data["rules"].as_array().cloned().unwrap_or_default());
            }
        }
        "custody.rules.views_transfer" | "custody.rules.views_swap" => {
            if r.error.is_none() {
                let views = r.data["views"].as_array().cloned().unwrap_or_default();
                let s = st(app);
                if tag.ends_with("transfer") {
                    s.transfer_views = views
                } else {
                    s.swap_views = views
                }
            }
        }
        "custody.rules.create" => {
            st(app).creating = false;
            match &r.error {
                // The dialog stays open with what was typed; the sheet says what went wrong.
                Some(_) => st(app).error = Some(failed()),
                None => {
                    let s = st(app);
                    s.create_open = false;
                    s.form = Form::default();
                    read_rules(app);
                }
            }
        }
        "custody.rules.set_enabled" | "custody.rules.delete" => {
            if r.error.is_some() {
                st(app).error = Some(failed());
            }
            read_rules(app);
        }
        "custody.rules.view_delete" => {
            if r.error.is_some() {
                st(app).error = Some(failed());
            }
            read_views(app);
        }
        _ => {}
    }
}

/// Escape closes the innermost: a select, the dialog, then the sheet.
pub fn escape(app: &mut App) -> bool {
    let s = st(app);
    if s.select.take().is_some() {
        return true;
    }
    if s.create_open {
        close_create(app);
        return true;
    }
    if s.open {
        close(app);
        return true;
    }
    false
}

// ---- Actions -----------------------------------------------------------------------------------

fn mutate(app: &mut App, kind: &str, mut args: Value, tag: &'static str) {
    st(app).error = None;
    args["expected_scope"] = app.expected_scope();
    app.send(kind, args, tag);
}

pub fn toggle_rule(app: &mut App, id: &str, enabled: bool) {
    mutate(
        app,
        "ui.transfers.rules.set_enabled",
        json!({"rule_id": id, "enabled": !enabled}),
        "custody.rules.set_enabled",
    );
}

pub fn ask_delete(app: &mut App, id: Option<String>) {
    st(app).confirm_delete = id;
}

pub fn delete_rule(app: &mut App, id: &str) {
    st(app).confirm_delete = None;
    mutate(
        app,
        "ui.transfers.rules.delete",
        json!({"rule_id": id}),
        "custody.rules.delete",
    );
}

pub fn delete_view(app: &mut App, id: &str) {
    mutate(
        app,
        "ui.saved_views.delete",
        json!({"view_id": id}),
        "custody.rules.view_delete",
    );
}

pub fn open_view(app: &mut App, id: &str) {
    super::page::open_saved_view(app, id);
}

pub fn open_create(app: &mut App) {
    st(app).create_open = true;
}

/// Closing the dialog resets it (`reset(); onClose()`).
pub fn close_create(app: &mut App) {
    let s = st(app);
    s.create_open = false;
    s.form = Form::default();
    s.select = None;
}

/// The rule the form describes (`submit`): only the fields set, the minimum confidence always.
pub fn payload(f: &Form) -> Value {
    let mut predicate = serde_json::Map::new();
    for (k, v) in [
        ("out_asset", &f.out_asset),
        ("in_asset", &f.in_asset),
        ("out_wallet_kind", &f.out_kind),
        ("in_wallet_kind", &f.in_kind),
    ] {
        if v != "any" {
            predicate.insert(k.into(), json!(v));
        }
    }
    if let Ok(Some(n)) = max_fee(f) {
        predicate.insert("max_fee_pct".into(), json!(n));
    }
    predicate.insert("min_confidence".into(), json!(f.min_confidence));
    let name = f.name.text.trim();
    json!({
        "name": if name.is_empty() { Value::Null } else { json!(name) },
        "predicate": Value::Object(predicate),
        "kind": f.kind,
        "policy": f.policy,
        "enabled": true,
    })
}

/// The fee cap as typed: None when blank, the whole field as a non-negative number (a decimal
/// comma counts, as German input writes it), Err when anything else is in it. A prefix is never
/// taken: `0,01` is 0.01, not 0, and `1.5x` is refused rather than saved as 1.5.
pub fn max_fee(f: &Form) -> Result<Option<f64>, ()> {
    let v = f.max_fee.text.trim();
    if v.is_empty() {
        return Ok(None);
    }
    let v = v.trim_end_matches('%').trim().replace(',', ".");
    match v.parse::<f64>() {
        Ok(n) if n.is_finite() && n >= 0.0 && parse_float(&v) == Some(n) => Ok(Some(n)),
        _ => Err(()),
    }
}

/// `Number.parseFloat`: the longest leading decimal number, None when there is none.
fn parse_float(s: &str) -> Option<f64> {
    let b = s.as_bytes();
    let mut end = 0;
    let mut seen_dot = false;
    let mut digits = false;
    if end < b.len() && (b[end] == b'+' || b[end] == b'-') {
        end += 1;
    }
    while end < b.len() {
        match b[end] {
            b'0'..=b'9' => digits = true,
            b'.' if !seen_dot => seen_dot = true,
            _ => break,
        }
        end += 1;
    }
    if !digits {
        return None;
    }
    s[..end].parse::<f64>().ok().filter(|v| v.is_finite())
}

pub fn create(app: &mut App) {
    if app.custody.rules.creating {
        return;
    }
    if max_fee(&app.custody.rules.form).is_err() {
        st(app).error = Some(t("review:swap.createRule.maxFeeInvalid"));
        return;
    }
    let p = payload(&app.custody.rules.form);
    st(app).creating = true;
    mutate(app, "ui.transfers.rules.create", p, "custody.rules.create");
}

pub fn set_select(app: &mut App, key: Option<String>) {
    st(app).select = key;
}

// ---- rulesModel.ts -----------------------------------------------------------------------------

/// A rule as one sentence: what it matches, then what the link records (`ruleSentence`).
pub fn rule_sentence(rule: &Value) -> String {
    let p = &rule["predicate"];
    let side = |asset: &Value, kind: &Value| {
        let mut parts = vec![];
        if let Some(k) = kind.as_str().filter(|k| !k.is_empty()) {
            let key = format!("review:swap.createRule.walletKind.{k}");
            parts.push(if crate::i18n::has(&key) {
                t(&key)
            } else {
                k.to_owned()
            });
        }
        match asset.as_str() {
            Some("LBTC") => parts.push(t("review:swap.rules.sentence.liquid")),
            Some("BTC") => parts.push(t("review:swap.rules.sentence.bitcoin")),
            _ => {}
        }
        if parts.is_empty() {
            t("review:swap.rules.sentence.anyWallet")
        } else {
            parts.join(" ")
        }
    };
    let mut matches = vec![format!(
        "{} → {}",
        side(&p["out_asset"], &p["out_wallet_kind"]),
        side(&p["in_asset"], &p["in_wallet_kind"])
    )];
    matches.push(t(if p["min_confidence"] == "exact" {
        "review:swap.rules.sentence.exactOnly"
    } else {
        "review:swap.rules.sentence.likelyOrExact"
    }));
    if let Some(pct) = p["max_fee_pct"].as_f64() {
        matches.push(tr(
            "review:swap.rules.sentence.maxFee",
            &[("pct", &js_number(pct))],
        ));
    }
    tr(
        "review:swap.rules.sentence.full",
        &[
            ("matches", &matches.join(", ")),
            (
                "kind",
                &t(&format!(
                    "review:transfersPage.kind.{}",
                    super::link_model::kind_key(rule["kind"].as_str())
                )),
            ),
            (
                "policy",
                &t(&format!(
                    "review:transfersPage.policyShort.{}",
                    if rule["policy"] == "taxable" {
                        "taxable"
                    } else {
                        "carrying"
                    }
                )),
            ),
        ],
    )
}

/// A number as JavaScript writes it in a template (`0.01`, `2`).
fn js_number(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

pub const SCENES: &[crate::app::Scene] = view::SCENES;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentence_reads_like_the_web() {
        crate::i18n::set_lang(crate::i18n::Lang::En);
        let rule = json!({"predicate": {"out_asset": "LBTC", "in_asset": "BTC", "min_confidence": "exact"}, "kind": "peg-out", "policy": "carrying-value"});
        assert_eq!(
            rule_sentence(&rule),
            "Liquid → Bitcoin, exact matches only: record as Peg-out from Liquid, tax-neutral"
        );
        let rule = json!({"predicate": {"out_wallet_kind": "phoenix", "max_fee_pct": 0.5}, "kind": "submarine-swap", "policy": "taxable"});
        assert_eq!(
            rule_sentence(&rule),
            "Phoenix → any wallet, likely or exact matches, fee up to 0.5 %: record as Swap to Lightning, taxable"
        );
    }

    #[test]
    fn payload_keeps_only_what_is_set() {
        let mut f = Form {
            max_fee: TextEdit::new(" 1.5 "),
            out_kind: "phoenix".into(),
            ..Form::default()
        };
        let p = payload(&f);
        assert_eq!(p["name"], Value::Null);
        assert_eq!(
            p["predicate"],
            json!({"out_wallet_kind": "phoenix", "max_fee_pct": 1.5, "min_confidence": "strong"})
        );
        assert_eq!(p["kind"], "submarine-swap");
        f.max_fee = TextEdit::new("abc");
        assert!(payload(&f)["predicate"].get("max_fee_pct").is_none());
    }

    /// German input: a decimal comma is the number, never its integer prefix; junk is refused.
    #[test]
    fn the_fee_cap_reads_the_whole_field() {
        let fee = |s: &str| {
            max_fee(&Form {
                max_fee: TextEdit::new(s),
                ..Form::default()
            })
        };
        assert_eq!(fee("0,01"), Ok(Some(0.01)));
        assert_eq!(fee(" 0.5 % "), Ok(Some(0.5)));
        assert_eq!(fee(""), Ok(None));
        assert_eq!(fee("1.5x"), Err(()));
        assert_eq!(fee("abc"), Err(()));
        assert_eq!(fee("-1"), Err(()));
    }
}
