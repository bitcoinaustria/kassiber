use super::sources;
use crate::{
    publication::row::{field, first, or_text, py_eq, Result},
    scan::Cancellation,
};
use kassiber_envelope::{dict, s, truthy, AppError};
use kassiber_pyjson::{Key, Value};
use kassiber_store::rusqlite::Connection;
use serde_json::Value as Json;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

mod lookup;
pub mod public;
mod references;
mod worker;

#[cfg(feature = "test-harness")]
pub use worker::fixture_followup;
pub use worker::followup;

/// `load_local_transaction_reference`: the most complete compatible local
/// graph for a txid, or a conflict when local observations disagree.
pub enum LocalReference {
    Conflict,
    Payload(Json),
    Absent,
}

/// Local transaction references for the desktop graph read; no request.
pub fn local_reference(
    conn: &Connection,
    profile: &str,
    chain: &str,
    network: &str,
    txid: &str,
    current: &Json,
) -> Result<LocalReference> {
    Ok(
        match references::local(conn, profile, chain, network, txid, current)? {
            references::Local::Conflict => LocalReference::Conflict,
            references::Local::Payload(raw) => LocalReference::Payload(raw),
            references::Local::Absent => LocalReference::Absent,
        },
    )
}

/// `_load_graph_lookup_cache`: a previously fetched public reference graph.
pub fn cached_reference(
    conn: &Connection,
    chain: &str,
    network: &str,
    txid: &str,
) -> Result<Option<Json>> {
    references::cached(conn, chain, network, txid)
}

pub const MAX_ROWS: usize = 1_000;
pub const MAX_TRANSACTIONS: usize = 50;
pub const MAX_PREVOUTS: usize = 250;
pub const MAX_RUN_SECONDS: u64 = 30;

pub const MAX_PENDING: usize = 128;

pub struct Grant {
    database: String,
    memory_connection: Option<usize>,
    database_identity: Option<Value>,
    context: Value,
    environment: Value,
    profile: String,
    wallet_id: String,
    wallet_signature: Value,
    backend_selection: String,
    backend: Json,
    stored_backend: Option<Value>,
    job_id: String,
    source_key: String,
    revoked: Arc<AtomicBool>,
    cancel: Cancellation,
}

struct Published {
    request_id: String,
    grant: Grant,
}

#[derive(Default)]
pub struct Pending {
    entries: Vec<Published>,
}

impl Pending {
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn publish(&mut self, request_id: &str, grants: Vec<Grant>) {
        for grant in grants {
            self.entries.retain(|old| !old.grant.same_wallet(&grant));
            if self.entries.len() == MAX_PENDING {
                self.entries.remove(0);
            }
            self.entries.push(Published {
                request_id: request_id.into(),
                grant,
            });
        }
    }

    pub fn revoke_request(&mut self, request_id: &str) -> bool {
        let found = self.entries.iter().any(|v| v.request_id == request_id);
        for entry in self.entries.iter().filter(|v| v.request_id == request_id) {
            entry.grant.revoke();
        }
        self.entries.retain(|v| v.request_id != request_id);
        found
    }

    pub fn clear(&mut self) {
        for entry in &self.entries {
            entry.grant.revoke();
        }
        self.entries.clear();
    }

    pub fn take(&mut self) -> Option<(String, Grant)> {
        if self.entries.is_empty() {
            None
        } else {
            let entry = self.entries.remove(0);
            Some((entry.request_id, entry.grant))
        }
    }
}

fn ended() -> AppError {
    AppError::new("cancelled", "Graph reference sync grant ended")
}

fn database_identity(conn: &Connection) -> Result<Option<Value>> {
    first(
        conn,
        "SELECT value FROM settings WHERE key='database_instance_id'",
        [],
    )
}

fn signature(wallet: &Value) -> Result<Value> {
    let mut config =
        kassiber_pytext::loads(&or_text(field(wallet, "config_json"))).map_err(|_| ended())?;
    let Value::Dict(pairs) = &mut config else {
        return Err(ended());
    };
    pairs.retain(|(key, _)| *key != Key::from("last_synced_at"));
    Ok(dict([
        ("kind", field(wallet, "kind").clone()),
        ("config", config),
    ]))
}

