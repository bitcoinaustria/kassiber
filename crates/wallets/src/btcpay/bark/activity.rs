//! Final Bark operations and provider text for the wallet's activity view.
//! These records do not enter journal inputs or balances.
use super::{btc_msat, field, row_id, text, unix_to_iso, Result};
use kassiber_chain::btcpay::normalize::parse_labels;
use kassiber_envelope::AppError;
use kassiber_pyjson::Value;
use kassiber_store::{
    optional::{self, BTCPAY_BARK_ACTIVITY},
    rusqlite::{params, Connection},
};
use sha2::{Digest, Sha256};

pub(crate) fn source_key(backend: &str, store_id: &str) -> String {
    // The wallet's saved BTCPay connection is stable across credential
    // rotation and transport changes. The label and URL are not published.
    let endpoint: String = Sha256::digest(backend.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("{endpoint}:{store_id}")
}

pub(crate) fn imported(conn: &Connection, wallet: &str, source: &str) -> Result<bool> {
    if !optional::present(conn, BTCPAY_BARK_ACTIVITY)
        .map_err(|e| AppError::new(e.code, e.message))?
    {
        return Ok(false);
    }
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM btcpay_bark_history_imports WHERE wallet_id=? AND source_key=?)",
        [wallet, source],
        |row| row.get(0),
    )
    .map_err(crate::database_error)
}

pub(crate) fn publish(conn: &Connection, wallet: &str, source: &str, rows: &[Value]) -> Result<()> {
    optional::ensure(conn, BTCPAY_BARK_ACTIVITY).map_err(|e| AppError::new(e.code, e.message))?;
    let mut upsert = conn
        .prepare_cached(
            "INSERT INTO btcpay_bark_activity
             (wallet_id, source_key, row_id, kind, occurred_at, amount_msat, fee_msat, has_txid, comment, labels_json)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(wallet_id, source_key, row_id) DO UPDATE SET
               kind=excluded.kind, occurred_at=excluded.occurred_at,
               amount_msat=excluded.amount_msat, fee_msat=excluded.fee_msat, has_txid=excluded.has_txid,
               comment=excluded.comment, labels_json=excluded.labels_json",
        )
        .map_err(crate::database_error)?;
    for row in rows {
        let id = row_id(row)?;
        let kind = text(field(row, "kind")).unwrap_or_default();
        let occurred_at = unix_to_iso(field(row, "timestamp"))?;
        let amount = btc_msat(field(row, "amount"), "amount", &id)?;
        let fee = match field(row, "fee") {
            Value::Null => None,
            value => Some(btc_msat(value, "fee", &id)?),
        };
        let comment = text(field(row, "comment")).map(|v| v.chars().take(2000).collect::<String>());
        let has_txid = text(field(row, "transactionHash"))
            .is_some_and(|value| value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()));
        let labels = match parse_labels(field(row, "labels")) {
            Value::List(labels) => labels
                .iter()
                .filter_map(text)
                .take(64)
                .map(|label| label.chars().take(200).collect::<String>())
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        let labels = serde_json::to_string(&labels).expect("label strings serialize");
        upsert
            .execute(params![
                wallet,
                source,
                id,
                kind,
                occurred_at,
                amount,
                fee,
                has_txid,
                comment,
                labels
            ])
            .map_err(crate::database_error)?;
    }
    conn.execute(
        "INSERT OR IGNORE INTO btcpay_bark_history_imports(wallet_id, source_key) VALUES (?, ?)",
        [wallet, source],
    )
    .map_err(crate::database_error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_visibility_does_not_create_a_booking() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE wallets(id TEXT PRIMARY KEY); INSERT INTO wallets VALUES('bark');",
        )
        .unwrap();
        let refresh = kassiber_pytext::loads(
            r#"{
            "id":"round-1","kind":"refresh","timestamp":1795000000,"amount":"0","fee":"0",
            "labels":{"Maintenance / auto":{"text":"Maintenance / auto","type":"raw"}}
        }"#,
        )
        .unwrap();
        assert!(super::super::records(&refresh, "shop").unwrap().is_empty());
        let source = source_key("shop-instance", "shop");
        assert!(!imported(&conn, "bark", &source).unwrap());
        publish(&conn, "bark", &source, &[refresh.clone()]).unwrap();
        publish(&conn, "bark", &source, &[refresh]).unwrap();
        assert!(imported(&conn, "bark", &source).unwrap());
        let saved: (i64, i64, String) = conn.query_row(
            "SELECT amount_msat, fee_msat, labels_json FROM btcpay_bark_activity WHERE row_id='round-1'",
            [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap();
        assert_eq!(saved, (0, 0, r#"["Maintenance / auto"]"#.into()));
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM btcpay_bark_activity", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            1
        );
    }
}
