//! Disclosure projections shared by future source-funds report callers.
//! Inputs are local rows; this module performs no I/O or accounting decisions.

use super::*;
use kassiber_pytext::{lower, strip, upper};
use std::collections::BTreeSet;

fn fallback(row: &Value, key: &str) -> Value {
    get(row, key)
        .filter(|v| truthy(v))
        .cloned()
        .unwrap_or_else(|| s(""))
}
fn normalized(v: &Value) -> String {
    lower(strip(&py_str(v)))
}
fn key_value(key: &kassiber_pyjson::Key) -> Value {
    use kassiber_pyjson::Key;
    match key {
        Key::Null => Value::Null,
        Key::Bool(b) => Value::Bool(*b),
        Key::Int(n) => Value::Int(n.clone()),
        Key::Float(n) => Value::Float(*n),
        Key::Str(s) => Value::Str(s.clone()),
    }
}
fn count(v: &Value) -> Result<Value> {
    let raw =
        super::source_funds_reads::integer(v).map_err(|e| AppError::new("internal_error", e))?;
    Ok(Value::Int(
        kassiber_pytext::parse_int(&raw).expect("validated integer"),
    ))
}

pub fn reveal_mode(v: &Value) -> Result<String> {
    let raw = if truthy(v) { v.clone() } else { s("standard") };
    let mode = normalized(&raw);
    if !["labels_only", "minimal", "standard", "full"].contains(&mode.as_str()) {
        return Err(AppError::new(
            "validation",
            format!("Unsupported reveal mode '{}'", py_str(v)),
        )
        .with_hint("Choose one of: labels_only, minimal, standard, full"));
    }
    Ok(mode)
}

pub fn report_purpose(v: &Value) -> Result<String> {
    let raw = if truthy(v) {
        v.clone()
    } else {
        s("existing_transaction")
    };
    let purpose = normalized(&raw).replace('-', "_");
    if !["existing_transaction", "planned_exchange_sale"].contains(&purpose.as_str()) {
        return Err(AppError::new(
            "validation",
            format!("Unsupported source-funds report purpose '{}'", py_str(v)),
        )
        .with_hint("Choose one of: existing_transaction, planned_exchange_sale"));
    }
    Ok(purpose)
}

pub fn report_options(options: &Value) -> Result<Value> {
    let empty = dict::<&str>([]);
    let options = if truthy(options) { options } else { &empty };
    if !matches!(options, Value::Dict(_)) {
        return Err(AppError::new(
            "internal_error",
            format!(
                "'{}' object has no attribute 'get'",
                kassiber_envelope::type_name(options)
            ),
        ));
    }
    let normalize = |key: &str, default: &str, accepted: &[&str]| {
        let input = fallback(options, key);
        let candidate = if truthy(&input) {
            normalized(&input)
        } else {
            default.into()
        };
        if accepted.contains(&candidate.as_str()) {
            candidate
        } else {
            default.into()
        }
    };
    let detail = normalize("diagram_detail", "summary", &["summary", "detailed"]);
    let precision = normalize("amount_precision", "btc", &["btc", "sats"]);
    let omit = value(options, "omit_sections");
    let mut omitted = BTreeSet::new();
    let items = match omit {
        Value::Str(_) => vec![omit.clone()],
        Value::List(items) => items.clone(),
        Value::Dict(fields) => fields.iter().map(|(key, _)| key_value(key)).collect(),
        _ if !truthy(omit) => vec![],
        _ => {
            return Err(AppError::new(
                "internal_error",
                format!(
                    "'{}' object is not iterable",
                    kassiber_envelope::type_name(omit)
                ),
            ))
        }
    };
    for item in items {
        let candidate = normalized(&item);
        if [
            "flow_levels",
            "transaction_details",
            "flow_links",
            "graph_nodes",
        ]
        .contains(&candidate.as_str())
        {
            omitted.insert(candidate);
        }
    }
    let mut overrides = dict::<String>([]);
    if let Value::Dict(fields) = value(options, "reveal_overrides") {
        for (key, decision) in fields {
            let key = py_str(&key_value(key));
            let decision = normalized(decision);
            if !key.is_empty() && ["show", "hide"].contains(&decision.as_str()) {
                replace(&mut overrides, &key, s(decision));
            }
        }
    }
    Ok(dict([
        ("diagram_detail", s(detail)),
        ("amount_precision", s(precision)),
        (
            "mask_recipient",
            Value::Bool(truthy(value(options, "mask_recipient"))),
        ),
        (
            "omit_sections",
            Value::List(omitted.into_iter().map(s).collect()),
        ),
        ("reveal_overrides", overrides),
    ]))
}

