//! `chain_analysis_backfill.run`: one leased pass of an authorized recurring
//! source. Requests reserve lifetime quota durably before transport; a crash
//! can waste quota, never replenish it. Lease and revision fencing stop a
//! revoked or superseded grant at the next request or commit.
use super::super::routing::py_text;
use super::super::sources::{grant, is_txid, row, validate, MAX_RESPONSE_BYTES};
use super::super::*;
use super::legs::*;
use super::reader::{Quota, Reader};
use bitcoin::consensus::Decodable;
use bitcoin::hashes::{sha256d, Hash};
use kassiber_pyjson::BigInt;

fn fail(code: &str, message: &str) -> AppError {
    AppError::new(code, message)
}

fn commit(conn: &Connection) -> Result<()> {
    if !conn.is_autocommit() {
        conn.execute_batch("COMMIT").map_err(database_error)?;
    }
    Ok(())
}

fn small(value: &Value) -> i64 {
    match value {
        Value::Int(n) => i64::try_from(n.clone()).unwrap_or(i64::MAX),
        _ => 0,
    }
}

/// `_GrantBudget`: every request revalidates the lease and reserves quota.
struct GrantQuota<'a> {
    conn: &'a Connection,
    profile: &'a str,
    original: Value,
    token: String,
    runtime: &'a dyn Runtime,
    reserved: bool,
    response_bytes: i64,
}

impl Quota for GrantQuota<'_> {
    fn settle(&mut self) -> Result<()> {
        if self.reserved {
            let refund = (MAX_RESPONSE_BYTES - self.response_bytes).max(0);
            atomic(self.conn, || {
                self.conn
                    .execute(
                        "UPDATE chain_analysis_acquisition_grants SET bytes_used=bytes_used-? WHERE id=?",
                        params![refund, sql(field(&self.original, "id"), 2)?],
                    )
                    .map_err(database_error)?;
                Ok(())
            })?;
            commit(self.conn)?;
            self.reserved = false;
        }
        Ok(())
    }

    fn reserve(&mut self) -> Result<()> {
        let (conn, profile) = (self.conn, self.profile);
        let original = self.original.clone();
        let token = self.token.clone();
        let runtime = self.runtime;
        atomic(conn, || {
            let (live, _) = validate(conn, profile, &original, Some(&token), runtime)?;
            let spec = kassiber_pytext::loads(&py_text(field(&live, "spec_json")))
                .map_err(|error| AppError::new("internal_error", error.to_string()))?;
            if small(field(&live, "requests_used")) >= small(field(&spec, "max_requests"))
                || small(field(&live, "bytes_used")) + MAX_RESPONSE_BYTES
                    > small(field(&spec, "max_bytes"))
            {
                return Err(fail(
                    "acquisition_quota",
                    "Source lifetime quota reached; review a new authorization",
                ));
            }
            conn.execute(
                "UPDATE chain_analysis_acquisition_grants SET requests_used=requests_used+1,bytes_used=bytes_used+? WHERE id=?",
                params![MAX_RESPONSE_BYTES, sql(field(&original, "id"), 2)?],
            )
            .map_err(database_error)?;
            Ok(())
        })?;
        commit(conn)?;
        self.reserved = true;
        self.response_bytes = 0;
        Ok(())
    }

    fn received(&mut self, size: usize) -> Result<()> {
        self.response_bytes += size as i64;
        if self.runtime.cancelled() {
            return Err(fail("cancelled", "Acquisition cancelled"));
        }
        Ok(())
    }
}

