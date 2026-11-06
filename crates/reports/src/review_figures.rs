//! The per-year figures a review preview reads from a stored journal:
//! `_tax_summary_from_journal_entries` summed by year, and the Austrian
//! Kennzahl summary with its rows dated before the crypto Kennzahlen.
use super::*;
use std::collections::BTreeMap;

/// The E 1kv crypto Kennzahlen, which apply from tax year 2022 and to rows
/// from 1 March 2022 (Europe/Vienna); KZ 801 is unaffected.
const E1KV_KENNZAHLEN: [i64; 3] = [172, 174, 176];
const E1KV_FIRST_TAX_YEAR: i64 = 2022;
const CRYPTO_KENNZAHLEN_FROM: &str = "2022-03-01";
const KENNZAHL_ORDER: [i64; 4] = [172, 174, 176, 801];

/// Realized gain/loss (`sell` rows) and income per tax year, summed the way
/// the review does: each summary bucket crosses `float` first.
pub fn year_totals(
    conn: &Connection,
    profile_id: &str,
    vienna: bool,
) -> Result<BTreeMap<i64, [Decimal; 2]>> {
    let query = "SELECT je.occurred_at,je.asset,je.entry_type,COALESCE(NULLIF(t.kind_override,''),t.kind) AS transaction_kind,COALESCE(je.capital_gains_type,'') AS capital_gains_type,ABS(je.quantity) AS quantity_msat,COALESCE(je.proceeds,0) AS proceeds,COALESCE(je.cost_basis,0) AS cost_basis,COALESCE(je.gain_loss,0) AS gain_loss FROM journal_entries je LEFT JOIN transactions t ON t.id=je.transaction_id WHERE je.profile_id=? AND je.entry_type IN ('disposal','income') AND COALESCE(t.taxability_override,1)!=0 AND COALESCE(je.at_category,'')!='neu_swap'";
    // Buckets keep the reference's insertion order; their sums round there.
    let mut buckets: Vec<(tax_summary::Key, Decimal)> = Vec::new();
    for row in rows(conn, query, [profile_id])? {
        let Some(key) = tax_summary::entry_key(&row, vienna)? else {
            continue;
        };
        decimal(value(&row, "proceeds"))?;
        decimal(value(&row, "cost_basis"))?;
        let gain = decimal(value(&row, "gain_loss"))?;
        match buckets.iter_mut().find(|(k, _)| *k == key) {
            Some((_, sum)) => *sum = sum.add(&gain, &DECIMAL).map_err(arithmetic)?,
            None => buckets.push((key, gain)),
        }
    }
    let mut totals = BTreeMap::<i64, [Decimal; 2]>::new();
    for ((year, _, transaction_type, _), sum) in buckets {
        let gain = decimal(&float(&sum)?)?;
        let bucket = totals
            .entry(year)
            .or_insert_with(|| [Decimal::zero(), Decimal::zero()]);
        let index = usize::from(transaction_type != "sell");
        bucket[index] = bucket[index].add(&gain, &DECIMAL).map_err(arithmetic)?;
    }
    Ok(totals)
}

/// `build_austrian_kennzahl_summary` amounts in its code order (a year
/// before 2022 states only KZ 801) and `austrian_rows_before_crypto_kennzahlen`.
pub fn kennzahl_summary(
    conn: &Connection,
    profile: &Value,
    year: i64,
) -> Result<(Vec<(i64, Value)>, usize)> {
    austrian::validate(profile)?;
    let mut amounts: Vec<(i64, Value)> = KENNZAHL_ORDER
        .iter()
        .map(|code| (*code, Value::Int(0.into())))
        .collect();
    let mut before = 0;
    for row in austrian::e1kv_rows(conn, profile, year)? {
        let mapped = austrian::kennzahl_for_category(&text(&row, "at_category"));
        let stated = match value(&row, "kennzahl") {
            Value::Int(code) => i64::try_from(code.clone()).ok(),
            _ => None,
        }
        .filter(|code| {
            !(E1KV_KENNZAHLEN.contains(code)
                && text(&row, "date").as_str() < CRYPTO_KENNZAHLEN_FROM)
        });
        let Some(code) = stated else {
            if mapped.is_some_and(|code| E1KV_KENNZAHLEN.contains(&code)) {
                before += 1;
            }
            continue;
        };
        let Some((_, total)) = amounts.iter_mut().find(|(known, _)| *known == code) else {
            continue;
        };
        if let (Value::Int(total), Value::Int(amount)) =
            (total, value(&row, "form_amount_eur_cents"))
        {
            *total += amount;
        }
    }
    if year < E1KV_FIRST_TAX_YEAR {
        amounts.retain(|(code, _)| !E1KV_KENNZAHLEN.contains(code));
    }
    Ok((amounts, before))
}
