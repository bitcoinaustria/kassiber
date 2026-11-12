//! The Austrian E 1kv report PDF (`export_austrian_e1kv_pdf_report`) in the
//! shared report design. The model is what Python hands to its writer: the
//! E 1kv report, the current portfolio, the tax year's transactions and the
//! report clock; the design renders it.
use super::*;
use artifact_csv::{File, Prepared};

/// `_austrian_e1kv_transaction_rows`: the tax year's active transactions in
/// the Europe/Vienna calendar, with their tags.
fn transactions(conn: &Connection, profile: &Value, year: i64) -> Result<Vec<Value>> {
    let (start, end) = calendar::window(year);
    let mut out = Vec::new();
    for row in rows(
        conn,
        "SELECT t.id AS transaction_id,t.external_id,t.occurred_at,w.label AS wallet,t.direction,COALESCE(NULLIF(t.kind_override,''),t.kind) AS kind,t.asset,t.amount,t.fee,t.fiat_value,COALESCE(t.description,'') AS description,COALESCE(t.note,'') AS note,GROUP_CONCAT(tags.code,', ') AS tags FROM transactions t JOIN wallets w ON w.id=t.wallet_id LEFT JOIN transaction_tags tt ON tt.transaction_id=t.id LEFT JOIN tags ON tags.id=tt.tag_id WHERE t.profile_id=? AND t.excluded=0 AND t.occurred_at>=? AND t.occurred_at<? GROUP BY t.id ORDER BY t.occurred_at ASC,t.created_at ASC,t.id ASC",
        [&text(profile, "id"), &start, &end],
    )? {
        if calendar::vienna(&text(&row, "occurred_at"))?.0 != year {
            continue;
        }
        let or_empty = |key: &str| {
            let v = value(&row, key);
            if truthy(v) {
                v.clone()
            } else {
                s("")
            }
        };
        let msat = |key: &str| int(&row, key);
        let fiat = match value(&row, "fiat_value") {
            v if truthy(v) => float(&decimal(v)?)?,
            _ => Value::Float(0.0),
        };
        out.push(dict([
            ("transaction_id", value(&row, "transaction_id").clone()),
            ("external_id", or_empty("external_id")),
            ("occurred_at", value(&row, "occurred_at").clone()),
            ("wallet", value(&row, "wallet").clone()),
            ("direction", value(&row, "direction").clone()),
            ("kind", or_empty("kind")),
            ("asset", value(&row, "asset").clone()),
            ("amount", float(&btc(msat("amount"))?)?),
            ("amount_msat", Value::Int(msat("amount").into())),
            ("fee", float(&btc(msat("fee"))?)?),
            ("fee_msat", Value::Int(msat("fee").into())),
            ("fiat_value", fiat),
            ("description", or_empty("description")),
            ("note", or_empty("note")),
            ("tags", or_empty("tags")),
        ]));
    }
    Ok(out)
}

/// The Austrian report's model, read in Python's order.
pub fn model(
    conn: &Connection,
    workspace: &Value,
    profile: &Value,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Value> {
    austrian::validate(profile)?;
    let report = austrian::report(conn, workspace, profile, args)?;
    let year = int(&report, "tax_year");
    let transactions = transactions(conn, profile, year)?;
    let portfolio = holdings::report(conn, profile, true)?;
    Ok(dict([
        ("report", report),
        ("portfolio", portfolio),
        ("transactions", Value::List(transactions)),
        ("generated_at", s(sources.now(conn)?)),
        ("workspace", value(workspace, "label").clone()),
        (
            "profile",
            dict(
                [
                    "label",
                    "gains_algorithm",
                    "fiat_currency",
                    "tax_country",
                    "journal_input_version",
                ]
                .map(|key| (key, value(profile, key).clone())),
            ),
        ),
    ]))
}

/// The sections Python's envelope lists for its own layout, kept so the
/// envelope stays Python's.
const SECTIONS: [&str; 9] = [
    "steuerpflichtige_gesamtuebersicht",
    "steuerpflichtige_detailuebersicht",
    "steuerfreie_gesamtuebersicht",
    "bestandsuebersicht",
    "besonderheiten",
    "erlaeuterungen",
    "transaktionsuebersicht",
    "steuerformulare",
    "faq",
];

pub(super) fn prepare(
    conn: &Connection,
    workspace: &Value,
    profile: &Value,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Prepared> {
    let model = model(conn, workspace, profile, args, sources)?;
    let (bytes, pages) = report_design::pdf(&report_design::austrian(&model))?;
    let report = value(&model, "report");
    let year = value(report, "tax_year").clone();
    Ok(Prepared {
        files: vec![File {
            name: "austrian-e1kv.pdf".into(),
            sheet: String::new(),
            bytes,
            rows: 0,
        }],
        metadata: dict([
            ("pages", Value::Int(pages.into())),
            (
                "title",
                s(format!(
                    "Kassiber Steuerbericht {} Österreich",
                    py_str(&year)
                )),
            ),
            ("renderer", s("typst")),
            ("tax_year", year.clone()),
            ("form", value(report, "form").clone()),
            ("assumptions", value(report, "assumptions").clone()),
            ("sections", Value::List(SECTIONS.map(s).to_vec())),
            (
                "transactions",
                Value::Int(
                    artifact_csv::list(value(&model, "transactions"))
                        .len()
                        .into(),
                ),
            ),
        ]),
        completion: Some(crate::export_completion::Completion {
            kind: "austrian-e1kv.pdf".into(),
            year: Some(year),
            end_year: None,
            scope: Value::Null,
        }),
    })
}
