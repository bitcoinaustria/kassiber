//! The editable draft of a transaction (`TransactionDetailDraft`, components/transactions/model.ts
//! `draftForTransaction`, `diffDraft`, `metadataUpdateArgs`) and the record helpers the tabs use.

use serde_json::{Value, json};

use crate::i18n::t;

// ---- Record access (the transaction row JSON as list/resolve return it) --------------------

pub fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}
pub fn opt_s(v: &Value, k: &str) -> Option<String> {
    v.get(k)
        .and_then(Value::as_str)
        .filter(|x| !x.is_empty())
        .map(str::to_owned)
}
pub fn f(v: &Value, k: &str) -> Option<f64> {
    v.get(k).and_then(Value::as_f64).filter(|x| x.is_finite())
}
pub fn b(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(false)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    Incoming,
    Outgoing,
    Transfer,
    Swap,
}

impl Flow {
    pub fn label(self) -> String {
        t(match self {
            Flow::Incoming => "transactions:flow.incoming",
            Flow::Outgoing => "transactions:flow.outgoing",
            Flow::Transfer => "transactions:flow.transfer",
            Flow::Swap => "transactions:flow.swap",
        })
    }
}

/// `transactionFlow(txn)`.
pub fn flow(r: &Value) -> Flow {
    let ty = s(r, "type");
    if s(r, "tag").to_lowercase().contains("swap") || matches!(ty, "Swap" | "Mint" | "Melt") {
        return Flow::Swap;
    }
    if b(r, "internal") || matches!(ty, "Transfer" | "Consolidation" | "Rebalance") {
        return Flow::Transfer;
    }
    if f(r, "amountSat").unwrap_or(0.0) >= 0.0 {
        Flow::Incoming
    } else {
        Flow::Outgoing
    }
}

/// `txn.direction`: Receive / Send / Transfer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Receive,
    Send,
    Transfer,
}

pub fn direction(r: &Value) -> Direction {
    match flow(r) {
        Flow::Transfer => Direction::Transfer,
        Flow::Incoming => Direction::Receive,
        Flow::Outgoing | Flow::Swap => {
            if f(r, "amountSat").unwrap_or(0.0) >= 0.0 {
                Direction::Receive
            } else {
                Direction::Send
            }
        }
    }
}

/// |amount| in BTC (`amountBtc`).
pub fn amount_btc(r: &Value) -> f64 {
    let sat = f(r, "amountSat").unwrap_or(0.0);
    let sat = if s(r, "type") == "Consolidation" && sat == 0.0 {
        -f(r, "feeSat").unwrap_or(0.0).abs()
    } else {
        sat
    };
    (sat / 1e8).abs()
}

/// `formatBtcAmount`: always 8 decimals and the asset code.
pub fn btc_amount(v: f64) -> String {
    format!("{v:.8} BTC")
}

/// `formatShortTxid`: ids over 18 characters become `first10...last6`.
pub fn short_txid(id: &str) -> String {
    if id.chars().count() > 18 {
        let head: String = id.chars().take(10).collect();
        let tail: String = id
            .chars()
            .rev()
            .take(6)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("{head}...{tail}")
    } else {
        id.to_owned()
    }
}

/// The best on-chain id of the row.
pub fn txid(r: &Value) -> String {
    opt_s(r, "explorerId")
        .or_else(|| opt_s(r, "externalId"))
        .unwrap_or_else(|| s(r, "id").to_owned())
}

pub fn status_key(status: &str) -> &'static str {
    match status {
        "completed" => "transactions:status.completed",
        "failed" => "transactions:status.failed",
        "review" => "transactions:status.review",
        _ => "transactions:status.pending",
    }
}

// ---- Codes and options ----------------------------------------------------------------------

/// Stable classification codes (stored as tags) and their translation keys.
pub const LABELS: [(&str, &str); 10] = [
    ("Unlabeled", "transactions:classification.unlabeled"),
    ("Income", "transactions:classification.income"),
    ("Expense", "transactions:classification.expense"),
    ("Transfer", "transactions:classification.transfer"),
    ("Swap", "transactions:classification.swap"),
    ("Fee", "transactions:classification.fee"),
    (
        "Merchant payment",
        "transactions:classification.merchantPayment",
    ),
    ("Gift", "transactions:classification.gift"),
    ("Review", "transactions:classification.review"),
    ("Other", "transactions:classification.other"),
];

pub fn label_text(code: &str) -> String {
    LABELS
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, k)| t(k))
        .unwrap_or_else(|| code.to_owned())
}

pub const REVIEW_STATUSES: [&str; 4] = ["completed", "pending", "failed", "review"];

