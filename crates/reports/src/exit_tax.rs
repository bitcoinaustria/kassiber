use super::*;
use std::collections::BTreeMap;

fn metadata() -> &'static Value {
    static META: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    META.get_or_init(|| kassiber_pytext::loads(include_str!("exit_tax_metadata.json")).unwrap())
}
fn note(key: &str) -> Value {
    value(metadata(), key).clone()
}
fn exact(row: &Value, key: &str) -> Result<Option<Decimal>> {
    for v in [value(row, &format!("{key}_exact")), value(row, key)] {
        if !matches!(v, Value::Null) && v != &s("") {
            return decimal(v).map(Some);
        }
    }
    Ok(None)
}
fn eur(d: Option<&Decimal>) -> Result<Value> {
    d.map(|d| {
        float(
            &d.quantize(-2, &Context::new(32, Rounding::HalfUp))
                .map_err(arithmetic)?,
        )
    })
    .transpose()
    .map(|v| v.unwrap_or(Value::Null))
}
fn sats(d: &Decimal) -> Result<Value> {
    austrian::integer(
        &d.mul(&Decimal::from(100_000_000), &DECIMAL)
            .map_err(arithmetic)?
            .quantize(0, &Context::new(32, Rounding::HalfUp))
            .map_err(arithmetic)?,
    )
}
fn marker(row: &Value) -> Option<String> {
    text(row, "description")
        .split(|c: char| kassiber_pytext::is_space(c as u32))
        .find_map(|t| t.strip_prefix("at_regime="))
        .filter(|v| ["alt", "neu"].contains(v))
        .map(String::from)
}
fn alt(row: &Value) -> Result<bool> {
    if let Some(m) = marker(row) {
        return Ok(m == "alt");
    }
    if truthy(value(row, "at_category")) {
        return Ok(text(row, "at_category").starts_with("alt"));
    }
    let at = text(row, "occurred_at");
    if !truthy(value(row, "occurred_at")) {
        return Ok(false);
    }
    let at = kassiber_rates::time::parse(&at).ok_or_else(|| {
        AppError::new(
            "internal_error",
            format!(
                "Invalid isoformat string: {}",
                kassiber_pytext::repr_str(&at)
            ),
        )
    })?;
    Ok(at < kassiber_rates::time::parse("2021-02-28T23:00:00Z").unwrap())
}
fn destination(raw: &Value) -> Result<String> {
    let v = if truthy(raw) {
        py_str(raw)
    } else {
        "eu_eea".into()
    };
    match kassiber_pytext::strip(&v)
        .to_lowercase()
        .replace('-', "_")
        .as_str()
    {
        "eu" | "eea" | "eu_eea" | "euea" => Ok("eu_eea".into()),
        "third_country" | "third" | "non_eu" | "noneu" => Ok("third_country".into()),
        _ => Err(AppError::new(
            "validation",
            format!("Unsupported exit-tax destination '{}'", py_str(raw)),
        )
        .with_hint("Choose one of: eu_eea, third_country")),
    }
}
fn departure(
    conn: &Connection,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<String> {
    let raw = value(args, "departure_date");
    if !truthy(raw) {
        return Ok(sources.now(conn)?[..10].into());
    }
    let raw = py_str(raw);
    let stripped = kassiber_pytext::strip(&raw);
    let input = if stripped.chars().count() == 10 {
        format!("{stripped}T00:00:00+00:00")
    } else {
        stripped.into()
    };
    kassiber_rates::time::parse(&input)
        .and_then(kassiber_rates::time::format)
        .map(|s| s[..10].into())
        .ok_or_else(|| {
            AppError::new(
                "validation",
                format!("Invalid occurred_at/date value '{raw}'"),
            )
            .with_hint("Use RFC3339 UTC like 2025-01-01T00:00:00Z")
        })
}
fn transaction_value(row: &Value, key: &str) -> Result<Option<Decimal>> {
    let precise = value(row, &format!("{key}_exact"));
    let stored = if matches!(precise, Value::Null) {
        value(row, key)
    } else {
        precise
    };
    if matches!(stored, Value::Null) {
        Ok(None)
    } else {
        decimal(stored).map(Some)
    }
}
fn fallback_rates(conn: &Connection, profile: &Value) -> Result<BTreeMap<String, Decimal>> {
    let mut result = BTreeMap::new();
    for row in rows(conn,"SELECT asset,fiat_rate,fiat_value,fiat_rate_exact,fiat_value_exact,amount FROM transactions WHERE profile_id=? AND excluded=0 ORDER BY occurred_at DESC,created_at DESC,id DESC",[text(profile,"id")])? {
        let asset = text(&row,"asset");
        if result.contains_key(&asset) { continue; }
        let rate = transaction_value(&row,"fiat_rate")?;
        let total = transaction_value(&row,"fiat_value")?;
        if let Some(rate) = rate { result.insert(asset,rate); }
        else if let Some(total) = total {
            if int(&row,"amount") != 0 { result.insert(asset,total.div(&btc(int(&row,"amount"))?,&DECIMAL).map_err(arithmetic)?); }
        }
    }
    Ok(result)
}
fn fmv(
    conn: &Connection,
    asset: &str,
    fiat: &str,
    cutoff: &str,
    fallback: &BTreeMap<String, Decimal>,
) -> Result<(Option<Decimal>, Value)> {
    let pair = kassiber_rates::transaction_rate_pair(asset, fiat);
    let mut found = None;
    let mut as_of = s(cutoff);
    let mut source = "missing";
    if let Some(pair) = &pair {
        let cutoff_at = kassiber_rates::time::parse(cutoff).unwrap();
        let cached = sql::optional_rows(conn,"SELECT timestamp,rate,rate_exact FROM rates_cache WHERE pair=? ORDER BY timestamp DESC,CASE WHEN source='manual' THEN 0 ELSE 1 END ASC,fetched_at DESC",[pair])?
            .into_iter().find(|row| kassiber_rates::time::parse(&text(row,"timestamp")).is_some_and(|at| at <= cutoff_at));
        if let Some(cached) = cached {
            if let Some(rate) = exact(&cached, "rate")? {
                if rate > Decimal::zero() {
                    found = Some(rate);
                    as_of = value(&cached, "timestamp").clone();
                    source = "cache";
                }
            }
        }
    }
    if found.is_none() {
        if let Some(rate) = fallback.get(asset).filter(|r| **r > Decimal::zero()) {
            found = Some(rate.clone());
            source = "transaction";
        }
    }
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
            "pair",
            s(pair.unwrap_or_else(|| format!("{asset}-{}", fiat.to_uppercase()))),
        ),
        ("asOf", as_of),
        ("source", s(source)),
    ]);
    Ok((found, row))
}

