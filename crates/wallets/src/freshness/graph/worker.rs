use super::{lookup, references, Grant, MAX_PREVOUTS, MAX_ROWS, MAX_RUN_SECONDS, MAX_TRANSACTIONS};
use crate::{
    freshness::{
        execution::{self, Failure, Handler, Progress},
        Queue,
    },
    publication::{
        row::{field, first, or_text, rows, Result},
        wallet::Sources,
    },
    scan::Cancellation,
};
use kassiber_envelope::{dict, s, truthy};
use kassiber_net::{Consent, Gate};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::{params, Connection};
use serde_json::Value as Json;
use std::{
    cell::RefCell,
    time::{Duration, Instant},
};

struct Worker<'a, B> {
    grant: &'a Grant,
    gate: &'a Gate,
    stop: &'a Cancellation,
    backend: RefCell<B>,
    version: &'a str,
    clock: &'a dyn Fn() -> Instant,
}

impl<B: FnMut(&Connection, &str) -> Result<Json>> Worker<'_, B> {
    fn check(&self, conn: &Connection) -> Result<()> {
        self.stop.check(Instant::now() + Duration::from_secs(1))?;
        self.grant.validate(conn, &mut *self.backend.borrow_mut())?;
        let (backend, _) = lookup::route(&self.grant.backend)?;
        let consent = Consent::explicit_trigger(backend.request());
        self.gate
            .authorize(backend.request().clone(), Some(consent))
            .map_err(|e| e.app_error())?;
        Ok(())
    }
}

impl<B: FnMut(&Connection, &str) -> Result<Json>> Handler for Worker<'_, B> {
    fn supports(&self, kind: &str) -> bool {
        kind == "wallet_graph_references"
    }

    fn run(
        &mut self,
        conn: &mut Connection,
        job: &Value,
        progress: &mut Progress<'_>,
    ) -> std::result::Result<Value, Failure> {
        self.check(conn)?;
        progress.emit(conn, &dict([("phase", s("backend_fetch"))]))?;
        let state = Queue::new(conn, progress.sources)
            .state(&self.grant.profile, &or_text(field(job, "source_key")))?
            .unwrap_or(Value::Null);
        let after = field(field(&state, "checkpoint"), "after_rowid");
        let mut last_rowid = if matches!(after, Value::Null) {
            0
        } else {
            super::super::integer(after)?
        };
        let deadline = (self.clock)() + Duration::from_secs(MAX_RUN_SECONDS);
        let selected = rows(conn,"SELECT t.rowid AS scan_rowid,t.*,w.kind AS wallet_kind,w.config_json AS wallet_config_json
            FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE t.profile_id=? AND t.wallet_id=? AND t.rowid>?
            ORDER BY t.rowid LIMIT ?",params![self.grant.profile,self.grant.wallet_id,last_rowid,MAX_ROWS as i64])?;
        let now = progress.sources.now();
        let check = || {
            progress.check(conn)?;
            self.check(conn)?;
            if first(
                conn,
                "SELECT paused FROM freshness_source_states WHERE profile_id=? AND source_key=?",
                [&self.grant.profile, &or_text(field(job, "source_key"))],
            )?
            .is_some_and(|v| truthy(field(&v, "paused")))
            {
                return Err(super::ended());
            }
            Ok(())
        };
        let (backend, route) = lookup::route(&self.grant.backend)?;
        let consent = Consent::explicit_trigger(backend.request());
        let mut lookup = lookup::Lookup {
            conn,
            backend,
            route,
            gate: self.gate,
            consent,
            check: &check,
            version: self.version,
            network: "main".into(),
            remaining: MAX_PREVOUTS,
            requested: 0,
            refused: false,
        };
        let (mut scanned, mut attempted, mut cached, mut skipped, mut failed) = (0, 0, 0, 0, 0);
        let mut exhausted = true;
        for row in &selected {
            check()?;
            if attempted >= MAX_TRANSACTIONS || (self.clock)() >= deadline {
                exhausted = false;
                break;
            }
            let resume = last_rowid;
            last_rowid = super::super::integer(field(row, "scan_rowid"))?;
            scanned += 1;
            let row = references::json(row);
            let raw = references::object(&row["raw_json"]);
            let explicit_id = !raw["txid"].is_null()
                && raw["txid"]
                    .as_str()
                    .is_none_or(|id| !kassiber_pytext::strip(id).is_empty());
            let id = if explicit_id {
                references::txid(&raw["txid"])
            } else {
                references::txid(&row["external_id"])
            };
            let Some(id) = id else {
                skipped += 1;
                continue;
            };
            let Some((chain, network)) = references::scope(&row) else {
                skipped += 1;
                continue;
            };
            if chain != "bitcoin"
                || row["asset"]
                    .as_str()
                    .is_some_and(|v| !v.is_empty() && !v.eq_ignore_ascii_case("BTC"))
                || row["wallet_kind"]
                    .as_str()
                    .is_some_and(|v| v.to_ascii_lowercase().contains("liquid"))
                || ["vin", "vout"].iter().any(|key| {
                    raw[*key].as_array().is_some_and(|entries| {
                        entries.iter().any(|v| {
                            references::confidential(v) || references::confidential(&v["prevout"])
                        })
                    })
                })
            {
                skipped += 1;
                continue;
            }
            let eligible = match (raw["vin"].as_array(), raw["vout"].as_array()) {
                (Some(vin), Some(_)) => vin.iter().any(|v| {
                    v.is_object()
                        && v["prevout"]["value_state"] != "confidential"
                        && crate::bitcoin_records::value_sats(&v["prevout"])
                            .ok()
                            .flatten()
                            .is_none()
                }),
                _ => true,
            };
            if !eligible
                || lookup::locally_complete(conn, &row, &raw, &self.grant.profile, &network, &id)?
            {
                skipped += 1;
                continue;
            }
            let configured = &self.grant.backend;
            let config = references::object(&configured["config_json"]);
            let backend_chain = configured
                .get("chain")
                .or_else(|| configured["config"].get("chain"))
                .or_else(|| config.get("chain"));
            let backend_network = configured
                .get("network")
                .or_else(|| configured["config"].get("network"))
                .or_else(|| config.get("network"));
            let selected_chain = crate::descriptors::normalize_chain(
                backend_chain.and_then(Json::as_str).unwrap_or("bitcoin"),
            );
            let matches = selected_chain
                .as_ref()
                .is_ok_and(|selected| selected == &chain)
                && crate::descriptors::normalize_network(
                    &chain,
                    backend_network.and_then(Json::as_str).unwrap_or(""),
                )
                .ok()
                .as_ref()
                    == Some(&network);
            attempted += 1;
            let result = if matches {
                lookup.network = network;
                lookup.fetch(&id, &now)
            } else {
                Ok(false)
            };
            check()?;
            if lookup.refused {
                attempted -= 1;
                last_rowid = resume;
                exhausted = false;
                break;
            }
            if result.unwrap_or(false) {
                cached += 1;
            } else {
                failed += 1;
            }
        }
        if exhausted && selected.len() < MAX_ROWS {
            last_rowid = 0;
        }
        Ok(dict([
            ("scanned", Value::from(scanned)),
            ("attempted", Value::from(attempted)),
            ("cached", Value::from(cached)),
            ("skipped", Value::from(skipped)),
            ("failed", Value::from(failed)),
            ("prevouts_requested", Value::from(lookup.requested)),
            (
                "freshness_checkpoint",
                dict([("after_rowid", Value::from(last_rowid))]),
            ),
        ]))
    }
}

