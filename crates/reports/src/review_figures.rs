//! The per-year figures a review preview reads from a stored journal:
//! `_tax_summary_from_journal_entries` summed by year, and the Austrian
//! Kennzahl summary with its rows dated before the crypto Kennzahlen.
use super::*;
use std::collections::BTreeMap;

/// Realized gain/loss (`sell` rows) and income per tax year, summed from the
/// journal's exact amounts. (Python's review sent each bucket through a
/// `float` first, so an exact gain of 1.00499999999999999 read 1.005 and
/// rounded a cent above the forms; the review now agrees with them.)
pub fn year_totals(
    conn: &Connection,
    profile_id: &str,
    cut: crate::tax_years::Cut<'_>,
) -> Result<BTreeMap<i64, [Decimal; 2]>> {
    // A country that disposes of fee units counts their realized gain with
    // its sales, as its form does.
    let country = rows(
        conn,
        "SELECT tax_country FROM profiles WHERE id=?",
        [profile_id],
    )?
    .first()
    .map(|row| text(row, "tax_country"))
    .unwrap_or_default();
    let country = kassiber_pytext::lower(kassiber_pytext::strip(&country));
    let fees = if crate::country::realizes_fee_units(&country) {
        ",'fee','transfer_fee'"
    } else {
        ""
    };
    let query = format!("SELECT je.occurred_at,je.asset,je.entry_type,COALESCE(NULLIF(t.kind_override,''),t.kind) AS transaction_kind,COALESCE(je.capital_gains_type,'') AS capital_gains_type,ABS(je.quantity) AS quantity_msat,COALESCE(je.proceeds,0) AS proceeds,COALESCE(je.cost_basis,0) AS cost_basis,COALESCE(je.gain_loss,0) AS gain_loss,je.gain_loss_exact FROM journal_entries je LEFT JOIN transactions t ON t.id=je.transaction_id WHERE je.profile_id=? AND je.entry_type IN ('disposal','income'{fees}) AND COALESCE(t.taxability_override,1)!=0 AND COALESCE(je.at_category,'')!='neu_swap'");
    // Buckets keep the reference's insertion order; their sums round there.
    let mut buckets: Vec<(tax_summary::Key, Decimal)> = Vec::new();
    for row in rows(conn, &query, [profile_id])? {
        let Some(key) = tax_summary::entry_key(&row, cut)? else {
            continue;
        };
        decimal(value(&row, "proceeds"))?;
        decimal(value(&row, "cost_basis"))?;
        let gain = crate::layer_reads::exact(&row, "gain_loss")?;
        match buckets.iter_mut().find(|(k, _)| *k == key) {
            Some((_, sum)) => *sum = sum.add(&gain, &DECIMAL).map_err(arithmetic)?,
            None => buckets.push((key, gain)),
        }
    }
    let mut totals = BTreeMap::<i64, [Decimal; 2]>::new();
    for ((year, _, transaction_type, _), sum) in buckets {
        let gain = sum;
        let bucket = totals
            .entry(year)
            .or_insert_with(|| [Decimal::zero(), Decimal::zero()]);
        let index = usize::from(transaction_type != "sell");
        bucket[index] = bucket[index].add(&gain, &DECIMAL).map_err(arithmetic)?;
    }
    Ok(totals)
}

/// `build_austrian_kennzahl_summary` amounts in its code order (a year
/// before 2022 states only KZ 801) and `austrian_rows_before_crypto_kennzahlen`,
/// read through the same functions as the Reports screen, as the review does.
pub fn kennzahl_summary(
    conn: &Connection,
    profile: &Value,
    year: i64,
) -> Result<(Vec<(i64, kassiber_pyjson::BigInt)>, i64)> {
    let mut amounts = Vec::new();
    for row in austrian::kennzahl_summary(conn, profile, year)? {
        amounts.push((
            int(&row, "kennzahl"),
            austrian::py_int(value(&row, "amount_eur_cents"))?,
        ));
    }
    Ok((
        amounts,
        austrian::rows_before_crypto_kennzahlen(conn, profile, year)?,
    ))
}