pub(super) fn report(
    conn: &Connection,
    workspace: &Value,
    profile: &Value,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Value> {
    let is_at = kassiber_pytext::strip(&text(profile, "tax_country")).to_lowercase() == "at";
    let fiat = if truthy(value(profile, "fiat_currency")) {
        text(profile, "fiat_currency").to_uppercase()
    } else {
        "EUR".into()
    };
    let entries = rows(conn,"SELECT * FROM journal_entries WHERE profile_id=? ORDER BY occurred_at ASC,created_at ASC,id ASC",[text(profile,"id")])?;
    for row in &entries {
        for field in [
            "fiat_value",
            "unit_cost",
            "cost_basis",
            "proceeds",
            "gain_loss",
        ] {
            exact(row, field)?;
        }
    }
    let mut holdings: Vec<Value> = Vec::new();
    for row in rows(conn,"SELECT wallet_id,wallet_label,account_code,asset,quantity,cost_basis FROM journal_wallet_holdings WHERE profile_id=?",[text(profile,"id")])? {
        exact(&row,"cost_basis")?;
        if let Some(previous) = holdings.iter_mut().find(|r| ["wallet_id","wallet_label","account_code","asset"].iter().all(|k| value(r,k) == value(&row,k))) { *previous = row; } else { holdings.push(row); }
    }
    let fallback = fallback_rates(conn, profile)?;
    let day = departure(conn, args, sources)?;
    let cutoff = format!("{day}T23:59:59Z");
    let dest = destination(value(args, "destination"))?;
    if !is_at && truthy(value(profile, "tax_long_term_days")) {
        artifact_csv::history_limit(value(profile, "tax_long_term_days")).map_err(|e| {
            if e.code == "validation" {
                AppError::new("internal_error", "tax_long_term_days cannot be negative")
            } else {
                e
            }
        })?;
    }

    let mut assets: BTreeMap<String, [(Decimal, Decimal); 2]> = BTreeMap::new();
    let (mut income, mut untagged) = (false, false);
    for row in &entries {
        if truthy(value(row, "occurred_at")) && text(row, "occurred_at") > cutoff {
            continue;
        }
        let kind = text(row, "entry_type");
        if ["transfer_in", "transfer_out"].contains(&kind.as_str()) {
            continue;
        }
        if kind == "income" {
            income = true;
            continue;
        }
        let asset = text(row, "asset");
        if !truthy(value(row, "asset")) {
            continue;
        }
        let quantity = btc(int(row, "quantity"))?;
        let regime = usize::from(is_at && alt(row)?);
        let bucket = &mut assets
            .entry(asset)
            .or_insert_with(|| std::array::from_fn(|_| (Decimal::zero(), Decimal::zero())))[regime];
        let basis = exact(row, "cost_basis")?;
        let basis = if quantity >= Decimal::zero() {
            match basis {
                Some(basis) => basis,
                None => exact(row, "fiat_value")?.unwrap_or_else(Decimal::zero),
            }
        } else {
            if is_at && !truthy(value(row, "at_category")) && marker(row).is_none() {
                untagged = true;
            }
            basis
                .unwrap_or_else(Decimal::zero)
                .neg(&DECIMAL)
                .map_err(arithmetic)?
        };
        bucket.0 = bucket.0.add(&quantity, &DECIMAL).map_err(arithmetic)?;
        bucket.1 = bucket.1.add(&basis, &DECIMAL).map_err(arithmetic)?;
    }
    let mut sources_out = Vec::new();
    let mut rates = BTreeMap::new();
    for asset in assets.keys() {
        let (rate, source) = fmv(conn, asset, &fiat, &cutoff, &fallback)?;
        rates.insert(asset.clone(), rate);
        sources_out.push(source);
    }
    let mut lots = Vec::new();
    let (mut neu_qty, mut neu_basis, mut alt_qty) =
        (Decimal::zero(), Decimal::zero(), Decimal::zero());
    let (mut neu_market, mut neu_gain, mut alt_market) = (
        Some(Decimal::zero()),
        Some(Decimal::zero()),
        Some(Decimal::zero()),
    );
    for (asset, buckets) in &assets {
        for (regime, (quantity, basis)) in buckets.iter().enumerate() {
            let qty = quantity.clone().max(Decimal::zero());
            let basis = basis.clone().max(Decimal::zero());
            if qty <= Decimal::zero() {
                continue;
            }
            let market = rates[asset]
                .as_ref()
                .map(|r| qty.mul(r, &DECIMAL).map_err(arithmetic))
                .transpose()?;
            let gain = if regime == 0 {
                market
                    .as_ref()
                    .map(|m| m.sub(&basis, &DECIMAL).map_err(arithmetic))
                    .transpose()?
            } else {
                None
            };
            let category = if regime == 1 {
                "alt_taxfree"
            } else if gain.as_ref().is_none_or(|g| *g >= Decimal::zero()) {
                "neu_gain"
            } else {
                "neu_loss"
            };
            if regime == 0 {
                neu_qty = neu_qty.add(&qty, &DECIMAL).map_err(arithmetic)?;
                neu_basis = neu_basis.add(&basis, &DECIMAL).map_err(arithmetic)?;
                neu_market = add_optional(neu_market, market.as_ref())?;
                neu_gain = add_optional(neu_gain, gain.as_ref())?;
            } else {
                alt_qty = alt_qty.add(&qty, &DECIMAL).map_err(arithmetic)?;
                alt_market = add_optional(alt_market, market.as_ref())?;
            }
            lots.push(dict([
                ("asset", s(asset.as_str())),
                ("regime", s(if regime == 0 { "neu" } else { "alt" })),
                ("quantitySats", sats(&qty)?),
                ("marketValue", eur(market.as_ref())?),
                (
                    "costBasis",
                    if regime == 0 {
                        eur(Some(&basis))?
                    } else {
                        Value::Null
                    },
                ),
                ("gain", eur(gain.as_ref())?),
                ("taxable", Value::Bool(regime == 0)),
                ("category", s(if is_at { category } else { "unrealized" })),
                (
                    "kennzahl",
                    if is_at && regime == 0 {
                        Value::Int((if category == "neu_gain" { 174 } else { 176 }).into())
                    } else {
                        Value::Null
                    },
                ),
            ]));
        }
    }
    let taxable = neu_gain.as_ref().map(|g| g.clone().max(Decimal::zero()));
    let rate: Decimal = "0.275".parse().unwrap();
    let estimated = if is_at {
        taxable
            .as_ref()
            .map(|g| g.mul(&rate, &DECIMAL).map_err(arithmetic))
            .transpose()?
    } else {
        None
    };
    let mut wallet_holdings = Vec::new();
    for row in holdings {
        let qty = btc(int(&row, "quantity"))?;
        if qty <= Decimal::zero() {
            continue;
        }
        let market = rates
            .get(&text(&row, "asset"))
            .and_then(Option::as_ref)
            .map(|r| qty.mul(r, &DECIMAL).map_err(arithmetic))
            .transpose()?;
        wallet_holdings.push(dict([
            ("wallet", value(&row, "wallet_label").clone()),
            ("asset", value(&row, "asset").clone()),
            ("quantitySats", sats(&qty)?),
            ("marketValue", eur(market.as_ref())?),
        ]));
    }
    wallet_holdings.sort_by_key(|r| (text(r, "asset"), text(r, "wallet")));
    let mut assumptions = if is_at {
        vec![
            note("_ASSUMPTION_ALT_EXCLUDED"),
            note("_ASSUMPTION_RATE"),
            note(if dest == "eu_eea" {
                "_ASSUMPTION_DEFERRAL_EU"
            } else {
                "_ASSUMPTION_DEFERRAL_THIRD"
            }),
            note("_ASSUMPTION_FMV"),
        ]
    } else {
        vec![note("_ASSUMPTION_GENERIC"), note("_ASSUMPTION_FMV")]
    };
    if is_at {
        if dest == "eu_eea" {
            assumptions.push(note("_ASSUMPTION_FUTURE_TIGHTENING"));
        }
        if income {
            assumptions.push(note("_ASSUMPTION_DERIVED_TOKENS"));
        }
        if untagged {
            assumptions.push(note("_ASSUMPTION_UNTAGGED_DISPOSAL"));
        }
    }
    if sources_out
        .iter()
        .any(|r| value(r, "source") == &s("missing"))
    {
        assumptions.push(note("missing"));
    }
    let quarantines: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM journal_quarantines WHERE profile_id=?",
            [text(profile, "id")],
            |r| r.get(0),
        )
        .map_err(sql_error)?;
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
        ("jurisdictionCode", s(if is_at { "AT" } else { "generic" })),
        ("fiatCurrency", s(fiat)),
        ("departureDate", s(day)),
        ("destination", s(dest.as_str())),
        (
            "method",
            s(if is_at { "moving_average_at" } else { "fifo" }),
        ),
        ("fmvSource", Value::List(sources_out)),
        (
            "totals",
            dict([
                ("neuQuantitySats", sats(&neu_qty)?),
                ("neuMarketValue", eur(neu_market.as_ref())?),
                ("neuCostBasis", eur(Some(&neu_basis))?),
                ("neuGain", eur(neu_gain.as_ref())?),
                ("altQuantitySats", sats(&alt_qty)?),
                ("altMarketValue", eur(alt_market.as_ref())?),
                ("taxableGain", eur(taxable.as_ref())?),
                (
                    "estimatedTaxRate",
                    if is_at { float(&rate)? } else { Value::Null },
                ),
                ("estimatedTax", eur(estimated.as_ref())?),
                (
                    "collectionTiming",
                    s(if dest == "eu_eea" {
                        "deferred"
                    } else {
                        "immediate"
                    }),
                ),
            ]),
        ),
        ("lots", Value::List(lots)),
        ("walletHoldings", Value::List(wallet_holdings)),
        ("assumptions", Value::List(assumptions)),
        ("reviewGate", note("EXIT_TAX_REVIEW_GATE")),
        (
            "status",
            dict([
                ("needsJournals", Value::Bool(entries.is_empty())),
                ("quarantines", Value::Int(quarantines.into())),
            ]),
        ),
    ]))
}
fn add_optional(a: Option<Decimal>, b: Option<&Decimal>) -> Result<Option<Decimal>> {
    a.zip(b)
        .map(|(a, b)| a.add(b, &DECIMAL).map_err(arithmetic))
        .transpose()
}

