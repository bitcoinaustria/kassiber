use super::*;

fn display(raw: &Py, normalized: &Py) -> Py {
    if *normalized != Py::Null {
        normalized.clone()
    } else if !missing(raw) {
        truncated(cleaned(raw), 128)
    } else {
        Py::Null
    }
}
fn fallback<'a>(v: &'a Py, keys: &[&str]) -> &'a Py {
    keys.iter()
        .map(|key| field(v, key))
        .find(|v| !missing(v))
        .unwrap_or(&Py::Null)
}
fn draft_row(
    raw: &Py,
    source_hash: &str,
    threshold: &Decimal,
    currency: &str,
    allowed_pages: Option<&[i64]>,
) -> Result<Py, AppError> {
    let date = cleaned(if truthy(field(raw, "occurred_at")) {
        field(raw, "occurred_at")
    } else {
        field(raw, "date")
    });
    let occurred = if date == Py::Null {
        None
    } else {
        crate::time::timestamp(&rendered(&date)).ok()
    };
    let side = direction(field(raw, "direction"));
    let mut asset = upper(truncated(cleaned(field(raw, "asset")), 32));
    let explicit = fallback(raw, &["amount_btc", "amount_crypto"]);
    let amount = if missing(explicit) && asset != Py::Null {
        field(raw, "amount")
    } else {
        explicit
    };
    let amount_text = decimal_text(amount);
    let amount_number = decimal(amount);
    if asset == Py::Null && !missing(explicit) {
        asset = s("BTC");
    }
    let fee = fallback(raw, &["fee_btc", "fee_crypto"]);
    let fee_text = decimal_text(fee);
    let fee_number = decimal(fee);
    let fee_display = display(fee, &fee_text);
    let fiat = truncated(upper(cleaned(field(raw, "fiat_currency"))), 16);
    let value = field(raw, "fiat_value");
    let rate = field(raw, "fiat_rate");
    let value_number = decimal(value);
    let rate_number = decimal(rate);
    let value_text = decimal_text(value);
    let rate_text = decimal_text(rate);
    let model_confidence = confidence(field(raw, "confidence"));
    let cells = cell_confidences(field(raw, "cell_confidences"));
    let counterparty = cleaned(field(raw, "counterparty"));
    let description = cleaned(field(raw, "description"));
    let mut flags: Vec<String> = vec![];
    if occurred.is_none() {
        flags.push("missing_date".into());
    }
    if date != Py::Null && occurred.is_none() {
        flags.push("invalid_date".into());
    }
    if side == Py::Null {
        flags.push("missing_direction".into());
    }
    if amount_text == Py::Null {
        flags.push("missing_amount".into());
    }
    if model_confidence < *threshold {
        flags.push("low_row_confidence".into());
    }
    for (key, present) in [
        ("occurred_at", date != Py::Null),
        ("direction", cleaned(field(raw, "direction")) != Py::Null),
        ("asset", asset != Py::Null),
        ("amount_btc", !missing(amount)),
        ("fee_btc", !missing(fee)),
        ("fiat_currency", fiat != Py::Null),
        ("fiat_value", !missing(value)),
        ("fiat_rate", !missing(rate)),
        ("counterparty", counterparty != Py::Null),
        ("description", description != Py::Null),
    ] {
        if present {
            match get(&cells, key) {
                None => flags.push(format!("missing_{key}_confidence")),
                Some(v) if decimal(v).unwrap() < *threshold => {
                    flags.push(format!("low_{key}_confidence"))
                }
                _ => (),
            }
        }
    }
    if asset != Py::Null && ![s("BTC"), s("LBTC")].contains(&asset) {
        flags.push("unsupported_asset".into());
    }
    if let Some(n) = &amount_number {
        if !n.is_positive() {
            flags.push("non_positive_amount".into());
        } else if !exact_msat(n) {
            flags.push("amount_not_representable".into());
        }
    }
    if !missing(fee) && fee_number.is_none() {
        flags.push("invalid_fee".into());
    } else if let Some(n) = &fee_number {
        if n.is_negative() {
            flags.push("negative_fee".into());
        } else if !exact_msat(n) {
            flags.push("fee_not_representable".into());
        }
    }
    for (key, raw, number) in [
        ("fiat_value", value, &value_number),
        ("fiat_rate", rate, &rate_number),
    ] {
        if !missing(raw) && number.is_none() {
            flags.push(format!("invalid_{key}"));
        } else if let Some(n) = number {
            if !n.is_positive() {
                flags.push(format!("non_positive_{key}"));
            } else if !storable_fiat(n) {
                flags.push(format!("{key}_out_of_range"));
            }
        }
    }
    if (!missing(value) || !missing(rate)) && fiat == Py::Null {
        flags.push("missing_fiat_currency".into());
    }
    let currency = kassiber_pytext::upper(kassiber_pytext::strip(currency));
    if fiat != Py::Null && !currency.is_empty() && fiat != s(&currency) {
        flags.push("fiat_currency_mismatch".into());
    }
    let region = source_region(field(raw, "source_region"), allowed_pages);
    if matches!(field(raw, "source_region"), Py::Dict(_)) && region == Py::Null {
        flags.push("invalid_source_page".into());
    }
    let evidence = truncated(cleaned(field(raw, "evidence_text")), 4000);
    let counterparty = truncated(counterparty, 500);
    let description = truncated(description, 4000);
    let occurred = occurred.map_or(Py::Null, s);
    let fee_display = if truthy(&fee_display) {
        fee_display
    } else {
        s("0")
    };
    let canonical = dict([
        ("occurred_at", occurred.clone()),
        ("direction", side.clone()),
        ("asset", asset.clone()),
        ("amount_btc", amount_text.clone()),
        ("fee_btc", fee_display.clone()),
    ]);
    let canonical = super::canonical(&canonical)?;
    let encoded = canonical
        .as_str()
        .ok_or_else(|| crate::unicode_binding_error(&canonical))?;
    let digest: String = Sha256::digest(encoded.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let row_id = format!(
        "docrow-{}-{}-001",
        source_hash.chars().take(16).collect::<String>(),
        &digest[..16]
    );
    let valid_fiat = [(value, &value_number), (rate, &rate_number)]
        .iter()
        .all(|(v, n)| missing(v) || n.as_ref().is_some_and(storable_fiat))
        && ((missing(value) && missing(rate)) || fiat != Py::Null)
        && (fiat == Py::Null || currency.is_empty() || fiat == s(&currency));
    let importable = occurred != Py::Null
        && side != Py::Null
        && amount_text != Py::Null
        && amount_number
            .as_ref()
            .is_some_and(|n| n.is_positive() && exact_msat(n))
        && [s("BTC"), s("LBTC")].contains(&asset)
        && (missing(fee) || fee_number.as_ref().is_some_and(exact_msat))
        && valid_fiat;
    let import_record = if importable {
        dict([
            ("id", s(&row_id)),
            ("occurred_at", occurred.clone()),
            ("direction", side.clone()),
            ("asset", asset.clone()),
            ("amount", amount_text),
            ("fee", if truthy(&fee_text) { fee_text } else { s("0") }),
            ("fiat_currency", fiat.clone()),
            ("fiat_value", value_text.clone()),
            ("fiat_rate", rate_text.clone()),
            ("counterparty", counterparty.clone()),
            ("description", description.clone()),
            (
                "raw_json",
                dict([
                    ("source", s("document_import")),
                    ("row_id", s(&row_id)),
                    (
                        "model_row_id",
                        truncated(
                            cleaned(if truthy(field(raw, "id")) {
                                field(raw, "id")
                            } else {
                                field(raw, "row_id")
                            }),
                            256,
                        ),
                    ),
                    ("model_confidence", Py::Float(float(&model_confidence))),
                    ("cell_confidences", cells.clone()),
                    ("fee_defaulted", Py::Bool(missing(fee))),
                    ("source_region", region.clone()),
                    ("evidence_text", evidence.clone()),
                ]),
            ),
        ])
    } else {
        Py::Null
    };
    Ok(dict([
        ("id", s(row_id)),
        (
            "status",
            s(if flags.is_empty() {
                "ready"
            } else {
                "quarantined"
            }),
        ),
        ("flags", Py::List(flags.into_iter().map(s).collect())),
        ("confidence", Py::Float(float(&model_confidence))),
        ("cell_confidences", cells),
        ("confidence_threshold", Py::Float(float(threshold))),
        ("source_region", region),
        ("evidence_text", evidence),
        (
            "record",
            dict([
                ("occurred_at", occurred),
                ("direction", side),
                ("asset", asset),
                ("amount_btc", display(amount, &decimal_text(amount))),
                ("fee_btc", fee_display),
                ("fee_defaulted", Py::Bool(missing(fee))),
                ("fiat_currency", fiat),
                ("fiat_value", display(value, &value_text)),
                ("fiat_rate", display(rate, &rate_text)),
                ("counterparty", counterparty),
                ("description", description),
            ]),
        ),
        ("import_record", import_record),
    ]))
}
pub(super) fn draft_rows(
    payload: &Py,
    source_hash: &str,
    threshold: &Decimal,
    currency: &str,
    allowed_pages: Option<&[i64]>,
) -> Result<Vec<Py>, AppError> {
    let Py::List(raw) = field(payload, "rows") else {
        let mut keys = match payload {
            Py::Dict(fields) => fields
                .iter()
                .map(|(key, _)| match key {
                    Key::Str(v) => Py::Str(v.clone()),
                    _ => s(format!("{key:?}")),
                })
                .collect::<Vec<_>>(),
            _ => vec![],
        };
        keys.sort_by_key(rendered);
        return Err(AppError::new(
            "document_import_ai_response_invalid",
            "Local AI OCR output must contain a rows list",
        )
        .with_details(dict([("keys", Py::List(keys))]))
        .retryable(true));
    };
    if raw.len() > 500 {
        return Err(AppError::new(
            "document_import_ai_response_invalid",
            "Local AI OCR output contains too many rows",
        )
        .with_details(dict([
            ("rows", Py::from(raw.len() as i64)),
            ("max_rows", Py::from(500)),
        ])));
    }
    let mut rows = raw
        .iter()
        .filter(|v| matches!(v, Py::Dict(_)))
        .map(|v| draft_row(v, source_hash, threshold, currency, allowed_pages))
        .collect::<Result<Vec<_>, _>>()?;
    let mut groups = BTreeMap::<String, Vec<(PyStr, usize)>>::new();
    for (index, row) in rows.iter().enumerate() {
        let id = rendered(field(row, "id"));
        let base = id.rsplit_once('-').unwrap().0;
        let order = dict([
            ("source_region", field(row, "source_region").clone()),
            ("evidence_text", field(row, "evidence_text").clone()),
            ("record", field(row, "record").clone()),
        ]);
        let order = canonical(&order)?;
        groups.entry(base.into()).or_default().push((order, index));
    }
    for (base, mut duplicates) in groups {
        duplicates.sort_by(|a, b| a.0.cmp(&b.0));
        for (ordinal, (_, index)) in duplicates.into_iter().enumerate() {
            let id = s(format!("{base}-{:03}", ordinal + 1));
            set(&mut rows[index], "id", id.clone());
            let mut record = field(&rows[index], "import_record").clone();
            if matches!(record, Py::Dict(_)) {
                set(&mut record, "id", id.clone());
                let mut raw = field(&record, "raw_json").clone();
                set(&mut raw, "row_id", id);
                set(&mut record, "raw_json", raw);
                set(&mut rows[index], "import_record", record);
            }
        }
    }
    Ok(rows)
}