pub fn report_context(profile: &Value) -> Value {
    let country = get(profile, "tax_country")
        .filter(|v| truthy(v))
        .map(|v| lower(&py_str(v)))
        .unwrap_or_else(|| kassiber_tax_countries::DEFAULT_CODE.into());
    let currency = upper(&py_str(&fallback(profile, "fiat_currency")));
    let entry = kassiber_tax_countries::lookup(&country);
    let at_eur = entry.is_some_and(|entry| entry.rules == crate::country::Rules::Austria)
        && currency == "EUR";
    let mut checklist = vec![
        s("Original fiat-to-bitcoin purchase or income evidence is attached to the root source."),
        s("Reviewed wallet-transfer and consolidation hops cover the disclosed target amount."),
        s("The target exchange, broker, or bank deposit is the selected report target."),
        s("The PDF is rendered from a saved immutable case snapshot."),
    ];
    if at_eur {
        checklist.push(s("EUR values and Austrian profile context are present for a basic Mittelherkunftsnachweis workflow."));
    }
    dict([
        ("tax_country", s(country.as_str())),
        ("fiat_currency", s(currency)),
        (
            "jurisdiction_label",
            s(entry
                .or_else(|| kassiber_tax_countries::lookup(kassiber_tax_countries::DEFAULT_CODE))
                .map_or("Generic", |entry| entry.name_en)),
        ),
        (
            "template_key",
            s(if at_eur {
                "at_eur_basic"
            } else {
                "generic_basic"
            }),
        ),
        (
            "report_title",
            s(if at_eur {
                "Mittelherkunftsnachweis / Source of Funds Report"
            } else {
                "Source of Funds Report"
            }),
        ),
        (
            "report_subtitle",
            s(if at_eur {
                "Reviewed local evidence disclosure for an Austrian/EUR source-of-funds workflow."
            } else {
                "Reviewed local evidence disclosure from a saved immutable case snapshot."
            }),
        ),
        ("evidence_checklist", Value::List(checklist)),
        (
            "deferred",
            Value::List(
                [
                    "Full country-specific legal templates",
                    "German localization",
                    "Locale-specific money/date formatting beyond the current ReportLab renderer",
                    "CoinJoin/PayJoin traversal",
                ]
                .map(s)
                .to_vec(),
            ),
        ),
    ])
}

pub fn public_transaction_id(
    row: &Value,
    mode: &str,
    is_target: bool,
    overrides: &Value,
) -> String {
    let decision = text(overrides, &text(row, "id"));
    if decision == "hide" {
        return String::new();
    }
    if decision != "show" && (mode == "labels_only" || mode == "minimal" && !is_target) {
        return String::new();
    }
    let external = fallback(row, "external_id");
    if truthy(&external) {
        py_str(&external)
    } else {
        text(row, "id")
    }
}

pub fn wallet_data_provenance(kind: &Value, config: &Value) -> String {
    let kind = normalized(if truthy(kind) { kind } else { &Value::Null });
    if ["descriptor", "xpub", "samourai", "address"].contains(&kind.as_str()) {
        let config = if truthy(config) {
            kassiber_pytext::loads(&py_str(config)).unwrap_or(Value::Null)
        } else {
            Value::Null
        };
        return if truthy(value(&config, "source_file")) || truthy(value(&config, "source_format")) { "platform_export" } else { "chain_sync" }.into();
    }
    if [
        "coreln",
        "lnd",
        "nwc",
        "phoenix",
        "river",
        "bullbitcoin",
        "coinfinity",
        "21bitcoin",
        "pocketbitcoin",
        "strike",
        "wasabi",
    ]
    .contains(&kind.as_str())
    {
        "platform_export"
    } else {
        "manual_import"
    }
    .into()
}