fn hex(bytes: impl AsRef<[u8]>) -> String {
    bytes.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// `_clean_block`: parse with the consensus decoder; raw witnesses never
/// enter stored rows. Returns the parent hash and sanitized transactions.
/// The parent hash and each sanitized transaction with its block position.
type Block = (String, Vec<(usize, String, Value)>);

fn clean_block(raw_hex: &Value, expected: &str) -> Result<Block> {
    let Value::Str(text) = raw_hex else {
        return Err(fail("invalid_observation", "Invalid block bytes"));
    };
    let text = py_text(&Value::Str(text.clone()));
    if text.chars().count() > 8_000_000 {
        return Err(fail("invalid_observation", "Invalid block bytes"));
    }
    let structure = || fail("invalid_observation", "Invalid block structure");
    let bytes: Vec<u8> = bitcoin::hex::FromHex::from_hex(&text).map_err(|_| structure())?;
    if bytes.len() < 80 {
        return Err(fail(
            "invalid_observation",
            "Block hash does not match its header",
        ));
    }
    let header = &bytes[..80];
    let mut hash = sha256d::Hash::hash(header).to_byte_array();
    hash.reverse();
    if hex(hash) != expected {
        return Err(fail(
            "invalid_observation",
            "Block hash does not match its header",
        ));
    }
    let mut cursor = std::io::Cursor::new(&bytes[80..]);
    let count = bitcoin::VarInt::consensus_decode(&mut cursor)
        .map_err(|_| structure())?
        .0;
    if !(1..=100_000).contains(&count) {
        return Err(fail(
            "invalid_observation",
            "Invalid block transaction count",
        ));
    }
    let mut leaves = Vec::new();
    let mut transactions = Vec::new();
    for number in 0..count as usize {
        let tx = bitcoin::Transaction::consensus_decode(&mut cursor).map_err(|_| structure())?;
        let txid = tx.compute_txid();
        leaves.push(txid.to_byte_array());
        let txid = txid.to_string();
        let inputs: Vec<serde_json::Value> = tx
            .input
            .iter()
            .map(|input| {
                let mut entry = serde_json::json!({
                    "txid": input.previous_output.txid.to_string(), "vout": input.previous_output.vout,
                    "sequence": input.sequence.0, "scriptsig": hex(input.script_sig.as_bytes()),
                    "witness": input.witness.iter().map(hex).collect::<Vec<_>>(), "prevout": {}
                });
                if input.previous_output.is_null() {
                    entry["is_coinbase"] = serde_json::Value::Bool(true);
                }
                entry
            })
            .collect();
        let feature_raw = serde_json::json!({
            "version": tx.version.0 as u32, "locktime": tx.lock_time.to_consensus_u32(),
            "vin": inputs,
            "vout": tx.output.iter().map(|o| serde_json::json!({"value_sats": o.value.to_sat(), "scriptpubkey": hex(o.script_pubkey.as_bytes())})).collect::<Vec<_>>(),
            "txid": txid,
        });
        let raw = from_serde(&feature_raw);
        let mut clean = sanitize(&raw, "bitcoin", &txid);
        let features = crate::analysis_features::extract(
            &feature_raw,
            "local_acquisition",
            Some(&format!("tx:{txid}")),
            &[],
        );
        set(
            &mut clean,
            crate::analysis_features::PERSISTED_FEATURE_KEY,
            from_serde(&features),
        );
        if let (Value::List(originals), Some(Value::List(targets))) =
            (field(&raw, "vin"), get_mut(&mut clean, "vin"))
        {
            for (original, target) in originals.iter().zip(targets.iter_mut()) {
                if let Some(sequence) = get(original, "sequence") {
                    set(target, "sequence", sequence.clone());
                }
                if truthy(field(original, "is_coinbase")) {
                    set(target, "is_coinbase", Value::Bool(true));
                }
            }
        }
        transactions.push((number, txid, clean));
    }
    while leaves.len() > 1 {
        if leaves.len() % 2 == 1 {
            leaves.push(*leaves.last().unwrap());
        }
        leaves = leaves
            .chunks(2)
            .map(|pair| {
                let mut joined = pair[0].to_vec();
                joined.extend_from_slice(&pair[1]);
                sha256d::Hash::hash(&joined).to_byte_array()
            })
            .collect();
    }
    if leaves[0][..] != header[36..68] {
        return Err(fail(
            "invalid_observation",
            "Block transactions do not match the header commitment",
        ));
    }
    if (cursor.position() as usize) < bytes.len() - 80 {
        return Err(fail("invalid_observation", "Trailing block bytes"));
    }
    let mut parent = header[4..36].to_vec();
    parent.reverse();
    Ok((hex(parent), transactions))
}

fn get_mut<'v>(value: &'v mut Value, key: &str) -> Option<&'v mut Value> {
    match value {
        Value::Dict(pairs) => pairs
            .iter_mut()
            .rev()
            .find(|(k, _)| k == &kassiber_pyjson::Key::from(key))
            .map(|(_, v)| v),
        _ => None,
    }
}