fn stored_backend(conn: &Connection, backend: &Json) -> Result<Option<Value>> {
    first(
        conn,
        "SELECT * FROM backends WHERE lower(name)=?",
        [backend["name"].as_str().unwrap_or("")],
    )
}

impl Grant {
    pub fn cancellation(&self) -> Cancellation {
        self.cancel.clone()
    }

    /// The profile (book) this grant was issued for.
    pub fn profile(&self) -> &str {
        &self.profile
    }

    #[cfg(feature = "test-harness")]
    pub fn fixture(
        conn: &Connection,
        job_id: &str,
        wallet_id: &str,
        backend: &Json,
    ) -> Result<Self> {
        let job =
            first(conn, "SELECT * FROM freshness_jobs WHERE id=?", [job_id])?.ok_or_else(ended)?;
        let wallet =
            first(conn, "SELECT * FROM wallets WHERE id=?", [wallet_id])?.ok_or_else(ended)?;
        Self::capture(
            conn,
            &job,
            &wallet,
            backend,
            Arc::new(AtomicBool::new(false)),
            Cancellation::default(),
        )
    }

    pub fn revoke(&self) {
        self.revoked.store(true, Ordering::SeqCst);
        self.cancel.cancel();
    }

    pub(super) fn capture(
        conn: &Connection,
        job: &Value,
        wallet: &Value,
        backend: &Json,
        revoked: Arc<AtomicBool>,
        cancel: Cancellation,
    ) -> Result<Self> {
        let config = sources::wallet_config(wallet)?;
        let profile = or_text(field(job, "profile_id"));
        let context = kassiber_setup::context::snapshot(conn)?;
        if field(&context, "profile_id") != &s(profile.as_str()) {
            return Err(ended());
        }
        Ok(Self {
            database: conn.path().unwrap_or("").into(),
            memory_connection: conn
                .path()
                .filter(|path| !path.is_empty())
                .is_none()
                .then_some(std::ptr::from_ref(conn).addr()),
            database_identity: database_identity(conn)?,
            context,
            environment: kassiber_setup::networks::resolve(conn, &profile)?,
            profile,
            wallet_id: or_text(field(wallet, "id")),
            wallet_signature: signature(wallet)?,
            backend_selection: config["backend"].as_str().unwrap_or("").into(),
            backend: backend.clone(),
            stored_backend: stored_backend(conn, backend)?,
            job_id: or_text(field(job, "id")),
            source_key: or_text(field(job, "source_key")),
            revoked,
            cancel,
        })
    }

    pub fn same_wallet(&self, other: &Self) -> bool {
        self.database == other.database
            && self.memory_connection == other.memory_connection
            && self.database_identity == other.database_identity
            && self.profile == other.profile
            && self.wallet_id == other.wallet_id
    }