/// `TAG_SUGGESTIONS` (model.ts:355-369): code → key (BTCPay/Liquid/Lightning are names).
pub const TAG_SUGGESTIONS: [(&str, Option<&str>); 13] = [
    ("Revenue", Some("transactions:tagSuggestion.revenue")),
    ("BTCPay", None),
    (
        "needs invoice",
        Some("transactions:tagSuggestion.needsInvoice"),
    ),
    ("client ACME", Some("transactions:tagSuggestion.clientAcme")),
    ("Hosting", Some("transactions:tagSuggestion.hosting")),
    ("Capex", Some("transactions:tagSuggestion.capex")),
    ("Meals", Some("transactions:tagSuggestion.meals")),
    ("Bank fees", Some("transactions:tagSuggestion.bankFees")),
    (
        "Consolidation",
        Some("transactions:tagSuggestion.consolidation"),
    ),
    ("Liquid", None),
    ("Lightning", None),
    (
        "manual review",
        Some("transactions:tagSuggestion.manualReview"),
    ),
    ("accountant", Some("transactions:tagSuggestion.accountant")),
];

pub struct TaxOption {
    pub regime: &'static str,
    pub category: &'static str,
    pub taxable: bool,
    pub key: &'static str,
}

/// `AT_TAX_OPTIONS` (model.ts:441-506).
pub const TAX_OPTIONS: [TaxOption; 7] = [
    TaxOption {
        regime: "neu",
        category: "neu_gain",
        taxable: true,
        key: "neuGain",
    },
    TaxOption {
        regime: "neu",
        category: "income_general",
        taxable: true,
        key: "incomeGeneral",
    },
    TaxOption {
        regime: "neu",
        category: "neu_loss",
        taxable: true,
        key: "neuLoss",
    },
    TaxOption {
        regime: "outside",
        category: "none",
        taxable: false,
        key: "ownWalletTransfer",
    },
    TaxOption {
        regime: "neu",
        category: "neu_swap",
        taxable: false,
        key: "neuSwap",
    },
    TaxOption {
        regime: "alt",
        category: "alt_spekulation",
        taxable: true,
        key: "altSpekulation",
    },
    TaxOption {
        regime: "alt",
        category: "alt_taxfree",
        taxable: false,
        key: "altTaxfree",
    },
];

pub fn tax_option(regime: &str, category: &str) -> &'static TaxOption {
    TAX_OPTIONS
        .iter()
        .find(|o| o.regime == regime && o.category == category)
        .unwrap_or(&TAX_OPTIONS[0])
}

pub fn tax_label(o: &TaxOption) -> String {
    t(&format!("transactions:taxClassification.{}.label", o.key))
}

pub fn tax_short(o: &TaxOption) -> String {
    t(&format!("transactions:taxClassification.{}.short", o.key))
}

/// Outbound `kind` values for the Send "Recorded as" select.
pub const OUTBOUND_KINDS: [&str; 7] = [
    "sell",
    "withdrawal",
    "spend",
    "gift",
    "donation",
    "lost",
    "stolen",
];
/// Inbound kinds for the acquisition review.
pub const INBOUND_KINDS: [&str; 10] = [
    "buy",
    "deposit",
    "income",
    "wages",
    "mining",
    "staking",
    "interest",
    "lending_interest",
    "airdrop",
    "hardfork",
];

pub fn kind_label(kind: &str) -> String {
    let key = match kind {
        "hardfork" => "hardFork",
        "lending_interest" => "interest",
        other => other,
    };
    let base = t(&format!("transactions:type.{key}"));
    if kind == "lending_interest" {
        format!("{base} (lending_interest)")
    } else {
        base
    }
}

/// The pricing source cards: value, label key, the kind and quality they set.
pub const PRICING_CHOICES: [(&str, &str, Option<&str>, &str); 4] = [
    (
        "generic_import",
        "sourcePrice",
        Some("generic_import"),
        "exact",
    ),
    (
        "fmv_provider",
        "fmvProvider",
        Some("fmv_provider"),
        "provider_sample",
    ),
    (
        "manual_override",
        "manualOverride",
        Some("manual_override"),
        "exact",
    ),
    ("missing", "missing", None, "missing"),
];

/// `pricingChoiceValue(kind, quality)`.
pub fn pricing_choice(kind: Option<&str>, quality: &str) -> &'static str {
    match kind {
        _ if quality == "missing" => "missing",
        None => "missing",
        Some("manual_override") => "manual_override",
        Some("fmv_provider") | Some("manual_rate_cache") => "fmv_provider",
        Some(_) => "generic_import",
    }
}