#[allow(clippy::too_many_arguments)]
pub fn followup(
    conn: &mut Connection,
    grant: Grant,
    gate: &Gate,
    stop: &Cancellation,
    sources: &mut dyn Sources,
    version: &str,
    backend: impl FnMut(&Connection, &str) -> Result<Json>,
) -> Result<Option<Value>> {
    run(
        conn,
        grant,
        gate,
        stop,
        sources,
        version,
        backend,
        &Instant::now,
    )
}

#[cfg(feature = "test-harness")]
#[allow(clippy::too_many_arguments)]
pub fn fixture_followup(
    conn: &mut Connection,
    grant: Grant,
    gate: &Gate,
    stop: &Cancellation,
    sources: &mut dyn Sources,
    version: &str,
    backend: impl FnMut(&Connection, &str) -> Result<Json>,
) -> Result<Option<Value>> {
    let now = Instant::now();
    run(conn, grant, gate, stop, sources, version, backend, &|| now)
}

#[allow(clippy::too_many_arguments)]
fn run(
    conn: &mut Connection,
    grant: Grant,
    gate: &Gate,
    stop: &Cancellation,
    sources: &mut dyn Sources,
    version: &str,
    backend: impl FnMut(&Connection, &str) -> Result<Json>,
    clock: &dyn Fn() -> Instant,
) -> Result<Option<Value>> {
    let mut worker = Worker {
        grant: &grant,
        gate,
        stop,
        backend: RefCell::new(backend),
        version,
        clock,
    };
    if worker.check(conn).is_err() {
        return Ok(None);
    }
    let key = format!("wallet_graph:{}", grant.wallet_id);
    let interrupted = rows(conn,"SELECT id FROM freshness_jobs WHERE profile_id=? AND source_key=? AND job_type='wallet_graph_references' AND status='running'",
        [&grant.profile,&key])?;
    for row in interrupted {
        let id = or_text(field(&row, "id"));
        Queue::new(conn, sources).cancel(&id)?;
        execution::run_job(
            conn,
            &id,
            sources,
            &mut worker,
            &mut |_| Ok(()),
            &execution::retry_time,
            false,
        )?;
    }
    worker.check(conn)?;
    execution::begin(conn)?;
    let job = Queue::new(conn, sources).enqueue(&dict([
        ("profile_id", s(grant.profile.as_str())),
        ("source_key", s(key)),
        ("source_type", s("onchain_wallet")),
        ("job_type", s("wallet_graph_references")),
        ("source_label", s("Transaction graph references")),
        ("priority", Value::from(200)),
    ]))?;
    execution::commit(conn)?;
    let completed = execution::run_requested(
        conn,
        &grant.profile,
        &[or_text(field(&job, "id"))],
        1,
        sources,
        &mut worker,
        &mut |_| Ok(()),
        &execution::retry_time,
    )?;
    Ok((!completed.is_empty()).then(|| {
        dict([
            ("profile", dict([("id", s(grant.profile))])),
            ("completed", Value::List(completed)),
        ])
    }))
}
