//! Private observer storage and projections. These rows must never be included
//! in public snapshots, AI context or diagnostics. No dependency sidecar exists.
use rusqlite::{Connection, OptionalExtension, Result};

/// Already validated private observer state. The caller owns the wallet
/// transaction, timestamp and compare-and-swap epoch; this function never
/// begins or commits a transaction.
pub struct Replacement<'a> {
    pub id: &'a str,
    pub workspace_id: &'a str,
    pub profile_id: &'a str,
    pub instance: &'a Instance,
    pub coverage: &'a [Coverage],
    pub expected_epoch: Option<i64>,
    pub timestamp: &'a str,
}

/// Returns false on an epoch conflict, before touching coverage. Errors and
/// conflicts must roll back the surrounding wallet transaction.
pub fn replace(conn: &Connection, replacement: &Replacement<'_>) -> Result<bool> {
    let Replacement {
        id,
        workspace_id,
        profile_id,
        instance: state,
        coverage,
        expected_epoch,
        timestamp,
    } = replacement;
    let changed = if let Some(epoch) = expected_epoch {
        conn.execute(
            "UPDATE chain_observer_instances SET workspace_id=?1,profile_id=?2,
             logical_wallet_id=?3,source_wallet_id=?4,source_key=?5,observer_kind=?6,
             chain=?7,network=?8,state_version=?9,state_json=?10,
             state_epoch=state_epoch+1,updated_at=?11 WHERE id=?12 AND state_epoch=?13",
            rusqlite::params![
                workspace_id,
                profile_id,
                state.logical_wallet_id,
                state.source_wallet_id,
                state.source_key,
                state.observer_kind,
                state.chain,
                state.network,
                state.state_version,
                state.state_json,
                timestamp,
                id,
                epoch
            ],
        )?
    } else {
        conn.execute(
            "INSERT INTO chain_observer_instances(id,workspace_id,profile_id,logical_wallet_id,
             source_wallet_id,source_key,observer_kind,chain,network,state_version,state_epoch,
             state_json,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,1,?11,?12,?12)
             ON CONFLICT(id) DO NOTHING",
            rusqlite::params![
                id,
                workspace_id,
                profile_id,
                state.logical_wallet_id,
                state.source_wallet_id,
                state.source_key,
                state.observer_kind,
                state.chain,
                state.network,
                state.state_version,
                state.state_json,
                timestamp
            ],
        )?
    };
    if changed != 1 {
        return Ok(false);
    }
    conn.execute(
        "DELETE FROM chain_observer_coverage WHERE observer_id=?",
        [id],
    )?;
    for point in *coverage {
        conn.execute(
            "INSERT INTO chain_observer_coverage(observer_id,branch_key,coverage_version,
             highest_used,scanned_to,details_json,updated_at) VALUES(?,?,?,?,?,?,?)",
            rusqlite::params![
                id,
                point.branch_key,
                point.coverage_version,
                point.highest_used,
                point.scanned_to,
                point.details_json,
                timestamp
            ],
        )?;
    }
    Ok(true)
}

/// One opaque dependency value (LWK's `ForeignStore` bytes). Never decoded,
/// logged or projected; only the observer that wrote it reads it back.
pub struct Value {
    pub namespace_version: i64,
    pub key: String,
    pub value: Vec<u8>,
}

/// `load_observer_values`: every value of one observer, by key.
pub fn values(conn: &Connection, observer_id: &str) -> Result<Vec<Value>> {
    conn.prepare(
        "SELECT namespace_version,key,value FROM chain_observer_values WHERE observer_id=? ORDER BY key",
    )?
    .query_map([observer_id], |r| {
        Ok(Value {
            namespace_version: r.get(0)?,
            key: r.get(1)?,
            value: r.get(2)?,
        })
    })?
    .collect()
}

/// `persist_observer_values`: replace every value of one observer inside the
/// caller's wallet savepoint. The caller validates keys and owns the commit.
pub fn replace_values(
    conn: &Connection,
    observer_id: &str,
    namespace_version: i64,
    values: &[(&str, &[u8])],
    timestamp: &str,
) -> Result<()> {
    conn.execute(
        "DELETE FROM chain_observer_values WHERE observer_id=?",
        [observer_id],
    )?;
    let mut insert = conn.prepare(
        "INSERT INTO chain_observer_values(observer_id,namespace_version,key,value,updated_at)
         VALUES(?,?,?,?,?)",
    )?;
    for (key, value) in values {
        insert.execute(rusqlite::params![
            observer_id,
            namespace_version,
            key,
            value,
            timestamp
        ])?;
    }
    Ok(())
}

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