    pub fn validate(
        &self,
        conn: &Connection,
        mut backend: impl FnMut(&Connection, &str) -> Result<Json>,
    ) -> Result<()> {
        self.cancel.check(Instant::now() + Duration::from_secs(1))?;
        if self.revoked.load(Ordering::SeqCst)
            || conn.path().unwrap_or("") != self.database
            || self
                .memory_connection
                .is_some_and(|identity| identity != std::ptr::from_ref(conn).addr())
            || database_identity(conn)? != self.database_identity
            || kassiber_setup::context::snapshot(conn)? != self.context
            || !py_eq(
                &kassiber_setup::networks::resolve(conn, &self.profile)?,
                &self.environment,
            )
            || field(&self.context, "profile_id") != &s(self.profile.as_str())
        {
            return Err(ended());
        }
        let wallet = first(
            conn,
            "SELECT * FROM wallets WHERE id=? AND profile_id=?",
            [&self.wallet_id, &self.profile],
        )?
        .ok_or_else(ended)?;
        if !py_eq(&signature(&wallet)?, &self.wallet_signature)
            || backend(conn, &self.backend_selection)? != self.backend
            || stored_backend(conn, &self.backend)? != self.stored_backend
        {
            return Err(ended());
        }
        let job = first(
            conn,
            "SELECT * FROM freshness_jobs WHERE id=?",
            [&self.job_id],
        )?
        .ok_or_else(ended)?;
        let source = first(
            conn,
            "SELECT paused FROM freshness_source_states WHERE profile_id=? AND source_key=?",
            [&self.profile, &self.source_key],
        )?
        .ok_or_else(ended)?;
        if field(&job, "profile_id") != &s(self.profile.as_str())
            || field(&job, "source_key") != &s(self.source_key.as_str())
            || field(&job, "job_type") != &s("onchain_wallet_history")
            || field(&job, "status") != &s("done")
            || truthy(field(&job, "cancel_requested"))
            || truthy(field(&source, "paused"))
        {
            return Err(ended());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        freshness::Queue,
        publication::{testbook, wallet::SystemSources},
    };

    fn prepared() -> (Box<Connection>, Grant, Json) {
        let cases: Json = serde_json::from_str(include_str!(
            "../../tests/vectors/python_wallet_publication.json"
        ))
        .unwrap();
        let case = cases["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["name"] == "receive-change/esplora/0")
            .unwrap();
        let conn = Box::new(testbook::load(
            &kassiber_pytext::loads(&case["before"].to_string()).unwrap(),
        ));
        conn.execute_batch(
            "INSERT OR REPLACE INTO settings VALUES('context_workspace','w');
             INSERT OR REPLACE INTO settings VALUES('context_profile','p');",
        )
        .unwrap();
        let backend = case["fetch"]["backend"].clone();
        let mut sources = SystemSources::default();
        let job = Queue::new(&conn, &mut sources)
            .enqueue(&dict([
                ("profile_id", s("p")),
                ("source_key", s("onchain_wallet:a")),
                ("source_type", s("onchain_wallet")),
                ("job_type", s("onchain_wallet_history")),
            ]))
            .unwrap();
        conn.execute("UPDATE freshness_jobs SET status='done'", [])
            .unwrap();
        let wallet = first(&conn, "SELECT * FROM wallets WHERE id='a'", [])
            .unwrap()
            .unwrap();
        let grant = Grant::capture(
            &conn,
            &job,
            &wallet,
            &backend,
            Arc::new(AtomicBool::new(false)),
            Cancellation::default(),
        )
        .unwrap();
        (conn, grant, backend)
    }

    #[test]
    fn graph_grant_checks_are_local_and_leave_the_book_unchanged() {
        let (conn, grant, backend) = prepared();
        let before = testbook::snapshot(&conn);
        grant.validate(&conn, |_, _| Ok(backend.clone())).unwrap();
        assert_eq!(before, testbook::snapshot(&conn));
        assert!(grant.same_wallet(&grant));
    }

    #[test]
    fn graph_grant_rejects_changed_authority_and_configuration() {
        for sql in [
            "UPDATE settings SET value='other' WHERE key='context_profile'",
            "INSERT OR REPLACE INTO settings VALUES('database_instance_id','other')",
            "UPDATE wallets SET kind='file' WHERE id='a'",
            "UPDATE wallets SET config_json='{}' WHERE id='a'",
            "UPDATE wallets SET config_json='invalid' WHERE id='a'",
            "UPDATE freshness_jobs SET status='running'",
            "UPDATE freshness_jobs SET cancel_requested=1",
            "UPDATE freshness_jobs SET source_key='other'",
            "UPDATE freshness_jobs SET job_type='wallet_graph_references'",
            "DELETE FROM freshness_jobs",
            "UPDATE freshness_source_states SET paused=1",
            "DELETE FROM freshness_source_states",
            "INSERT INTO book_network_bindings(profile_id,environment_id,environment,domains_json,acknowledgements_json,inventory_digest,created_at) VALUES('p','new','test','[]','[]','new','now')",
            "INSERT INTO backends(name,kind,url,created_at,updated_at) VALUES('fixture','esplora','http://127.0.0.1:2','now','now')",
        ] {
            let (conn, grant, backend) = prepared();
            conn.execute_batch(sql).unwrap();
            assert_eq!(
                grant.validate(&conn, |_, _| Ok(backend.clone())).unwrap_err().code,
                "cancelled",
                "{sql}"
            );
        }
        let (conn, grant, mut backend) = prepared();
        backend["url"] = Json::String("http://127.0.0.1:1".into());
        assert_eq!(
            grant
                .validate(&conn, |_, _| Ok(backend.clone()))
                .unwrap_err()
                .code,
            "cancelled"
        );
        let (_conn, grant, backend) = prepared();
        let other = Connection::open_in_memory().unwrap();
        assert_eq!(
            grant
                .validate(&other, |_, _| Ok(backend.clone()))
                .unwrap_err()
                .code,
            "cancelled"
        );
    }

    #[test]
    fn graph_grant_ignores_only_sync_time_and_json_key_order() {
        let (conn, grant, backend) = prepared();
        let config: String = conn
            .query_row("SELECT config_json FROM wallets WHERE id='a'", [], |row| {
                row.get(0)
            })
            .unwrap();
        let mut config: Json = serde_json::from_str(&config).unwrap();
        config["last_synced_at"] = Json::String("later".into());
        conn.execute(
            "UPDATE wallets SET config_json=? WHERE id='a'",
            [config.to_string()],
        )
        .unwrap();
        grant.validate(&conn, |_, _| Ok(backend.clone())).unwrap();
        config["backend"] = Json::String("changed".into());
        conn.execute(
            "UPDATE wallets SET config_json=? WHERE id='a'",
            [config.to_string()],
        )
        .unwrap();
        assert!(grant.validate(&conn, |_, _| Ok(backend.clone())).is_err());
    }

    #[test]
    fn graph_grant_retains_request_cancellation_and_revocation_after_handoff() {
        let (conn, grant, backend) = prepared();
        grant.cancel.cancel();
        assert_eq!(
            grant
                .validate(&conn, |_, _| Ok(backend.clone()))
                .unwrap_err()
                .code,
            "cancelled"
        );
        let (conn, grant, backend) = prepared();
        grant.revoked.store(true, Ordering::SeqCst);
        assert_eq!(
            grant
                .validate(&conn, |_, _| Ok(backend.clone()))
                .unwrap_err()
                .code,
            "cancelled"
        );
    }

    #[test]
    fn graph_mailbox_bounds_replacement_and_request_revocation() {
        let (conn, original, backend) = prepared();
        let job = first(&conn, "SELECT * FROM freshness_jobs LIMIT 1", [])
            .unwrap()
            .unwrap();
        let wallet = first(&conn, "SELECT * FROM wallets WHERE id='a'", [])
            .unwrap()
            .unwrap();
        let make = |id: &str| {
            let mut grant = Grant::capture(
                &conn,
                &job,
                &wallet,
                &backend,
                Arc::new(AtomicBool::new(false)),
                Cancellation::default(),
            )
            .unwrap();
            grant.wallet_id = id.into();
            grant
        };
        let mut pending = Pending::default();
        pending.publish("original", vec![original]);
        pending.publish("replacement", vec![make("a")]);
        assert_eq!(pending.len(), 1);
        assert!(!pending.revoke_request("original"));
        assert_eq!(pending.len(), 1);
        assert!(pending.revoke_request("replacement"));
        assert!(pending.is_empty());
        for index in 0..MAX_PENDING + 2 {
            pending.publish(&index.to_string(), vec![make(&index.to_string())]);
        }
        assert_eq!(pending.len(), MAX_PENDING);
        assert!(!pending.revoke_request("0"));
        assert!(!pending.revoke_request("1"));
        assert!(pending.revoke_request("2"));
        let flag = Arc::clone(&pending.entries[0].grant.revoked);
        pending.clear();
        assert!(pending.is_empty());
        assert!(flag.load(Ordering::SeqCst));
    }
}