pub(super) fn render(report: &Value) -> Result<String> {
    let totals = value(report, "totals");
    let ccy = text(report, "fiatCurrency");
    let money = |key| {
        if matches!(value(totals, key), Value::Null) {
            "—".into()
        } else {
            format!("{} {ccy}", summary_text::number(value(totals, key), 2))
        }
    };
    let btc = |key| -> Result<String> {
        decimal(value(totals, key))?
            .div(&Decimal::from(100_000_000), &DECIMAL)
            .map_err(arithmetic)?
            .to_fixed_places(8, Rounding::HalfEven)
            .map_err(arithmetic)
    };
    let mut lines = vec![
        format!(
            "Exit-tax estimate — {} ({})",
            text(report, "profile"),
            text(report, "jurisdictionCode")
        ),
        format!(
            "Departure date: {}    Destination: {}",
            text(report, "departureDate"),
            text(report, "destination")
        ),
        format!("Accounting method: {}", text(report, "method")),
        String::new(),
        "Deemed disposal at fair market value:".into(),
        format!(
            "  Neubestand (taxable):  {} @ market {}, basis {}, gain {}",
            btc("neuQuantitySats")?,
            money("neuMarketValue"),
            money("neuCostBasis"),
            money("neuGain")
        ),
        format!(
            "  Altbestand (excluded): {} @ market {} — tax-free, not in the base",
            btc("altQuantitySats")?,
            money("altMarketValue")
        ),
        String::new(),
        format!("  Taxable gain:        {}", money("taxableGain")),
    ];
    if !matches!(value(totals, "estimatedTaxRate"), Value::Null) {
        lines.push("  Rate:                27.5%".into());
    }
    lines.push(format!("  Estimated exit tax:  {}", money("estimatedTax")));
    lines.push(
        if text(totals, "collectionTiming") == "deferred" {
            "  Collection:          assessed but deferred until you sell (EU/EEA Nichtfestsetzung)"
        } else {
            "  Collection:          due immediately on departure (third country)"
        }
        .into(),
    );
    lines.push(String::new());
    let quarantines = int(value(report, "status"), "quarantines");
    if quarantines != 0 {
        lines.push(format!(
            "⚠ {quarantines} quarantined transaction(s) — estimate is incomplete until resolved."
        ));
        lines.push(String::new());
    }
    lines.push("Assumptions:".into());
    for note in artifact_csv::list(value(report, "assumptions")) {
        lines.push(format!("  - {}", py_str(note)));
    }
    lines.push(String::new());
    lines.push(text(report, "reviewGate"));
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kassiber_xlsx::Document;
    struct Fixed;
    impl kassiber_setup::Sources for Fixed {
        fn now(&self, _: &Connection) -> Result<String> {
            Ok("2026-01-02T03:04:05Z".into())
        }
        fn uuid(&self, _: &Connection) -> Result<String> {
            Ok("00000000-0000-4000-8000-000000000001".into())
        }
    }
    fn operations(doc: &Document) -> Vec<(String, Vec<String>)> {
        doc.sheets
            .iter()
            .map(|sheet| {
                let mut operations: Vec<String> = sheet
                    .operations
                    .iter()
                    .map(|op| {
                        let mut json = serde_json::to_value(op).unwrap();
                        if let Some(index) = json.get("style").and_then(|n| n.as_u64()) {
                            json["style"] =
                                serde_json::to_value(&doc.formats[index as usize]).unwrap();
                        }
                        serde_json::to_string(&json).unwrap()
                    })
                    .collect();
                operations.sort();
                (sheet.name.clone(), operations)
            })
            .collect()
    }
    fn normalize_numbers(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Number(number)
                if number.as_u64().is_none() && number.as_i64().is_none() =>
            {
                *number = serde_json::Number::from_f64(number.as_f64().unwrap()).unwrap();
            }
            serde_json::Value::Array(values) => values.iter_mut().for_each(normalize_numbers),
            serde_json::Value::Object(fields) => fields.values_mut().for_each(normalize_numbers),
            _ => (),
        }
    }

    fn encoded(v: &Value) -> String {
        kassiber_pyjson::dumps(
            v,
            &kassiber_pyjson::Options {
                sort_keys: true,
                ..Default::default()
            },
        )
        .unwrap()
    }
    fn error(e: AppError) -> Value {
        dict([
            ("code", s(e.code)),
            ("message", s(e.message)),
            ("hint", e.hint.map(s).unwrap_or(Value::Null)),
            ("details", e.details.unwrap_or(Value::Null)),
            ("retryable", Value::Bool(e.retryable)),
        ])
    }
    #[test]
    fn exit_tax_vectors_pin_inventory_rates_disclosures_and_readiness() {
        let mut cases: serde_json::Value =
            serde_json::from_str(include_str!("../tests/data/exit_tax.json")).unwrap();
        normalize_numbers(&mut cases);
        let root = tempfile::tempdir().unwrap();
        let book = kassiber_store::Book::create(&root.path().join("data"), None).unwrap();
        let conn = book.connection();
        conn.execute_batch(include_str!("../tests/data/book.sql"))
            .unwrap();
        for case in cases.as_array().unwrap() {
            conn.execute_batch("SAVEPOINT exit_vector").unwrap();
            conn.execute_batch(case["sql"].as_str().unwrap()).unwrap();
            let args = kassiber_pytext::loads(case["args_json"].as_str().unwrap()).unwrap();
            let result = execute_with_sources(conn, "reports.exit-tax", &args, &Fixed);
            if case["expected"].get("error").is_some() {
                let expected =
                    kassiber_pytext::loads(&case["expected"]["error"].to_string()).unwrap();
                assert_eq!(
                    encoded(&error(result.unwrap_err())),
                    encoded(&expected),
                    "{}",
                    case["name"]
                );
                assert_eq!(
                    encoded(&error(exit_xlsx::project(conn, &args, &Fixed).unwrap_err())),
                    encoded(&expected),
                    "{} workbook",
                    case["name"]
                );
            } else {
                let report = result.unwrap();
                let expected =
                    kassiber_pytext::loads(&case["expected"]["report"].to_string()).unwrap();
                assert_eq!(encoded(&report), encoded(&expected), "{}", case["name"]);
                assert_eq!(
                    render(&report).unwrap(),
                    case["expected"]["text"].as_str().unwrap(),
                    "{} text",
                    case["name"]
                );
                let (_, doc) = exit_xlsx::project(conn, &args, &Fixed).unwrap();
                let expected: Document =
                    serde_json::from_value(case["expected"]["document"].clone()).unwrap();
                assert_eq!(
                    (
                        &doc.title,
                        &doc.subject,
                        &doc.author,
                        &doc.comments,
                        &doc.created,
                        &doc.names
                    ),
                    (
                        &expected.title,
                        &expected.subject,
                        &expected.author,
                        &expected.comments,
                        &expected.created,
                        &expected.names
                    ),
                    "{} properties",
                    case["name"]
                );
                assert_eq!(
                    operations(&doc),
                    operations(&expected),
                    "{} workbook",
                    case["name"]
                );
            }
            conn.execute_batch("ROLLBACK TO exit_vector; RELEASE exit_vector")
                .unwrap();
        }
    }
    #[test]
    fn exit_xlsx_managed_publication_preserves_reference_without_snapshots() {
        let root = tempfile::tempdir().unwrap();
        let book = kassiber_store::Book::create(&root.path().join("data"), None).unwrap();
        let conn = book.connection();
        conn.execute_batch(include_str!("../tests/data/book.sql"))
            .unwrap();
        let args = dict([
            ("departure_date", s("2025-01-01")),
            ("destination", s("third_country")),
        ]);
        let output = export_request::execute(
            conn,
            "ui.reports.export_exit_tax_xlsx",
            &args,
            &root.path().join("exports"),
            &Fixed,
        )
        .unwrap();
        assert_eq!(value(&output, "format"), &s("xlsx"));
        assert_eq!(value(&output, "scope"), &s("exit_tax"));
        assert_eq!(value(&output, "departure_date"), &s("2025-01-01"));
        assert_eq!(value(&output, "destination"), &s("third_country"));
        assert_eq!(
            value(&output, "filename"),
            &s("kassiber-exit-tax-2025-01-01-20260102-030405.xlsx")
        );
        let bytes = std::fs::read(text(&output, "file")).unwrap();
        assert!(bytes.starts_with(b"PK"));
        assert_eq!(int(&output, "bytes") as usize, bytes.len());
        assert!(get(&output, "report_snapshot").is_none());
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM filed_report_snapshots", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
        conn.execute_batch("INSERT INTO freshness_source_states(source_key,profile_id,source_type,source_label,status,blocking_reports,updated_at) VALUES('blocked','p','wallet','Cold','failed',1,'fixed')").unwrap();
        assert_eq!(
            export_request::execute(
                conn,
                "ui.reports.export_exit_tax_xlsx",
                &args,
                &root.path().join("exports"),
                &Fixed
            )
            .unwrap_err()
            .code,
            "report_freshness_blocked"
        );
    }
}
