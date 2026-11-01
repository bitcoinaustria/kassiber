//! Reviewed conversion audit metadata, after finalized row selection.
//! This pass cannot inspect or establish quarantine dependency origins.
use super::row::Fields;
use super::*;
use std::collections::BTreeMap;

fn candidates<'a, R: Fields>(
    rows: &[&'a R],
    amount: &Value,
    direction: &str,
) -> Result<Vec<&'a R>, String> {
    let mut selected = Vec::new();
    for row in rows {
        // Python parses amounts before filtering directions, and parses the
        // reviewed amount only when a candidate exists. Keep both lazy gates.
        if !amount.is_null() && amount != "" {
            let actual = integer_big(row.field("amount"))?;
            let reviewed = BigInt::from_str(&crate::interpretation::integer(amount)?.to_string())
                .map_err(|_| "ValueError".to_owned())?;
            if actual != reviewed {
                continue;
            }
        }
        selected.push(*row);
    }
    Ok(selected
        .into_iter()
        .filter(|row| row.field("direction") == direction)
        .collect())
}

pub(super) fn evaluate(value: &Value) -> Result<Value, String> {
    evaluate_rows(
        value["rows"].as_array().unwrap(),
        value["pairs"].as_array().unwrap().clone(),
        value["reviewed"].as_array().unwrap(),
    )
    .map(Value::Array)
}

pub(super) fn evaluate_rows<R: Fields>(
    rows: &[R],
    mut pairs: Vec<Value>,
    reviewed: &[Value],
) -> Result<Vec<Value>, String> {
    if reviewed.is_empty() {
        return Ok(pairs);
    }
    let mut existing: BTreeSet<String> = pairs.iter().map(|p| text(&p["pair_id"])).collect();
    let mut by_anchor: BTreeMap<String, Vec<&R>> = BTreeMap::new();
    for row in rows {
        let anchor = text(row.field("journal_transaction_id"));
        if !anchor.is_empty() {
            by_anchor.entry(anchor).or_default().push(row);
        }
    }
    for pair in reviewed {
        let id = text(&pair["pair_id"]);
        if id.is_empty() || existing.contains(&id) {
            continue;
        }
        let out_id = text(&pair["out_id"]);
        let in_id = text(&pair["in_id"]);
        let out = candidates(
            by_anchor.get(&out_id).map(Vec::as_slice).unwrap_or(&[]),
            &pair["out_amount"],
            "outbound",
        )?;
        let input = candidates(
            by_anchor.get(&in_id).map(Vec::as_slice).unwrap_or(&[]),
            &pair["in_amount"],
            "inbound",
        )?;
        if out.len() != 1 || input.len() != 1 {
            continue;
        }
        let out = out[0];
        let input = input[0];
        let required = |row: &R| row.get("id").cloned().ok_or_else(|| "KeyError".to_owned());
        pairs.push(json!({
            "pair_id":id,"component_id":pair["component_id"],
            "out_id":required(out)?,"in_id":required(input)?,
            "out_transaction_id":out_id,"in_transaction_id":in_id,
            "out_asset":if codec::truth(&pair["out_asset"]) { &pair["out_asset"] } else { out.field("asset") },
            "in_asset":if codec::truth(&pair["in_asset"]) { &pair["in_asset"] } else { input.field("asset") },
            "policy":if codec::truth(&pair["policy"]) { pair["policy"].clone() } else { json!("taxable") },
            "kind":if codec::truth(&pair["kind"]) { pair["kind"].clone() } else { json!("swap") }
        }));
        existing.insert(id);
    }
    Ok(pairs)
}
