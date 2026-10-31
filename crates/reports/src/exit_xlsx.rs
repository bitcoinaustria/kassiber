use super::*;
use artifact_csv::{list, File, Prepared, Section};
use kassiber_xlsx::Document;

pub(super) fn project(
    conn: &Connection,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<(Value, Document)> {
    let (workspace, profile) = kassiber_setup::references::scope(conn, args)?;
    require_context(conn, &profile, false)?;
    let report = exit_tax::report(conn, &workspace, &profile, args, sources)?;
    let totals = value(&report, "totals");
    let fiat = text(&report, "fiatCurrency");
    let overview = [
        ("Profile".into(), value(&report, "profile")),
        ("Jurisdiction".into(), value(&report, "jurisdictionCode")),
        ("Departure date".into(), value(&report, "departureDate")),
        ("Destination".into(), value(&report, "destination")),
        (
            "Collection timing".into(),
            value(totals, "collectionTiming"),
        ),
        ("Accounting method".into(), value(&report, "method")),
        (
            format!("Neubestand gain ({fiat})"),
            value(totals, "neuGain"),
        ),
        (
            format!("Taxable gain ({fiat})"),
            value(totals, "taxableGain"),
        ),
        ("Rate".into(), value(totals, "estimatedTaxRate")),
        (
            format!("Estimated exit tax ({fiat})"),
            value(totals, "estimatedTax"),
        ),
        (
            format!("Altbestand market value — excluded ({fiat})"),
            value(totals, "altMarketValue"),
        ),
    ]
    .into_iter()
    .map(|(k, v)| dict([("field", s(k)), ("value", v.clone())]))
    .collect();
    let quantity = |r: &Value| -> Result<Value> {
        float(
            &decimal(value(r, "quantitySats"))?
                .div(&Decimal::from(100_000_000), &DECIMAL)
                .map_err(arithmetic)?,
        )
    };
    let lots = list(value(&report, "lots"))
        .iter()
        .map(|r| {
            Ok(dict([
                ("asset", value(r, "asset").clone()),
                ("regime", value(r, "regime").clone()),
                ("quantity", quantity(r)?),
                ("cost_basis", value(r, "costBasis").clone()),
                ("market_value", value(r, "marketValue").clone()),
                ("gain_loss", value(r, "gain").clone()),
                (
                    "taxable",
                    s(if truthy(value(r, "taxable")) {
                        "yes"
                    } else {
                        "no"
                    }),
                ),
                ("kennzahl", value(r, "kennzahl").clone()),
            ]))
        })
        .collect::<Result<Vec<_>>>()?;
    let holdings = list(value(&report, "walletHoldings"))
        .iter()
        .map(|r| {
            Ok(dict([
                ("asset", value(r, "asset").clone()),
                ("wallet", value(r, "wallet").clone()),
                ("quantity", quantity(r)?),
                ("market_value", value(r, "marketValue").clone()),
            ]))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut notes: Vec<Value> = list(value(&report, "assumptions"))
        .iter()
        .map(|n| dict([("note", n.clone())]))
        .collect();
    notes.push(dict([("note", value(&report, "reviewGate").clone())]));
    let sections = [
        Section {
            name: "Overview",
            title: format!("Exit-tax estimate — {}", text(&report, "profile")),
            headers: &["field", "value"],
            rows: overview,
        },
        Section {
            name: "Deemed disposal",
            title: "Deemed disposal at fair market value".into(),
            headers: &[
                "asset",
                "regime",
                "quantity",
                "cost_basis",
                "market_value",
                "gain_loss",
                "taxable",
                "kennzahl",
            ],
            rows: lots,
        },
        Section {
            name: "Wallet holdings",
            title: "Wallet holdings (context)".into(),
            headers: &["asset", "wallet", "quantity", "market_value"],
            rows: holdings,
        },
        Section {
            name: "Assumptions",
            title: "Assumptions & Steuerberater review gate".into(),
            headers: &["note"],
            rows: notes,
        },
    ];
    let document = Document {
        title: format!(
            "Kassiber Exit-Tax Estimate — {} ({})",
            text(&report, "profile"),
            text(&report, "departureDate")
        ),
        subject: "Wegzugsbesteuerung deemed-disposal estimate".into(),
        author: "Kassiber".into(),
        comments: text(&report, "reviewGate"),
        created: sources.now(conn)?,
        formats: artifact_xlsx::styles(),
        sheets: sections
            .iter()
            .map(artifact_xlsx::sheet)
            .collect::<Result<_>>()?,
        names: vec![],
    };
    let metadata = dict([
        ("scope", s("exit_tax")),
        ("format", s("xlsx")),
        ("departure_date", value(&report, "departureDate").clone()),
        ("destination", value(&report, "destination").clone()),
        (
            "sheets",
            Value::List(sections.iter().map(|section| s(section.name)).collect()),
        ),
        (
            "rows",
            Value::Int(list(value(&report, "lots")).len().into()),
        ),
    ]);
    Ok((metadata, document))
}
pub(super) fn prepare(
    conn: &Connection,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Prepared> {
    let (metadata, document) = project(conn, args, sources)?;
    let bytes = kassiber_xlsx::render(&document)
        .map_err(|e| AppError::new("internal_error", e.to_string()))?;
    Ok(Prepared {
        files: vec![File {
            name: "report.xlsx".into(),
            sheet: "".into(),
            bytes,
            rows: int(&metadata, "rows") as usize,
        }],
        metadata,
        completion: None,
    })
}