/// `_assertions`: upsert reference assertions owned by this grant.
fn assertions(
    conn: &Connection,
    profile: &str,
    grant_row: &Value,
    observations: &[(String, String, Value, Value)],
    sources: &mut dyn Sources,
) -> Result<()> {
    let spec = kassiber_pytext::loads(&py_text(field(grant_row, "spec_json")))
        .map_err(|error| AppError::new("internal_error", error.to_string()))?;
    let binding = kassiber_pytext::loads(&py_text(field(grant_row, "binding_json")))
        .map_err(|error| AppError::new("internal_error", error.to_string()))?;
    for (occurrence, txid, raw, status) in observations {
        conn.execute(
            "INSERT INTO chain_analysis_reference_assertions
            VALUES(?,?,?,?,?,?,?,?,?,1,?) ON CONFLICT(grant_id,occurrence_id) DO UPDATE SET
            payload_json=excluded.payload_json,status_json=excluded.status_json,active=1,observed_at=excluded.observed_at",
            params![
                sql(field(grant_row, "id"), 1)?,
                profile,
                sql(field(&binding, "domain_id"), 3)?,
                sql(field(&spec, "chain"), 4)?,
                sql(field(&spec, "network"), 5)?,
                occurrence,
                txid,
                canonical(raw)?,
                canonical(status)?,
                sources.now()
            ],
        )
        .map_err(database_error)?;
    }
    Ok(())
}

struct Pass<'r, 'a> {
    conn: &'a Connection,
    profile: &'a str,
    row: Value,
    token: String,
    reader: &'r mut Reader<'a>,
    runtime: &'a dyn Runtime,
}

impl Pass<'_, '_> {
    fn validate(&self) -> Result<()> {
        validate(
            self.conn,
            self.profile,
            &self.row,
            Some(&self.token),
            self.runtime,
        )?;
        Ok(())
    }

    fn hash(&mut self, height: i64) -> Result<Value> {
        self.reader
            .rpc("getblockhash", Value::List(vec![int(height)]))
    }

