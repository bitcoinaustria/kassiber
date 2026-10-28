//! Complete report entry listing; journals are produced by Python.
use super::*;
pub fn entries(conn: &Connection, profile: &Value) -> Result<Value> {
    let mut result=rows(conn,"SELECT je.id,je.occurred_at,w.label AS wallet,COALESCE(a.code,'') AS account,je.entry_type,je.asset,je.quantity,je.fiat_value,COALESCE(je.cost_basis,0) AS cost_basis,COALESCE(je.proceeds,0) AS proceeds,COALESCE(je.gain_loss,0) AS gain_loss,COALESCE(je.description,'') AS description,je.at_category,je.at_kennzahl FROM journal_entries je JOIN wallets w ON w.id=je.wallet_id LEFT JOIN accounts a ON a.id=je.account_id WHERE je.profile_id=? ORDER BY je.occurred_at DESC,je.created_at DESC,je.id DESC",[text(profile,"id")])?;
    for row in &mut result {
        let quantity = int(row, "quantity");
        replace(row, "quantity_msat", Value::Int(quantity.into()));
        replace(row, "quantity", float(&btc(quantity)?)?);
        if let Value::Dict(fields) = row {
            fields.retain(|(k, v)| {
                !(["at_category".into(), "at_kennzahl".into()].contains(k)
                    && matches!(v, Value::Null))
            });
        }
    }
    Ok(Value::List(result))
}