pub fn quality_label(q: &str) -> String {
    t(match q {
        "coarse_fallback" => "transactions:pricingQuality.coarseFallback",
        "provider_sample" => "transactions:pricingQuality.providerSample",
        "exact" => "transactions:pricingQuality.exact",
        _ => "transactions:pricingQuality.missing",
    })
}

pub fn source_kind_label(kind: &str) -> String {
    let key = format!("transactions:pricingSourceKind.{kind}.label");
    if crate::i18n::has(&key) {
        t(&key)
    } else {
        kind.replace('_', " ")
    }
}

pub fn source_kind_phrase(kind: &str) -> String {
    let key = format!("transactions:pricingSourceKind.{kind}.phrase");
    if crate::i18n::has(&key) {
        t(&key)
    } else {
        source_kind_label(kind)
    }
}

/// `pricingProviderLabel`.
pub fn provider_label(p: &str) -> String {
    match p {
        "" => t("transactions:pricing.providerUnknown"),
        "kraken-csv" => "Kraken CSV".into(),
        "coinbase-exchange" => "Coinbase Exchange".into(),
        "coingecko" => "CoinGecko".into(),
        "manual" => t("transactions:pricingProvider.manual"),
        other => other
            .split(['-', '_', ' '])
            .filter(|w| !w.is_empty())
            .map(|w| {
                let mut c = w.chars();
                c.next()
                    .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

// ---- Draft -----------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    pub label: String,
    pub tags: Vec<String>,
    pub note: String,
    pub at_regime: String,
    pub at_category: String,
    pub pricing_source_kind: Option<String>,
    pub pricing_quality: String,
    pub manual_currency: String,
    pub manual_price: String,
    pub manual_value: String,
    pub manual_source: String,
    pub review_status: String,
    pub taxable: bool,
    pub excluded: bool,
    pub kind: Option<String>,
}

/// `draftForTransaction`.
pub fn baseline(r: &Value) -> Draft {
    // `txn.tags ?? splitDraftTags(txn.tag)`: an empty `tags` array is the answer (`tag` is
    // only the display fallback when the field is absent).
    let mut tags: Vec<String> = match r.get("tags").and_then(Value::as_array) {
        Some(a) => a
            .iter()
            .filter_map(|x| x.as_str().map(str::to_owned))
            .collect(),
        None => s(r, "tag")
            .split(',')
            .map(|x| x.trim().to_owned())
            .filter(|x| !x.is_empty())
            .collect(),
    };
    let label = tags
        .iter()
        .find_map(|tag| {
            LABELS
                .iter()
                .find(|(c, _)| c.eq_ignore_ascii_case(tag))
                .map(|(c, _)| c.to_string())
        })
        .unwrap_or_else(|| "Unlabeled".into());
    tags.retain(|x| !x.eq_ignore_ascii_case(&label));
    let mut seen: Vec<String> = Vec::new();
    tags.retain(|x| {
        let l = x.to_lowercase();
        if seen.contains(&l) {
            false
        } else {
            seen.push(l);
            true
        }
    });
    let (reg, cat, tx) = if s(r, "type") == "Fee" {
        ("outside", "none", false)
    } else {
        match flow(r) {
            Flow::Incoming => ("neu", "income_general", true),
            Flow::Transfer => ("outside", "none", false),
            Flow::Swap => ("neu", "neu_swap", false),
            Flow::Outgoing => ("neu", "neu_gain", true),
        }
    };
    let rate = f(r, "rate").filter(|x| *x != 0.0);
    let amount = f(r, "eur").filter(|x| *x != 0.0);
    Draft {
        label,
        tags,
        note: s(r, "note").to_owned(),
        at_regime: opt_s(r, "atRegime").unwrap_or_else(|| reg.into()),
        at_category: opt_s(r, "atCategory").unwrap_or_else(|| cat.into()),
        pricing_source_kind: opt_s(r, "pricingSourceKind")
            .or_else(|| rate.map(|_| "generic_import".into())),
        pricing_quality: opt_s(r, "pricingQuality").unwrap_or_else(|| {
            if rate.is_some() {
                "exact".into()
            } else {
                "missing".into()
            }
        }),
        manual_currency: opt_s(r, "fiatCurrency").unwrap_or_else(|| "EUR".into()),
        manual_price: rate.map(js_num).unwrap_or_default(),
        manual_value: amount.map(js_num).unwrap_or_default(),
        manual_source: opt_s(r, "pricingExternalRef").unwrap_or_default(),
        review_status: opt_s(r, "reviewStatus")
            .or_else(|| opt_s(r, "status"))
            .unwrap_or_else(|| s(r, "queueStatus").to_owned()),
        taxable: r.get("taxable").and_then(Value::as_bool).unwrap_or(tx),
        excluded: b(r, "excluded"),
        kind: opt_s(r, "kindOverride"),
    }
}

/// `String(number)` as JavaScript prints it.
pub fn js_num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Dirty {
    pub label: bool,
    pub tags: bool,
    pub note: bool,
    pub at: bool,
    pub pricing: bool,
    pub review_status: bool,
    pub taxable: bool,
    pub excluded: bool,
    pub kind: bool,
    pub count: usize,
}

/// `diffDraft` + `countDirty`: 15 fields, tags as ordered arrays.
pub fn diff(a: &Draft, b: &Draft) -> Dirty {
    let fields = [
        a.label != b.label,
        a.tags != b.tags,
        a.note != b.note,
        a.at_regime != b.at_regime,
        a.at_category != b.at_category,
        a.pricing_source_kind != b.pricing_source_kind,
        a.pricing_quality != b.pricing_quality,
        a.manual_currency != b.manual_currency,
        a.manual_price != b.manual_price,
        a.manual_value != b.manual_value,
        a.manual_source != b.manual_source,
        a.review_status != b.review_status,
        a.taxable != b.taxable,
        a.excluded != b.excluded,
        a.kind != b.kind,
    ];
    Dirty {
        label: fields[0],
        tags: fields[1],
        note: fields[2],
        at: fields[3] || fields[4],
        pricing: fields[5..11].iter().any(|x| *x),
        review_status: fields[11],
        taxable: fields[12],
        excluded: fields[13],
        kind: fields[14],
        count: fields.iter().filter(|x| **x).count(),
    }
}

/// A fresh read merged into a draft: each field still at the old baseline takes the fresh value
/// (a save's normalized form, a change made elsewhere); a field the user edited keeps the edit.
pub fn rebase(old: &Draft, fresh: &Draft, draft: &Draft) -> Draft {
    let mut out = draft.clone();
    macro_rules! take {
        ($($f:ident),*) => {$(
            if draft.$f == old.$f {
                out.$f = fresh.$f.clone();
            }
        )*};
    }
    take!(
        label,
        tags,
        note,
        at_regime,
        at_category,
        pricing_source_kind,
        pricing_quality,
        manual_currency,
        manual_price,
        manual_value,
        manual_source,
        review_status,
        taxable,
        excluded,
        kind
    );
    out
}

/// `parseManualDecimal`.
pub fn parse_decimal(s: &str) -> Option<f64> {
    let t = s.trim().replacen(',', ".", 1);
    if t.is_empty() {
        return None;
    }
    t.parse::<f64>().ok().filter(|x| x.is_finite())
}

/// A manual decimal as the daemon's decimal parser reads it: trimmed, the one decimal comma
/// [`parse_decimal`] accepts turned into a point, text kept as typed (no float round trip).
/// None when it isn't a number.
pub fn decimal_text(s: &str) -> Option<String> {
    parse_decimal(s).map(|_| s.trim().replacen(',', ".", 1))
}

/// `metadataUpdateArgs(id, draft, baseline, sourceTags)`.
pub fn save_args(id: &str, d: &Draft, base: &Draft, source_tags: &[String], scope: Value) -> Value {
    let tags_of = |d: &Draft| {
        let persist_label = d.label != "Unlabeled"
            && !d.label.is_empty()
            && (source_tags.iter().any(|x| x.eq_ignore_ascii_case(&d.label))
                || d.label != base.label);
        let mut tags: Vec<String> = Vec::new();
        for tag in std::iter::once(if persist_label {
            d.label.clone()
        } else {
            String::new()
        })
        .chain(d.tags.iter().cloned())
        {
            if !tag.is_empty() && !tags.contains(&tag) {
                tags.push(tag);
            }
        }
        tags
    };
    // Only what this panel changed: an older tab's unchanged note, tags or exclusion must not
    // overwrite what another tab (or the table) saved since. The core keeps omitted fields.
    let mut args = json!({
        "transaction": id,
        "expected_scope": scope,
    });
    if d.note != base.note {
        args["note"] = if d.note.trim().is_empty() {
            Value::Null
        } else {
            Value::String(d.note.clone())
        };
    }
    let tags = tags_of(d);
    if tags != tags_of(base) {
        args["tags"] = json!(tags);
    }
    if d.excluded != base.excluded {
        args["excluded"] = json!(d.excluded);
    }
    if d.review_status != base.review_status {
        args["review_status"] = json!(d.review_status);
    }
    if d.taxable != base.taxable
        || d.at_regime != base.at_regime
        || d.at_category != base.at_category
    {
        args["taxable"] = json!(d.taxable);
        args["at_regime"] = json!(d.at_regime);
        args["at_category"] = json!(d.at_category);
    }
    if d.kind != base.kind {
        args["kind"] = json!(d.kind);
    }
    let pricing_changed = d.pricing_source_kind != base.pricing_source_kind
        || d.pricing_quality != base.pricing_quality
        || d.manual_currency != base.manual_currency
        || d.manual_price != base.manual_price
        || d.manual_value != base.manual_value
        || d.manual_source != base.manual_source;
    if pricing_changed {
        args["pricing_source_kind"] = json!(d.pricing_source_kind);
        args["pricing_quality"] = json!(d.pricing_quality);
        args["fiat_currency"] = json!(d.manual_currency.trim().to_uppercase());
        args["fiat_rate"] = decimal_text(&d.manual_price).map_or(Value::Null, |v| json!(v));
        args["fiat_value"] = decimal_text(&d.manual_value).map_or(Value::Null, |v| json!(v));
        args["pricing_external_ref"] = if d.manual_source.trim().is_empty() {
            Value::Null
        } else {
            json!(d.manual_source.trim())
        };
    }
    args
}

/// `isPricingMissing`.
pub fn pricing_missing(d: &Draft, r: &Value) -> bool {
    d.pricing_source_kind.is_none() || d.pricing_quality == "missing" || f(r, "eur").is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> Value {
        json!({"id": "t1", "type": "Deposit", "amountSat": 534123, "eur": 472.27, "rate": 88421.2, "fiatCurrency": "EUR",
            "pricingSourceKind": "fmv_provider", "pricingQuality": "coarse_fallback", "tag": "Deposit", "tags": [], "note": "",
            "queueStatus": "pending", "reviewStatus": null, "excluded": false})
    }

    #[test]
    fn baseline_and_save_args_follow_the_web() {
        let r = row();
        let base = baseline(&r);
        assert_eq!(base.label, "Unlabeled");
        assert!(base.tags.is_empty(), "an empty tags array stays empty");
        assert_eq!(
            (
                base.at_regime.as_str(),
                base.at_category.as_str(),
                base.taxable
            ),
            ("neu", "income_general", true)
        );
        assert_eq!(base.manual_price, "88421.2");
        let mut d = base.clone();
        d.note = "hello".into();
        d.label = "Income".into();
        let dirt = diff(&d, &base);
        assert_eq!(dirt.count, 2);
        let a = save_args("t1", &d, &base, &[], Value::Null);
        assert_eq!(a["tags"], json!(["Income"]));
        assert_eq!(a["note"], json!("hello"));
        assert!(a.get("pricing_source_kind").is_none() && a.get("review_status").is_none());
        d.pricing_source_kind = Some("manual_override".into());
        d.manual_price = "70000,5".into();
        let a = save_args("t1", &d, &base, &[], Value::Null);
        // The daemon's decimal parser takes a point; the text is otherwise sent as typed.
        assert_eq!(a["fiat_rate"], json!("70000.5"));
        d.manual_value = " 1234,56789012345678901 ".into();
        let a = save_args("t1", &d, &base, &[], Value::Null);
        assert_eq!(a["fiat_value"], json!("1234.56789012345678901"));
        assert_eq!(a["pricing_external_ref"], Value::Null);
    }

    /// An older tab that only changed a tag sends only the tags: its stale note and exclusion
    /// do not overwrite what was saved elsewhere since.
    #[test]
    fn a_save_sends_only_what_changed() {
        let base = baseline(&row());
        let mut d = base.clone();
        d.tags = vec!["cold".into()];
        let a = save_args("t1", &d, &base, &[], Value::Null);
        assert_eq!(a["tags"], json!(["cold"]));
        assert!(a.get("note").is_none() && a.get("excluded").is_none());
        d.excluded = true;
        let a = save_args("t1", &d, &base, &[], Value::Null);
        assert_eq!(a["excluded"], json!(true));
    }

    #[test]
    fn pricing_choice_maps_kinds() {
        assert_eq!(
            pricing_choice(Some("exchange_execution"), "exact"),
            "generic_import"
        );
        assert_eq!(
            pricing_choice(Some("manual_rate_cache"), "exact"),
            "fmv_provider"
        );
        assert_eq!(pricing_choice(Some("fmv_provider"), "missing"), "missing");
        assert_eq!(
            short_txid("373b15e0a627488e80c3ad301d0fe61efdcfd3a2d8cf387b5577bb6faf3ea6cb"),
            "373b15e0a6...3ea6cb"
        );
    }
}