    /// `_blocks`: one bounded, resumable, reorg-aware block range pass.
    fn blocks(&mut self, sources: &mut dyn Sources) -> Result<&'static str> {
        let (conn, id) = (self.conn, field(&self.row, "id").clone());
        let spec = kassiber_pytext::loads(&py_text(field(&self.row, "spec_json")))
            .map_err(|error| AppError::new("internal_error", error.to_string()))?;
        let tip = match self.reader.rpc("getblockcount", Value::Null)? {
            Value::Int(n) if n >= BigInt::from(0) => {
                i64::try_from(n).map_err(|_| fail("invalid_observation", "Invalid chain height"))?
            }
            _ => return Err(fail("invalid_observation", "Invalid chain height")),
        };
        let tip_hash = self.hash(tip)?;
        if !is_txid(&tip_hash) {
            return Err(fail("invalid_observation", "Invalid chain tip"));
        }
        let cursor = match field(&self.row, "cursor_height") {
            Value::Int(n) => Some(i64::try_from(n.clone()).unwrap_or(i64::MAX)),
            _ => None,
        };
        let cursor_hash = field(&self.row, "cursor_hash").clone();
        // Disconnect one bounded suffix per run; history is retained.
        let reorged = match cursor {
            Some(cursor) if cursor > tip => true,
            Some(cursor) => self.hash(cursor)? != cursor_hash,
            None => false,
        };
        if reorged {
            atomic(conn, || {
                self.validate()?;
                let id = sql(&id, 1)?;
                for statement in [
                    "UPDATE chain_analysis_acquisition_grants SET verified_tip_height=NULL,verified_tip_hash=NULL,verified_tip_at=NULL WHERE id=?",
                    // Withhold the uncertain source until a common ancestor is checked.
                    "UPDATE chain_analysis_reference_blocks SET active=2 WHERE grant_id=? AND active=1",
                    "UPDATE chain_analysis_reference_assertions SET active=2 WHERE grant_id=? AND active=1",
                ] {
                    conn.execute(statement, params![id]).map_err(database_error)?;
                }
                conn.execute(
                    "UPDATE chain_analysis_reference_blocks SET active=0 WHERE grant_id=? AND block_hash=?",
                    params![id, sql(&cursor_hash, 2)?],
                )
                .map_err(database_error)?;
                conn.execute(
                    "UPDATE chain_analysis_reference_assertions SET active=0 WHERE grant_id=? AND occurrence_id LIKE ?",
                    params![id, format!("{}:%", py_text(&cursor_hash))],
                )
                .map_err(database_error)?;
                let previous = first(
                    conn,
                    "SELECT height,block_hash FROM chain_analysis_reference_blocks WHERE grant_id=? AND active=2 ORDER BY height DESC LIMIT 1",
                    params![id],
                )?;
                let (height, hash) = previous.map_or((Value::Null, Value::Null), |row| {
                    (
                        field(&row, "height").clone(),
                        field(&row, "block_hash").clone(),
                    )
                });
                conn.execute(
                    "UPDATE chain_analysis_acquisition_grants SET cursor_height=?,cursor_hash=?,last_code='reorg_reconciling' WHERE id=?",
                    params![sql(&height, 1)?, sql(&hash, 2)?, id],
                )
                .map_err(database_error)?;
                Ok(())
            })?;
            commit(conn)?;
            return Ok("reorg_reconciling");
        }
        atomic(conn, || {
            self.validate()?;
            for statement in [
                "UPDATE chain_analysis_reference_blocks SET active=1 WHERE grant_id=? AND active=2",
                "UPDATE chain_analysis_reference_assertions SET active=1 WHERE grant_id=? AND active=2",
            ] {
                conn.execute(statement, params![sql(&id, 1)?]).map_err(database_error)?;
            }
            Ok(())
        })?;
        commit(conn)?;
        let start = cursor.map_or_else(|| small(field(&spec, "start_height")), |c| c + 1);
        let end_height = match get(&spec, "end_height") {
            Some(Value::Int(n)) => Some(i64::try_from(n.clone()).unwrap_or(i64::MAX)),
            _ => None,
        };
        let end = tip
            .min(end_height.unwrap_or(tip))
            .min(start + small(field(&spec, "blocks_per_run")) - 1);
        let mut parent = cursor_hash.clone();
        if parent == Value::Null && start > 0 && start <= end {
            parent = self.hash(start - 1)?;
            if !is_txid(&parent) {
                return Err(fail("invalid_observation", "Invalid range predecessor"));
            }
        }
        let network = py_text(field(&spec, "network"));
        for height in start..=end {
            let block_hash = self.hash(height)?;
            if !is_txid(&block_hash) {
                return Err(fail("invalid_observation", "Invalid block identity"));
            }
            if height == 0 && block_hash != s(super::genesis("bitcoin", &network).unwrap_or("")) {
                return Err(fail(
                    "backend_network_mismatch",
                    "Genesis changed during block acquisition",
                ));
            }
            let raw = self
                .reader
                .rpc("getblock", Value::List(vec![block_hash.clone(), int(0)]))?;
            let (previous, transactions) = clean_block(&raw, &py_text(&block_hash))?;
            if parent != Value::Null && s(previous.as_str()) != parent
                || self.hash(height)? != block_hash
            {
                return Err(stale("Chain changed during block acquisition"));
            }
            let block = py_text(&block_hash);
            atomic(conn, || {
                self.validate()?;
                let grant_id = sql(&id, 1)?;
                conn.execute(
                    "INSERT INTO chain_analysis_reference_blocks VALUES(?,?,?,?,1)
                    ON CONFLICT(grant_id,block_hash) DO UPDATE SET active=1",
                    params![grant_id, block, height, previous],
                )
                .map_err(database_error)?;
                let status = dict([
                    ("confirmed", Value::Bool(true)),
                    ("block_hash", s(block.as_str())),
                    ("block_height", int(height)),
                ]);
                for (number, txid, raw) in &transactions {
                    let mut raw = raw.clone();
                    if let Some(Value::List(inputs)) = get_mut(&mut raw, "vin") {
                        for item in inputs.iter_mut() {
                            if truthy(field(item, "is_coinbase")) {
                                continue;
                            }
                            let prior = first(
                                conn,
                                "SELECT occurrence_id,payload_json FROM chain_analysis_reference_assertions
                                WHERE grant_id=? AND active=1 AND txid=?
                                ORDER BY CAST(json_extract(status_json,'$.block_height') AS INTEGER) DESC LIMIT 1",
                                params![grant_id, sql(field(item, "txid"), 2)?],
                            )?;
                            if let Some(prior) = prior {
                                let payload =
                                    kassiber_pytext::loads(&py_text(field(&prior, "payload_json")))
                                        .map_err(|error| {
                                            AppError::new("internal_error", error.to_string())
                                        })?;
                                if let (Value::List(outputs), Value::Int(vout)) =
                                    (field(&payload, "vout"), field(item, "vout").clone())
                                {
                                    if vout >= BigInt::from(0)
                                        && vout < BigInt::from(outputs.len() as u64)
                                    {
                                        let output =
                                            outputs[usize::try_from(vout).unwrap()].clone();
                                        set(
                                            item,
                                            "reference_occurrence_id",
                                            field(&prior, "occurrence_id").clone(),
                                        );
                                        set(
                                            item,
                                            "reference_grant_id",
                                            field(&self.row, "id").clone(),
                                        );
                                        set(item, "prevout", output);
                                    }
                                }
                            }
                        }
                    }
                    assertions(
                        conn,
                        self.profile,
                        &self.row,
                        &[(
                            format!("{block}:{number}"),
                            txid.clone(),
                            raw,
                            status.clone(),
                        )],
                        sources,
                    )?;
                }
                conn.execute(
                    "UPDATE chain_analysis_acquisition_grants SET cursor_height=?,cursor_hash=?,last_code=NULL WHERE id=?",
                    params![height, block, grant_id],
                )
                .map_err(database_error)?;
                Ok(())
            })?;
            commit(conn)?;
            parent = s(block.as_str());
        }
        if self.hash(tip)? != tip_hash {
            return Err(stale("Chain tip changed during acquisition"));
        }
        atomic(conn, || {
            self.validate()?;
            conn.execute(
                "UPDATE chain_analysis_acquisition_grants SET verified_tip_height=?,verified_tip_hash=?,verified_tip_at=? WHERE id=?",
                params![tip, py_text(&tip_hash), sources.now(), sql(&id, 4)?],
            )
            .map_err(database_error)?;
            Ok(())
        })?;
        commit(conn)?;
        Ok(if end_height.is_some_and(|limit| end >= limit) {
            "range_complete"
        } else if end >= tip {
            "caught_up"
        } else {
            "backfilling"
        })
    }
}

