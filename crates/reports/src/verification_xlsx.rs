use super::*;
use artifact_csv::list;
use kassiber_xlsx::{Cell, Document, Operation, Sheet, Style};
use std::sync::OnceLock;
use verification::number;

fn metadata() -> &'static serde_json::Value {
    static META: OnceLock<serde_json::Value> = OnceLock::new();
    META.get_or_init(|| serde_json::from_str(include_str!("verification_metadata.json")).unwrap())
}
fn style(name: &str, offset: usize) -> usize {
    metadata()["styles"][name].as_u64().unwrap() as usize + offset
}
fn write(ops: &mut Vec<Operation>, row: u32, col: u16, value: Cell, style: usize) {
    ops.push(Operation::Cell {
        row,
        col,
        value,
        style,
    });
}
fn column(index: usize) -> String {
    let mut index = index + 1;
    let mut result = String::new();
    while index > 0 {
        index -= 1;
        result.insert(0, char::from(b'A' + (index % 26) as u8));
        index /= 26;
    }
    result
}
fn range(sheet: &str, col: usize, count: usize) -> String {
    let col = column(col);
    format!("{sheet}!${col}$3:${col}${}", count.max(1) + 2)
}
fn formula(
    kind: &str,
    key: &str,
    row: &Value,
    excel: usize,
    add: usize,
    sub: usize,
) -> Option<(String, Value)> {
    let n = |k| number(row, k);
    let avg = || {
        if n("quantity") != 0.0 {
            n("cost_basis") / n("quantity")
        } else {
            0.0
        }
    };
    let result = if matches!(kind, "acquisitions" | "disposals") {
        match key {
            "quantity" => (format!("=F{excel}/100000000000"),Value::Float(n("quantity"))),
            "unit_price" => (format!("=IF(G{excel}=0,0,H{excel}/G{excel})"),Value::Float(if n("quantity")!=0.0{n("fiat_value")/n("quantity")}else{0.0})),
            "gain_loss" if kind=="disposals" => (format!("=H{excel}-I{excel}"),Value::Float(n("proceeds")-n("cost_basis"))),
            "gain_check" => (format!("=IF(ABS(J{excel}-K{excel})<=Verify!$B$3,\"OK\",\"DIFF \"&TEXT(J{excel}-K{excel},\"0.00\"))"),s(if (n("proceeds")-n("cost_basis")-n("gain_loss")).abs()<=0.01{"OK"}else{"DIFF"})),
            _=>return None,
        }
    } else if kind == "asset_rows" {
        let a = |col| range("Acquisitions", col, add);
        let d = |col| range("Disposals", col, sub);
        let asset = format!("$A{excel}");
        match key {
            "holdings_qty" => (format!("=SUMIFS({},{},{asset},{},\"acquisition\")+SUMIFS({},{},{asset},{},\"transfer_in\")-SUMIFS({},{},{asset})+SUMIFS({},{},{asset},{},\"transfer_fee\")",a(6),a(3),a(4),a(6),a(3),a(4),d(6),d(3),d(6),d(3),d(4)),Value::Float(n("quantity"))),
            "cost_basis" => (format!("=SUMIFS({},{},{asset},{},\"acquisition\")+SUMIFS({},{},{asset},{},\"transfer_in\")-SUMIFS({},{},{asset})",a(7),a(3),a(4),a(7),a(3),a(4),d(8),d(3)),Value::Float(n("cost_basis"))),
            "avg_cost" => (format!("=IF(E{excel}=0,0,H{excel}/E{excel})"),Value::Float(avg())),
            "market_value" => (format!("=E{excel}*B{excel}"),Value::Float(n("market_value"))),
            "unrealized_pnl" => (format!("=N{excel}-H{excel}"),Value::Float(n("unrealized_pnl"))),
            "realized_gain" => (format!("=SUMIFS({},{},{asset},{},1)+SUMIFS({},{},{asset},{},1)",d(9),d(3),d(12),a(9),a(3),a(10)),Value::Float(n("realized_gain"))),
            _=>{
                let (left,right,original,target,tolerance) = match key {
                    "qty_check"=>("E","F","quantity","holdings_qty_kassiber","0.00000001"),
                    "basis_check"=>("H","I","cost_basis","cost_basis_kassiber","Verify!$B$3"),
                    "avg_check"=>("K","L","avg_cost","avg_cost_kassiber","Verify!$B$3"),
                    "mv_check"=>("N","O","market_value","market_value_kassiber","Verify!$B$3"),
                    "unrealized_check"=>("Q","R","unrealized_pnl","unrealized_kassiber","Verify!$B$3"),
                    "realized_check"=>("T","U","realized_gain","realized_gain_kassiber","Verify!$B$3"),
                    _=>return None,
                };
                let diff=if original=="avg_cost"{avg()}else{n(original)}-n(target);
                let tol=if key=="qty_check"{5e-9}else{0.01};
                (format!("=IF(ABS({left}{excel}-{right}{excel})<={tolerance},\"OK\",\"DIFF \"&TEXT({left}{excel}-{right}{excel},\"0.00000000\"))"),s(if diff.abs()<=tol{"OK".into()}else{format!("DIFF {diff:.8}")}))
            }
        }
    } else {
        return None;
    };
    Some(result)
}
fn conditional(
    ops: &mut Vec<Operation>,
    col: u16,
    count: usize,
    text: &str,
    format: &str,
    offset: usize,
) {
    ops.push(Operation::Contains {
        first_row: 2,
        first_col: col,
        last_row: count.max(1) as u32 + 1,
        last_col: col,
        text: text.into(),
        style: style(format, offset),
    });
}
fn data_sheet(
    kind: &str,
    name: &str,
    title: &str,
    rows: &[Value],
    add: usize,
    sub: usize,
    offset: usize,
) -> Result<Sheet> {
    let cols = metadata()["columns"][kind].as_array().unwrap();
    let mut ops = vec![
        Operation::Landscape,
        Operation::Fit {
            width: 1,
            height: 0,
        },
        Operation::Margins {
            left: 0.35,
            right: 0.35,
            top: 0.5,
            bottom: 0.5,
        },
    ];
    let widths: Vec<usize> = cols
        .iter()
        .map(|col| {
            let key = col["key"].as_str().unwrap();
            let fmt = col["fmt"].as_str().unwrap();
            let mut width = col["label"].as_str().unwrap().chars().count() + 2;
            for row in rows.iter().take(50) {
                let raw = get(row, key).cloned().unwrap_or_else(|| s(""));
                width = width.max(py_str(&raw).chars().count().min(36));
            }
            if [
                "quantity",
                "quantity_input",
                "formula_quantity",
                "kassiber_qty",
            ]
            .contains(&fmt)
            {
                width = width.max(14);
            }
            if ["money", "money_input", "formula_money", "kassiber"].contains(&fmt) {
                width = width.max(12);
            }
            (width + 1).clamp(9, 40)
        })
        .collect();
    ops.push(Operation::Row {
        row: 0,
        height: 28.0,
    });
    ops.push(Operation::Merge {
        first_row: 0,
        first_col: 0,
        last_row: 0,
        last_col: cols.len() as u16 - 1,
        text: title.into(),
        style: style("title", offset),
    });
    let header_lines = cols
        .iter()
        .enumerate()
        .map(|(i, c)| artifact_xlsx::lines(&s(c["label"].as_str().unwrap()), widths[i] as f64))
        .max()
        .unwrap_or(1);
    ops.push(Operation::Row {
        row: 1,
        height: (header_lines * 15 + 6).max(30) as f64,
    });
    for (i, col) in cols.iter().enumerate() {
        ops.push(Operation::Column {
            first: i as u16,
            last: i as u16,
            width: widths[i] as f64,
        });
        write(
            &mut ops,
            1,
            i as u16,
            Cell::String(col["label"].as_str().unwrap().into()),
            style("header", offset),
        );
    }
    ops.push(Operation::Freeze { row: 2, col: 0 });
    for (index, row) in rows.iter().enumerate() {
        let mut lines = 1;
        for (i, col) in cols.iter().enumerate() {
            let key = col["key"].as_str().unwrap();
            let fmt = col["fmt"].as_str().unwrap();
            let raw = get(row, key).cloned().unwrap_or_else(|| s(""));
            let mut format = style(fmt, offset);
            let cell =
                if let Some((expression, result)) = formula(kind, key, row, index + 3, add, sub) {
                    Cell::Formula {
                        expression,
                        result: py_str(&result),
                    }
                } else if col.get("link_key").is_some() {
                    let name = py_str(&raw);
                    lines = lines.max(artifact_xlsx::lines(&raw, widths[i] as f64));
                    let url = text(row, col["link_key"].as_str().unwrap());
                    if !url.is_empty() {
                        format = style("link", offset);
                        Cell::Link { url, text: name }
                    } else {
                        format = style("text", offset);
                        artifact_xlsx::cell(&s(name))?
                    }
                } else {
                    if ["text", "subheader"].contains(&fmt) {
                        lines = lines.max(artifact_xlsx::lines(&raw, widths[i] as f64));
                    }
                    artifact_xlsx::cell(&raw)?
                };
            write(&mut ops, index as u32 + 2, i as u16, cell, format);
        }
        ops.push(Operation::Row {
            row: index as u32 + 2,
            height: (lines * 15 + 4).max(18) as f64,
        });
    }
    if rows.is_empty() {
        write(
            &mut ops,
            2,
            0,
            Cell::String("No rows in scope.".into()),
            style("text", offset),
        );
    }
    ops.push(Operation::Autofilter {
        first_row: 1,
        first_col: 0,
        last_row: rows.len().max(1) as u32 + 1,
        last_col: cols.len() as u16 - 1,
    });
    if kind == "disposals" {
        conditional(&mut ops, 11, rows.len(), "DIFF", "diff", offset);
        conditional(&mut ops, 11, rows.len(), "OK", "ok", offset);
    }
    if matches!(kind, "acquisitions" | "disposals") {
        let col = if kind == "acquisitions" { 12 } else { 14 };
        for token in ["coarse", "missing"] {
            conditional(&mut ops, col, rows.len(), token, "coarse", offset);
        }
    }
    if kind == "asset_rows" {
        for col in [6, 9, 12, 15, 18, 21] {
            conditional(&mut ops, col, rows.len(), "DIFF", "diff", offset);
            conditional(&mut ops, col, rows.len(), "OK", "ok", offset);
        }
    }
    Ok(Sheet {
        name: name.into(),
        operations: ops,
    })
}
fn readme(
    profile: &Value,
    wallet: Option<&str>,
    generated: &str,
    control: usize,
    sub: usize,
    offset: usize,
) -> Result<Sheet> {
    let mut ops = vec![
        Operation::Column {
            first: 0,
            last: 0,
            width: 26.0,
        },
        Operation::Column {
            first: 1,
            last: 1,
            width: 78.0,
        },
        Operation::Margins {
            left: 0.4,
            right: 0.4,
            top: 0.5,
            bottom: 0.5,
        },
        Operation::Row {
            row: 0,
            height: 26.0,
        },
    ];
    for (r, text, fmt) in [
        (0, "How to verify this report", "title"),
        (1, "Verification status", "subheader"),
        (2, "Check tolerance", "subheader"),
    ] {
        write(
            &mut ops,
            r,
            0,
            Cell::String(text.into()),
            style(fmt, offset),
        );
    }
    write(
        &mut ops,
        2,
        1,
        Cell::Number(0.01),
        style("tolerance", offset),
    );
    let method = text(profile, "gains_algorithm").to_uppercase();
    let method = if method.is_empty() {
        "(unset)".into()
    } else {
        method
    };
    let country = text(profile, "tax_country");
    let at = country.to_lowercase() == "at";
    let version = include_str!("../../kassiber/Cargo.toml")
        .lines()
        .find_map(|line| {
            line.strip_prefix("version = \"")
                .and_then(|v| v.strip_suffix('"'))
        })
        .unwrap();
    let meta = [
        ("Generated at", s(generated)),
        ("Kassiber version", s(version)),
        (
            "Lot-selection method",
            s(if at {
                format!("{method} (Austrian moving-average cost base)")
            } else {
                method.clone()
            }),
        ),
        ("Fiat currency", value(profile, "fiat_currency").clone()),
        ("Tax country", s(country.as_str())),
        (
            "Journals last processed",
            if truthy(value(profile, "last_processed_at")) {
                value(profile, "last_processed_at").clone()
            } else {
                s("")
            },
        ),
        (
            "Processed tx count",
            Value::Int(int(profile, "last_processed_tx_count").into()),
        ),
        ("Wallet scope", s(wallet.unwrap_or("All wallets"))),
    ];
    for (i, (label, v)) in meta.iter().enumerate() {
        let row = i as u32 + 3;
        ops.push(Operation::Row {
            row,
            height: (artifact_xlsx::lines(v, 74.0) * 15 + 4).max(18) as f64,
        });
        write(
            &mut ops,
            row,
            0,
            Cell::String((*label).into()),
            style("subheader", offset),
        );
        write(
            &mut ops,
            row,
            1,
            artifact_xlsx::cell(v)?,
            style("text", offset),
        );
    }
    let mut index = 12;
    for row in metadata()["readme"][if at { "at" } else { "" }]
        .as_array()
        .unwrap()
    {
        let raw = row[0].as_str().unwrap();
        if raw.contains("__WALLET__") && wallet.is_none() {
            continue;
        }
        let text = raw
            .replace("__METHOD__", &method)
            .replace("__WALLET__", wallet.unwrap_or(""));
        let fmt = row[1].as_str().unwrap();
        let height = if fmt == "subheader" {
            24
        } else {
            artifact_xlsx::lines(&s(text.as_str()), 104.0) * 15 + 6
        };
        ops.push(Operation::Row {
            row: index,
            height: height as f64,
        });
        ops.push(Operation::Merge {
            first_row: index,
            first_col: 0,
            last_row: index,
            last_col: 1,
            text,
            style: style(fmt, offset),
        });
        index += 1;
    }
    let mut counts: Vec<String> = [6, 9, 12, 15, 18, 21]
        .iter()
        .map(|col| {
            let r = range("Control", *col, control);
            format!("(COUNTA({r})-COUNTIF({r},\"OK\"))")
        })
        .collect();
    let r = range("Disposals", 11, sub);
    counts.push(format!("(COUNTA({r})-COUNTIF({r},\"OK\"))"));
    write(
        &mut ops,
        1,
        1,
        Cell::Formula {
            expression: format!(
                "=IF(({})=0,\"ALL CHECKS OK\",\"MISMATCH — see the highlighted check columns\")",
                counts.join("+")
            ),
            result: "ALL CHECKS OK".into(),
        },
        style("status_ok", offset),
    );
    ops.push(Operation::Contains {
        first_row: 1,
        first_col: 1,
        last_row: 1,
        last_col: 1,
        text: "MISMATCH".into(),
        style: style("diff", offset),
    });
    Ok(Sheet {
        name: "Verify".into(),
        operations: ops,
    })
}
/// The appendix in the shared design's palette: ink, grey, hairlines and
/// red only for a failed check. Input cells keep a neutral fill because the
/// README tells the reader to look for them.
pub(super) fn shared_style(mut style: Style) -> Style {
    const INK: &str = "#1F2328";
    const MUTED: &str = "#5B6169";
    match style.bg_color.as_deref() {
        // Headers: grey label over a rule, no fill.
        Some("#EFEFEF") => {
            style.bg_color = None;
            style.font_color = Some(MUTED.into());
            style.bottom = 1;
        }
        // Inputs.
        Some("#FFFDEB") => style.bg_color = Some("#F3F4F6".into()),
        // Formulas and passed checks read as plain ink.
        Some("#EAF4FF") | Some("#C6EFCE") => {
            style.bg_color = None;
            if style.font_color.is_some() {
                style.font_color = Some(INK.into());
            }
        }
        // A failed check is a blocker: red text, no fill.
        Some("#FFC7CE") => {
            style.bg_color = None;
            style.font_color = Some("#C8102E".into());
        }
        // A coarse check is a caveat, not a blocker.
        Some("#FFEB9C") => {
            style.bg_color = None;
            style.font_color = Some(MUTED.into());
        }
        _ => (),
    }
    if style.font_color.as_deref() == Some("#185FA5") {
        style.font_color = Some(INK.into());
    }
    style
}

