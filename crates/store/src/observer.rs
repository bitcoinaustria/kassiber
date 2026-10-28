//! Private, read-only observer projections. These rows must never be included
//! in public snapshots, AI context or diagnostics. No dependency sidecar exists.
use rusqlite::{Connection, OptionalExtension, Result};

pub struct Instance {
    pub observer_kind: String,
    pub chain: String,
    pub network: String,
    pub logical_wallet_id: String,
    pub source_wallet_id: String,
    pub source_key: String,
    pub state_version: i64,
    pub state_epoch: i64,
    pub state_json: String,
}

pub struct Coverage {
    pub branch_key: String,
    pub coverage_version: i64,
    pub highest_used: Option<i64>,
    pub scanned_to: i64,
    pub details_json: String,
}

/// Read instance, epoch and coverage in one snapshot, releasing the read
/// transaction before any caller starts network preparation. Never call inside
/// another transaction; the domain layer supplies its typed refusal first.
pub fn snapshot(conn: &Connection, observer_id: &str) -> Result<Option<(Instance, Vec<Coverage>)>> {
    snapshot_with(conn, observer_id, |_| Ok(()))
}

fn snapshot_with(
    conn: &Connection,
    observer_id: &str,
    after_instance: impl FnOnce(&Connection) -> Result<()>,
) -> Result<Option<(Instance, Vec<Coverage>)>> {
    let tx = conn.unchecked_transaction()?;
    let instance = tx.query_row(
        "SELECT observer_kind,chain,network,logical_wallet_id,source_wallet_id,source_key,state_version,state_epoch,state_json
         FROM chain_observer_instances WHERE id=?",
        [observer_id], |r| Ok(Instance {
            observer_kind:r.get(0)?,chain:r.get(1)?,network:r.get(2)?,logical_wallet_id:r.get(3)?,
            source_wallet_id:r.get(4)?,source_key:r.get(5)?,state_version:r.get(6)?,state_epoch:r.get(7)?,state_json:r.get(8)?,
        })
    ).optional()?;
    after_instance(&tx)?;
    let result = if let Some(instance) = instance {
        let coverage = tx.prepare("SELECT branch_key,coverage_version,highest_used,scanned_to,details_json FROM chain_observer_coverage WHERE observer_id=? ORDER BY branch_key")?
            .query_map([observer_id], |r| Ok(Coverage { branch_key:r.get(0)?,coverage_version:r.get(1)?,highest_used:r.get(2)?,scanned_to:r.get(3)?,details_json:r.get(4)? }))?
            .collect::<Result<Vec<_>>>()?;
        Some((instance, coverage))
    } else {
        None
    };
    tx.commit()?;
    Ok(result)
}

pub struct ObservedRow {
    pub external_id: Option<String>,
    pub wallet_id: String,
    pub direction: String,
    pub asset: String,
    pub amount: i64,
    pub fee: i64,
    pub amount_includes_fee: bool,
    pub raw_json: Option<String>,
    pub authority_version: Option<i64>,
    pub graph_hash: Option<String>,
    pub quantity_hash: Option<String>,
    pub observer_ids_json: Option<String>,
}

pub fn observed_rows(conn: &Connection, wallet_id: &str) -> Result<Vec<ObservedRow>> {
    conn.prepare(
        "SELECT tx.external_id,tx.wallet_id,tx.direction,tx.asset,tx.amount,tx.fee,tx.amount_includes_fee,tx.raw_json,
         proof.authority_version,proof.graph_hash,proof.quantity_hash,proof.observer_ids_json
         FROM transactions tx LEFT JOIN chain_observation_provenance proof ON proof.transaction_id=tx.id
         WHERE tx.wallet_id=? AND tx.excluded=0"
    )?.query_map([wallet_id], |r| Ok(ObservedRow {
        external_id:r.get(0)?,wallet_id:r.get(1)?,direction:r.get(2)?,asset:r.get(3)?,amount:r.get(4)?,fee:r.get(5)?,
        amount_includes_fee:r.get::<_,i64>(6)? != 0,raw_json:r.get(7)?,authority_version:r.get(8)?,graph_hash:r.get(9)?,
        quantity_hash:r.get(10)?,observer_ids_json:r.get(11)?,
    }))?.collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observer_snapshot_epoch_and_coverage_share_one_read_transaction_and_release_on_error() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("book.sqlite3");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL;
            CREATE TABLE chain_observer_instances(id TEXT,observer_kind TEXT,chain TEXT,network TEXT,logical_wallet_id TEXT,source_wallet_id TEXT,source_key TEXT,state_version INTEGER,state_epoch INTEGER,state_json TEXT);
            CREATE TABLE chain_observer_coverage(observer_id TEXT,branch_key TEXT,coverage_version INTEGER,highest_used INTEGER,scanned_to INTEGER,details_json TEXT);
            INSERT INTO chain_observer_instances VALUES('i','bdk','bitcoin','main','w','w','descriptor:default',1,7,'{}');
            INSERT INTO chain_observer_coverage VALUES('i','receive',2,NULL,20,'{}');").unwrap();
        let writer = Connection::open(&path).unwrap();
        let (instance,coverage) = snapshot_with(&conn,"i", |reading| {
            assert!(!reading.is_autocommit());
            writer.execute_batch("BEGIN IMMEDIATE;UPDATE chain_observer_instances SET state_epoch=8;UPDATE chain_observer_coverage SET scanned_to=40;COMMIT")?;
            Ok(())
        }).unwrap().unwrap();
        assert_eq!(instance.state_epoch, 7);
        assert_eq!(coverage[0].scanned_to, 20);
        assert!(conn.is_autocommit());
        let (instance, coverage) = snapshot(&conn, "i").unwrap().unwrap();
        assert_eq!(instance.state_epoch, 8);
        assert_eq!(coverage[0].scanned_to, 40);
        assert!(snapshot_with(&conn, "i", |_| Err(rusqlite::Error::InvalidQuery)).is_err());
        assert!(conn.is_autocommit());
    }
}
