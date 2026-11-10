//! Stored generic summaries and Vienna-year Austrian journal aggregation.
use super::*;
use std::collections::{BTreeMap, BTreeSet};
pub(crate) type Key = (i64, String, String, String);
const MONEY: [&str; 3] = ["proceeds", "cost_basis", "gain_loss"];
#[derive(Clone)]
struct Totals {
    quantity: Decimal,
    msat: Value,
    money: [Decimal; 3],
    assets: BTreeSet<String>,
}
impl Default for Totals {
    fn default() -> Self {
        Self {
            quantity: Decimal::zero(),
            msat: Value::Int(0.into()),
            money: std::array::from_fn(|_| Decimal::zero()),
            assets: BTreeSet::new(),
        }
    }
}
impl Totals {
    fn add(&mut self, row: &Value, quantity: Decimal) -> Result<()> {
        self.quantity = self.quantity.add(&quantity, &DECIMAL).map_err(arithmetic)?;
        if let (Value::Int(msat), Value::Int(n)) = (&mut self.msat, value(row, "quantity_msat")) {
            *msat += n;
        }
        for (i, key) in MONEY.iter().enumerate() {
            self.money[i] = self.money[i]
                .add(&decimal(value(row, key))?, &DECIMAL)
                .map_err(arithmetic)?;
        }
        self.assets.insert(text(row, "asset"));
        Ok(())
    }
    fn detail(&self, key: &Key) -> Result<Value> {
        Ok(dict([
            ("year", Value::Int(key.0.into())),
            ("asset", s(key.1.as_str())),
            ("transaction_type", s(key.2.as_str())),
            ("capital_gains_type", s(key.3.as_str())),
            ("quantity", float(&self.quantity)?),
            ("quantity_msat", self.msat.clone()),
            ("proceeds", float(&self.money[0])?),
            ("cost_basis", float(&self.money[1])?),
            ("gain_loss", float(&self.money[2])?),
        ]))
    }
    fn total(&self, kind: &str, year: Option<i64>) -> Result<Value> {
        let single = self.assets.len() == 1;
        Ok(dict([
            ("row_type", s(kind)),
            (
                "year",
                year.map(|n| Value::Int(n.into())).unwrap_or(Value::Null),
            ),
            (
                "asset",
                s(if single {
                    self.assets.first().unwrap().as_str()
                } else {
                    ""
                }),
            ),
            ("transaction_type", s("")),
            ("capital_gains_type", s("")),
            (
                "quantity",
                if single {
                    float(&self.quantity)?
                } else {
                    Value::Null
                },
            ),
            (
                "quantity_msat",
                if single {
                    austrian::integer(
                        &self
                            .quantity
                            .mul(&Decimal::from(100_000_000_000_i64), &DECIMAL)
                            .map_err(arithmetic)?
                            .quantize(0, &Context::new(32, Rounding::HalfUp))
                            .map_err(arithmetic)?,
                    )?
                } else {
                    Value::Null
                },
            ),
            ("proceeds", float(&self.money[0])?),
            ("cost_basis", float(&self.money[1])?),
            ("gain_loss", float(&self.money[2])?),
        ]))
    }
}
fn gains_type(row: &Value) -> String {
    let v = if truthy(value(row, "capital_gains_type")) {
        text(row, "capital_gains_type")
    } else {
        "short".into()
    };
    let v = kassiber_pytext::strip(&v).to_lowercase();
    if v.is_empty() {
        "short".into()
    } else {
        v
    }
}
fn income_kind(kind: &str) -> &'static str {
    match kassiber_pytext::strip(kind).to_lowercase().as_str() {
        "airdrop" => "airdrop",
        "hard_fork" | "hardfork" => "hardfork",
        "interest" | "lending_interest" => "interest",
        "mining" | "mining_reward" => "mining",
        "staking" => "staking",
        _ => "income",
    }
}
pub(crate) fn entry_key(row: &Value, zone: Option<&str>) -> Result<Option<Key>> {
    let at = text(row, "occurred_at");
    let year = if let Some(zone) = zone {
        calendar::local(&at, zone)?.0
    } else {
        let Some(year) = at.get(..4).and_then(|v| v.parse::<i64>().ok()) else {
            return Ok(None);
        };
        year
    };
    let asset = if truthy(value(row, "asset")) {
        text(row, "asset")
    } else {
        String::new()
    };
    if asset.is_empty() {
        return Ok(None);
    }
    Ok(Some((
        year,
        asset,
        if text(row, "entry_type") == "income" {
            income_kind(&text(row, "transaction_kind")).into()
        } else {
            "sell".into()
        },
        gains_type(row),
    )))
}
fn normalize(row: Value) -> Value {
    dict(
        [
            "row_type",
            "year",
            "asset",
            "transaction_type",
            "capital_gains_type",
            "quantity",
            "quantity_msat",
            "proceeds",
            "cost_basis",
            "gain_loss",
            "count",
            "total_swap_fee_msat",
            "total_swap_fee",
        ]
        .map(|k| (k, value(&row, k).clone())),
    )
}
pub fn report(conn: &Connection, profile: &Value) -> Result<Value> {
    let id = text(profile, "id");
    let country = text(profile, "tax_country").to_lowercase();
    let vienna = crate::country::vienna(&country);
    let zone = crate::country::zone(&country);
    let austrian = crate::country::austrian(&country);
    let query = "SELECT je.occurred_at,je.asset,je.entry_type,COALESCE(NULLIF(t.kind_override,''),t.kind) AS transaction_kind,COALESCE(je.capital_gains_type,'') AS capital_gains_type,ABS(je.quantity) AS quantity_msat,COALESCE(je.proceeds,0) AS proceeds,COALESCE(je.cost_basis,0) AS cost_basis,COALESCE(je.gain_loss,0) AS gain_loss FROM journal_entries je LEFT JOIN transactions t ON t.id=je.transaction_id WHERE je.profile_id=? AND ";
    let predicate = if austrian {
        "je.entry_type IN ('disposal','income') AND COALESCE(t.taxability_override,1)!=0 AND COALESCE(je.at_category,'')!='neu_swap'"
    } else {
        "((je.entry_type='disposal' AND je.at_category='neu_swap') OR (COALESCE(t.taxability_override,1)=0 AND je.entry_type IN ('disposal','income')))"
    };
    // Insertion order is Python's query order; sort only after Decimal buckets
    // cross the float boundary, exactly where the reference does.
    let mut groups: Vec<(Key, Totals)> = Vec::new();
    for row in rows(conn, &format!("{query}{predicate}"), [&id])? {
        if let Some(key) = entry_key(&row, zone)? {
            let index = groups
                .iter()
                .position(|(k, _)| *k == key)
                .unwrap_or_else(|| {
                    groups.push((key, Totals::default()));
                    groups.len() - 1
                });
            groups[index]
                .1
                .add(&row, btc(int(&row, "quantity_msat"))?)?;
        }
    }
    let has_adjustments = !groups.is_empty();
    let mut details = if vienna {
        groups
            .iter()
            .map(|(key, t)| t.detail(key))
            .collect::<Result<Vec<_>>>()?
    } else {
        let mut result = Vec::new();
        for mut row in rows(conn,"SELECT year,asset,transaction_type,capital_gains_type,quantity,proceeds,cost_basis,gain_loss FROM journal_tax_summary WHERE profile_id=?",[&id])? {
            let msat = int(&row,"quantity");
            replace(&mut row,"quantity",float(&btc(msat)?)?);
            replace(&mut row,"quantity_msat",Value::Int(msat.into()));
            if has_adjustments {
                let key = (int(&row,"year"),text(&row,"asset"),text(&row,"transaction_type").to_lowercase(),gains_type(&row));
                if let Some(index) = groups.iter().position(|(k,_)| *k == key) {
                    let (_,adjustment) = groups.remove(index);
                    let adjusted_quantity = float(&decimal(value(&row,"quantity"))?.sub(&adjustment.quantity,&DECIMAL).map_err(arithmetic)?)?;
                    replace(&mut row,"quantity",adjusted_quantity);
                    if let (Value::Int(a),Value::Int(b)) = (value(&row,"quantity_msat"),&adjustment.msat) {let n=a-b; replace(&mut row,"quantity_msat",Value::Int(n));}
                    for (i,key) in MONEY.iter().enumerate() {
                        let amount = decimal(value(&row,key))?.sub(&adjustment.money[i],&DECIMAL).map_err(arithmetic)?;
                        replace(&mut row,key,float(&amount)?);
                    }
                }
                if value(&row,"quantity_msat") == &Value::Int(0.into()) && MONEY.iter().all(|k| decimal(value(&row,k)).is_ok_and(|d| d.abs(&DECIMAL).unwrap() < "0.00000001".parse().unwrap())) {continue}
            }
            // The SELECT order differs from the public normalized key order.
            result.push(row);
        }
        result
    };
    details.sort_by_key(|r| {
        (
            int(r, "year"),
            text(r, "asset"),
            text(r, "transaction_type"),
            text(r, "capital_gains_type"),
        )
    });
    let fees = fees(conn, &id, zone)?;
    if details.is_empty() && fees.is_empty() {
        return Ok(Value::List(Vec::new()));
    }
    let mut per_year = BTreeMap::<i64, (Totals, Vec<Value>)>::new();
    let mut grand = Totals::default();
    for mut row in details {
        let quantity = decimal(value(&row, "quantity"))?;
        grand.add(&row, quantity.clone())?;
        let (totals, rows) = per_year.entry(int(&row, "year")).or_default();
        totals.add(&row, quantity)?;
        replace(&mut row, "row_type", s("detail"));
        rows.push(row);
    }
    let mut result = Vec::new();
    for (year, (totals, rows)) in per_year {
        result.extend(rows);
        result.push(totals.total("year_total", Some(year))?);
    }
    result.push(grand.total("grand_total", None)?);
    result.extend(fees);
    Ok(Value::List(result.into_iter().map(normalize).collect()))
}
fn fees(conn: &Connection, id: &str, zone: Option<&str>) -> Result<Vec<Value>> {
    let rows = rows(conn,"SELECT p.kind,p.policy,p.swap_fee_msat,p.out_asset,p.in_asset,COALESCE(p.target_occurred_at,t_out.occurred_at) AS occurred_at FROM journal_custody_projection_relations p JOIN transactions t_out ON t_out.id=p.out_transaction_id WHERE p.profile_id=? AND p.swap_fee_msat IS NOT NULL",[id])?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let mut years = BTreeMap::<i64, (i64, Value)>::new();
    let mut total = Value::Int(0.into());
    let mut count = 0;
    for row in rows {
        let normalize_asset = |key| {
            let raw = if truthy(value(&row, key)) {
                text(&row, key)
            } else {
                String::new()
            };
            let normalized = kassiber_pytext::strip(&raw).to_uppercase();
            if normalized == "L-BTC" {
                "LBTC".into()
            } else {
                normalized
            }
        };
        let same = normalize_asset("out_asset") == normalize_asset("in_asset");
        if same
            && ![
                "chain-swap",
                "peg-in",
                "peg-out",
                "reverse-submarine-swap",
                "submarine-swap",
                "swap-refund",
            ]
            .contains(
                &kassiber_pytext::strip(&text(&row, "kind"))
                    .to_lowercase()
                    .as_str(),
            )
        {
            continue;
        }
        let at = text(&row, "occurred_at");
        let year = if let Some(zone) = zone {
            calendar::local(&at, zone)?.0
        } else {
            let Some(year) = at.get(..4).and_then(|v| v.parse::<i64>().ok()) else {
                continue;
            };
            year
        };
        let (year_count, msat) = years
            .entry(year)
            .or_insert_with(|| (0, Value::Int(0.into())));
        if let (Value::Int(msat), Value::Int(total), Value::Int(n)) =
            (msat, &mut total, value(&row, "swap_fee_msat"))
        {
            *msat += n;
            *total += n;
        }
        *year_count += 1;
        count += 1;
    }
    let fee = |kind: &str, year: Option<i64>, count: i64, msat: Value| -> Result<Value> {
        Ok(dict([
            ("row_type", s(kind)),
            (
                "year",
                year.map(|n| Value::Int(n.into())).unwrap_or(Value::Null),
            ),
            ("count", Value::Int(count.into())),
            (
                "total_swap_fee",
                float(
                    &decimal(&msat)?
                        .div(&Decimal::from(100_000_000_000_i64), &DECIMAL)
                        .map_err(arithmetic)?,
                )?,
            ),
            ("total_swap_fee_msat", msat),
        ]))
    };
    let mut result = Vec::new();
    for (year, (count, msat)) in years {
        result.push(fee("swap_fees_year", Some(year), count, msat)?);
    }
    result.push(fee("swap_fees_total", None, count, total)?);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tax_summary_total_rounding() {
        for (quantity, expected) in [("0.000000000005", 1), ("-0.000000000005", -1)] {
            let mut total = Totals {
                quantity: quantity.parse().unwrap(),
                ..Default::default()
            };
            total.assets.insert("BTC".into());
            let row = total.total("year_total", Some(2025)).unwrap();
            assert_eq!(value(&row, "quantity_msat"), &Value::Int(expected.into()));
        }
        for raw in [Value::Null, s(""), s("  "), s(" SHORT ")] {
            assert_eq!(gains_type(&dict([("capital_gains_type", raw)])), "short");
        }
    }
}