pub(super) fn append(
    document: &mut Document,
    profile: &Value,
    wallet: Option<&str>,
    generated: &str,
    data: &Value,
) -> Result<()> {
    let offset = document.formats.len();
    let formats: Vec<Style> = serde_json::from_value(metadata()["formats"].clone()).unwrap();
    document
        .formats
        .extend(formats.into_iter().map(shared_style));
    let add = list(value(data, "acquisitions")).len();
    let sub = list(value(data, "disposals")).len();
    let assets = list(value(data, "asset_rows")).len();
    document
        .sheets
        .push(readme(profile, wallet, generated, assets, sub, offset)?);
    for (kind, name, title) in [
        (
            "acquisitions",
            "Acquisitions",
            "Acquisitions ledger (inputs add to holdings)",
        ),
        (
            "disposals",
            "Disposals",
            "Disposals ledger (inputs reduce holdings)",
        ),
        (
            "asset_rows",
            "Control",
            "Control — recompute every figure and compare to Kassiber",
        ),
        (
            "attachments",
            "Evidence",
            "Linked evidence — one clickable link per attachment",
        ),
        (
            "quarantines",
            "Quarantined",
            "Quarantined transactions (excluded from every figure above)",
        ),
    ] {
        let rows = list(value(data, kind));
        if matches!(kind, "attachments" | "quarantines") && rows.is_empty() {
            continue;
        }
        document
            .sheets
            .push(data_sheet(kind, name, title, rows, add, sub, offset)?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn verification_formula_checks_preserve_tolerances_and_cached_results() {
        let mut row = dict([
            ("quantity", Value::Float(1.0)),
            ("holdings_qty_kassiber", Value::Float(1.0 + 4e-9)),
            ("cost_basis", Value::Float(100.0)),
            ("cost_basis_kassiber", Value::Float(100.005)),
            ("avg_cost_kassiber", Value::Float(101.0)),
        ]);
        assert_eq!(
            formula("asset_rows", "qty_check", &row, 3, 0, 0).unwrap().1,
            s("OK")
        );
        replace(&mut row, "holdings_qty_kassiber", Value::Float(1.0 + 6e-9));
        assert_eq!(
            formula("asset_rows", "qty_check", &row, 3, 0, 0).unwrap().1,
            s("DIFF -0.00000001")
        );
        assert_eq!(
            formula("asset_rows", "basis_check", &row, 3, 0, 0)
                .unwrap()
                .1,
            s("OK")
        );
        replace(&mut row, "cost_basis_kassiber", Value::Float(100.02));
        assert_eq!(
            formula("asset_rows", "basis_check", &row, 3, 0, 0)
                .unwrap()
                .1,
            s("DIFF -0.02000000")
        );
        assert_eq!(
            formula("asset_rows", "avg_check", &row, 3, 0, 0).unwrap().1,
            s("DIFF -1.00000000")
        );
        assert_eq!(
            formula("acquisitions", "unit_price", &dict::<&str>([]), 3, 0, 0)
                .unwrap()
                .1,
            Value::Float(0.0)
        );
        assert_eq!(column(26), "AA");
        assert_eq!(range("Disposals", 11, 0), "Disposals!$L$3:$L$3");
    }
}