const PAUSING: [&str; 9] = [
    "acquisition_quota",
    "acquisition_expired",
    "acquisition_source_changed",
    "scope_changed",
    "book_network_unbound",
    "book_network_mismatch",
    "watch_requires_encrypted_database",
    "backend_network_mismatch",
    "acquisition_revoked",
];

/// Python records only `AppError`, `OSError`, `ValueError` and `TypeError`
/// as a pass outcome; SQLite failures propagate after the lease is released.
/// Transport failures are `OSError`s there and become `history_unavailable`.
fn outcome_code(error: &AppError) -> Option<String> {
    if error.code == "internal_error" {
        let transport = error.message.starts_with("<urlopen error") || error.message == "timed out";
        return transport.then(|| "history_unavailable".to_owned());
    }
    (!error.code.starts_with("database")).then(|| error.code.clone())
}

/// `run(conn, profile, id)`: one leased pass; returns the public grant.
pub(in super::super) fn run(
    conn: &Connection,
    profile: &str,
    id: &Value,
    sources: &mut dyn Sources,
    runtime: &dyn Runtime,
) -> Result<Value> {
    if !conn.is_autocommit() {
        return Err(fail(
            "transaction_active",
            "Commit pending edits before acquisition",
        ));
    }
    let token = sources.id().replace('-', "");
    let (live, backend) = atomic(conn, || {
        let current = row(conn, profile, id)?;
        let (live, backend) = validate(conn, profile, &current, None, runtime)?;
        let now = runtime.epoch();
        if truthy(field(&live, "lease_token")) && small(field(&live, "lease_until")) > now {
            return Err(fail(
                "acquisition_running",
                "This source is already running",
            ));
        }
        conn.execute(
            "UPDATE chain_analysis_acquisition_grants SET lease_token=?,lease_until=? WHERE id=?",
            params![token, now + 90, sql(id, 3)?],
        )
        .map_err(database_error)?;
        Ok((live, backend))
    })?;
    commit(conn)?;
    let spec = kassiber_pytext::loads(&py_text(field(&live, "spec_json")))
        .map_err(|error| AppError::new("internal_error", error.to_string()))?;
    let outcome = (|| -> Result<&'static str> {
        // The grant enabled this narrowly scoped recurring read, not a click.
        let mut reader = Reader::scoped(backend, runtime)?;
        reader.budget = super::reader::Budget::new(64);
        reader.quota = Some(Box::new(GrantQuota {
            conn,
            profile,
            original: live.clone(),
            token: token.clone(),
            runtime,
            reserved: false,
            response_bytes: 0,
        }));
        let result = (|| -> Result<&'static str> {
            let network = py_text(field(&spec, "network"));
            if reader.genesis()? != s(super::genesis("bitcoin", &network).unwrap_or("")) {
                return Err(fail("backend_network_mismatch", "Source genesis changed"));
            }
            let mut pass = Pass {
                conn,
                profile,
                row: live.clone(),
                token: token.clone(),
                reader: &mut reader,
                runtime,
            };
            if field(&spec, "mode") == &s("blocks") {
                return pass.blocks(sources);
            }
            let subject = py_text(field(&spec, "subject"));
            let (raw, observed) = super::apply::transaction(pass.reader, &subject, "bitcoin")?;
            atomic(conn, || {
                pass.validate()?;
                conn.execute(
                    "UPDATE chain_analysis_reference_assertions SET active=0 WHERE grant_id=? AND active=1",
                    params![sql(id, 1)?],
                )
                .map_err(database_error)?;
                assertions(
                    conn,
                    profile,
                    &live,
                    &[(format!("{token}:{subject}"), subject.clone(), raw, observed)],
                    sources,
                )
            })?;
            commit(conn)?;
            Ok("observed")
        })();
        // A failed request keeps its reservation conservatively.
        let result = result.and_then(|code| reader.settle().map(|()| code));
        reader.revoke();
        result
    })();
    let (code, status, failure) = match outcome {
        Ok(code) => (s(code), "active", None),
        Err(error) => {
            if !conn.is_autocommit() {
                let _ = conn.execute_batch("ROLLBACK");
            }
            match outcome_code(&error) {
                Some(code) => {
                    let paused = PAUSING.contains(&code.as_str());
                    (
                        s(code.as_str()),
                        if paused { "paused" } else { "active" },
                        None,
                    )
                }
                None => (Value::Null, "active", Some(error)),
            }
        }
    };
    let delay = if ["backfilling", "reorg_reconciling"].contains(&py_text(&code).as_str()) {
        1
    } else {
        small(field(&spec, "interval_seconds"))
    };
    let next = runtime.epoch() + delay;
    atomic(conn, || {
        // Revocation or another worker's lease can never be undone here.
        conn.execute(
            "UPDATE chain_analysis_acquisition_grants SET status=?,last_code=?,next_run_at=?,lease_token=NULL,lease_until=NULL
            WHERE id=? AND status='active' AND revision=? AND lease_token=?",
            params![status, sql(&code, 2)?, next, sql(id, 4)?, sql(field(&live, "revision"), 5)?, token],
        )
        .map_err(database_error)?;
        Ok(())
    })?;
    commit(conn)?;
    if let Some(error) = failure {
        return Err(error);
    }
    grant(conn, profile, id)
}