pub fn transaction_node(
    row: &Value,
    mode: &str,
    required_msat: &Value,
    is_target: bool,
    overrides: &Value,
) -> Result<Value> {
    let public_id = public_transaction_id(row, mode, is_target, overrides);
    let wallet = get(row, "wallet_label")
        .cloned()
        .unwrap_or_else(|| get(row, "wallet").cloned().unwrap_or_else(|| s("")));
    let label = if public_id.is_empty() {
        let amount = super::source_funds_reads::truncated_amount(value(row, "amount"))?;
        let Value::Float(amount) = amount else {
            return Err(AppError::new(
                "internal_error",
                "missing transaction amount",
            ));
        };
        format!(
            "{} {} {} {amount:.8}",
            py_str(&wallet),
            text(row, "direction"),
            text(row, "asset")
        )
    } else {
        public_id.clone()
    };
    // The node's fields evaluate in Python's order: the amount's dec() before
    // any int() of the amount or fee.
    let amount = super::source_funds_reads::amount(value(row, "amount"))?;
    let amount_msat = count(value(row, "amount"))?;
    let fee = if truthy(value(row, "fee")) {
        count(value(row, "fee"))?
    } else {
        Value::Int(0.into())
    };
    let free_text = matches!(mode, "standard" | "full");
    let mut node = dict([
        ("id", s(format!("tx:{}", text(row, "id")))),
        ("node_type", s("transaction")),
        ("transaction_id", value(row, "id").clone()),
        ("label", s(label)),
        ("wallet", wallet),
        ("occurred_at", value(row, "occurred_at").clone()),
        ("direction", value(row, "direction").clone()),
        ("asset", value(row, "asset").clone()),
        ("amount", amount),
        ("amount_msat", amount_msat),
        ("fee", super::source_funds_reads::amount(&fee)?),
        ("fee_msat", fee),
        (
            "required_amount",
            super::source_funds_reads::amount(required_msat)?,
        ),
        ("required_amount_msat", required_msat.clone()),
        ("external_id", s(public_id)),
        ("fiat_currency", fallback(row, "fiat_currency")),
        ("fiat_value", value(row, "fiat_value").clone()),
        (
            "pricing_source_kind",
            if truthy(value(row, "pricing_source_kind")) {
                value(row, "pricing_source_kind").clone()
            } else {
                fallback(row, "fiat_price_source")
            },
        ),
        (
            "data_provenance",
            s(wallet_data_provenance(
                value(row, "wallet_kind"),
                value(row, "wallet_config_json"),
            )),
        ),
        (
            "description",
            if free_text {
                fallback(row, "description")
            } else {
                s("")
            },
        ),
        (
            "counterparty",
            if free_text {
                fallback(row, "counterparty")
            } else {
                s("")
            },
        ),
    ]);
    if mode == "full" {
        replace(
            &mut node,
            "internal_transaction_id",
            value(row, "id").clone(),
        );
    }
    Ok(node)
}

pub fn source_node(row: &Value, mode: &str, required_msat: &Value) -> Result<Value> {
    let free_text = matches!(mode, "standard" | "full");
    Ok(dict([
        ("id", s(format!("source:{}", text(row, "id")))),
        ("node_type", s("source")),
        ("source_id", value(row, "id").clone()),
        ("source_type", value(row, "source_type").clone()),
        (
            "label",
            if mode == "labels_only" {
                s(text(row, "source_type").replace('_', " "))
            } else {
                value(row, "label").clone()
            },
        ),
        ("asset", value(row, "asset").clone()),
        (
            "amount",
            super::source_funds_reads::amount(value(row, "amount"))?,
        ),
        ("amount_msat", value(row, "amount").clone()),
        (
            "required_amount",
            super::source_funds_reads::amount(required_msat)?,
        ),
        ("required_amount_msat", required_msat.clone()),
        ("fiat_currency", fallback(row, "fiat_currency")),
        ("fiat_value", value(row, "fiat_value").clone()),
        ("acquired_at", fallback(row, "acquired_at")),
        (
            "description",
            if free_text {
                fallback(row, "description")
            } else {
                s("")
            },
        ),
        ("review_state", value(row, "review_state").clone()),
    ]))
}

pub fn attachment(row: &Value, mode: &str) -> Result<Value> {
    let mode = reveal_mode(&s(mode))?;
    let mut item = dict([
        ("id", value(row, "id").clone()),
        ("attachment_type", value(row, "attachment_type").clone()),
        (
            "label",
            s(super::source_funds_reads::attachment_label(row)?),
        ),
    ]);
    if matches!(mode.as_str(), "standard" | "full") {
        replace(
            &mut item,
            "transaction_id",
            value(row, "transaction_id").clone(),
        );
        replace(&mut item, "media_type", fallback(row, "media_type"));
        replace(&mut item, "sha256", fallback(row, "sha256"));
    }
    if mode == "full" {
        replace(&mut item, "source_url", fallback(row, "source_url"));
        replace(&mut item, "stored_relpath", fallback(row, "stored_relpath"));
    }
    Ok(item)
}

/// Hidden endpoints also redact machine-generated identifiers inside explanations.
pub fn edge_explanation(
    explanation: &Value,
    mode: &str,
    endpoint_ids: &[String],
    overrides: &Value,
) -> Value {
    if !truthy(explanation) || !matches!(mode, "standard" | "full") {
        return s("");
    }
    if endpoint_ids.iter().any(|id| text(overrides, id) == "hide") {
        return s("Reviewed link; details withheld for a hidden transaction.");
    }
    explanation.clone()
}
