//! The desktop capital-gains CSV in the machine format of the shared report
//! design: snake_case columns, exact decimals, integer msat beside BTC, the
//! Austrian columns only in Austrian books. No saved marker.
use super::*;
use artifact_csv::{File, Prepared};

/// The capital-gains rows with their exact stored amounts.
const SQL: &str = "SELECT je.occurred_at,w.label AS wallet,je.transaction_id,je.entry_type,je.asset,ABS(je.quantity) AS quantity,COALESCE(je.proceeds,0) AS proceeds,je.proceeds_exact,COALESCE(je.cost_basis,0) AS cost_basis,je.cost_basis_exact,COALESCE(je.gain_loss,0) AS gain_loss,je.gain_loss_exact,COALESCE(je.description,'') AS description,je.at_category,je.at_kennzahl FROM journal_entries je JOIN wallets w ON w.id=je.wallet_id LEFT JOIN transactions t ON t.id=je.transaction_id WHERE je.profile_id=? AND je.entry_type IN ('disposal','income') AND COALESCE(t.taxability_override,1)!=0 AND COALESCE(je.at_category,'')!='neu_swap' ORDER BY je.occurred_at ASC,je.created_at ASC,je.id ASC";

/// The exact amount: the stored exact text, else the stored number.
fn amount(row: &Value, key: &str) -> Result<String> {
    let exact = value(row, &format!("{key}_exact"));
    let raw = if truthy(exact) {
        exact
    } else {
        value(row, key)
    };
    Ok(report_design::plain(&match raw {
        Value::Float(_) => report_design::from_float(raw),
        other => decimal(other)?,
    }))
}

pub(super) fn prepare(conn: &Connection, profile: &Value, args: &Value) -> Result<Prepared> {
    let raw = value(args, "year");
    let year = if matches!(raw, Value::Null) {
        None
    } else {
        Some(austrian::normalize_tax_year(raw)?)
    };
    // The identity check and its refusal are the capital-gains read's.
    capital_gains_rows(conn, profile)?;
    let austria = kassiber_pytext::lower(&text(profile, "tax_country")) == "at";
    let mut records = rows(conn, SQL, [text(profile, "id")])?;
    if let Some(year) = year {
        if austria {
            let (start, end) = calendar::window(year);
            records.retain(|r| {
                let at = text(r, "occurred_at");
                at >= start && at < end
            });
            let mut selected = Vec::new();
            for row in records {
                if calendar::vienna(&text(&row, "occurred_at"))?.0 == year {
                    selected.push(row);
                }
            }
            records = selected;
        } else {
            records.retain(|r| {
                text(r, "occurred_at").chars().take(4).collect::<String>() == year.to_string()
            });
        }
    }
    let mut header = vec![
        "occurred_at",
        "wallet",
        "transaction_id",
        "entry_type",
        "asset",
        "quantity_btc",
        "quantity_msat",
        "proceeds",
        "cost_basis",
        "gain_loss",
        "fiat_currency",
        "description",
    ];
    if austria {
        header.extend(["at_category", "at_kennzahl"]);
    }
    let fiat = text(profile, "fiat_currency");
    let lines = records
        .iter()
        .map(|row| {
            let quantity = int(row, "quantity");
            let mut cells = vec![
                text(row, "occurred_at"),
                text(row, "wallet"),
                artifact_csv::cell(value(row, "transaction_id")),
                text(row, "entry_type"),
                text(row, "asset"),
                report_design::plain(&btc(quantity)?),
                quantity.to_string(),
                amount(row, "proceeds")?,
                amount(row, "cost_basis")?,
                amount(row, "gain_loss")?,
                fiat.clone(),
                text(row, "description"),
            ];
            if austria {
                cells.push(artifact_csv::cell(value(row, "at_category")));
                cells.push(artifact_csv::cell(value(row, "at_kennzahl")));
            }
            Ok(cells)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Prepared {
        files: vec![File {
            name: "capital-gains.csv".into(),
            sheet: "Capital gains".into(),
            bytes: report_design::csv_table(&header, lines.into_iter()),
            rows: records.len(),
        }],
        metadata: dict([("rows", Value::Int(records.len().into()))]),
        completion: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capital_gains_csv_is_exact_with_austrian_columns_only_in_austria() {
        let root = tempfile::tempdir().unwrap();
        let book = kassiber_store::Book::create(&root.path().join("data"), None).unwrap();
        let conn = book.connection();
        conn.execute_batch(include_str!("../tests/data/book.sql"))
            .unwrap();
        // Finer than a float: the exact text is what the file must carry.
        conn.execute_batch(
            "UPDATE journal_entries SET cost_basis_exact='10.000000010000000001' WHERE id='z'",
        )
        .unwrap();
        let generic = "occurred_at,wallet,transaction_id,entry_type,asset,quantity_btc,quantity_msat,proceeds,cost_basis,gain_loss,fiat_currency,description";
        for (country, header) in [
            ("generic", generic.to_owned()),
            ("at", format!("{generic},at_category,at_kennzahl")),
        ] {
            conn.execute("UPDATE profiles SET tax_country=?1 WHERE id='p'", [country])
                .unwrap();
            let (_, profile) = kassiber_setup::references::scope(conn, &dict::<&str>([])).unwrap();
            let prepared = prepare(conn, &profile, &dict::<&str>([])).unwrap();
            let csv = String::from_utf8(prepared.files[0].bytes.clone()).unwrap();
            let mut lines = csv.lines();
            assert_eq!(lines.next(), Some(header.as_str()), "{country}");
            let row = lines.find(|l| l.contains("Split z")).unwrap();
            assert!(
                row.contains(",BTC,0.00100000001,100000001,12.34,10.000000010000000001,2.33999999,EUR,Split z"),
                "{country}: {row}"
            );
        }
    }
}