/// `run_due`: at most one due grant per profile and pass.
pub(in super::super) fn run_due(
    conn: &Connection,
    profile: &str,
    sources: &mut dyn Sources,
    runtime: &dyn Runtime,
) -> Result<Vec<Value>> {
    let due = crate::publication::row::rows(
        conn,
        "SELECT id FROM chain_analysis_acquisition_grants WHERE profile_id=? AND status='active' AND next_run_at<=? ORDER BY next_run_at,id LIMIT 1",
        params![profile, runtime.epoch()],
    )?;
    let mut results = Vec::new();
    for row in due {
        if runtime.cancelled() {
            break;
        }
        let id = field(&row, "id").clone();
        match run(conn, profile, &id, sources, runtime) {
            Ok(value) => results.push(value),
            Err(error) if error.code == "acquisition_running" => continue,
            Err(error)
                if [
                    "acquisition_expired",
                    "acquisition_source_changed",
                    "scope_changed",
                    "book_network_unbound",
                    "book_network_mismatch",
                    "watch_requires_encrypted_database",
                ]
                .contains(&error.code.as_str()) =>
            {
                atomic(conn, || {
                    conn.execute(
                        "UPDATE chain_analysis_acquisition_grants SET status='paused',last_code=? WHERE id=? AND status='active'",
                        params![error.code, sql(&id, 2)?],
                    )
                    .map_err(database_error)?;
                    Ok(())
                })?;
                commit(conn)?;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(results)
}
