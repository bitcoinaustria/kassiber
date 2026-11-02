//! R4-13 orchestration. R4-10 supplies the ordered pre-build implementation.
use crate::{boundary, loader, persistence, prebuild, storage, Refusal};
use kassiber_custody::{codec, ledger_request, snapshot::Snapshot, Stages};
use kassiber_store::rusqlite::Connection;
use serde_json::{json, Value};
use std::collections::BTreeMap;

mod ordered;
mod refresh;
pub use refresh::{refresh_native, refresh_native_checked, RefreshOptions};

/// The R4-10 seam is mandatory: no default may silently skip repairs or pricing.
pub trait Prebuild {
    fn resolve(&mut self, conn: &Connection) -> Result<Value, Refusal>;
    fn repair(&mut self, conn: &Connection, profile: &mut Value) -> Result<Repairs, Refusal>;
    fn price(&mut self, conn: &Connection, profile: &Value) -> Result<usize, Refusal>;
}

#[derive(Default)]
pub struct Repairs {
    pub source_overlap_repair: Option<Value>,
    pub source_overlap_warning: Option<Value>,
    pub custody_pair_evidence_repairs: usize,
}

struct NativePrebuild<'a, R, S> {
    resolve: R,
    sources: &'a S,
    decimal_context: kassiber_pydecimal::Context,
}

impl<R, S> Prebuild for NativePrebuild<'_, R, S>
where
    R: FnMut(&Connection) -> Result<Value, Refusal>,
    S: kassiber_transfers::Sources,
{
    fn resolve(&mut self, conn: &Connection) -> Result<Value, Refusal> {
        (self.resolve)(conn)
    }

    fn repair(&mut self, conn: &Connection, profile: &mut Value) -> Result<Repairs, Refusal> {
        let repaired = prebuild::repair_journal_inputs(
            conn,
            profile,
            &mut || self.sources.now(conn).map_err(prebuild::app),
            &mut || self.sources.uuid(conn).map_err(prebuild::app),
        )?;
        *profile = repaired.profile;
        Ok(Repairs {
            source_overlap_repair: repaired.source_overlap_repair,
            source_overlap_warning: repaired.source_overlap_warning,
            custody_pair_evidence_repairs: repaired.consolidation.activated,
        })
    }

    fn price(&mut self, conn: &Connection, profile: &Value) -> Result<usize, Refusal> {
        prebuild::auto_price_transactions_from_rates_cache(
            conn,
            &prebuild::PricingScope {
                profile_id: profile["id"]
                    .as_str()
                    .ok_or_else(|| Refusal::new("KeyError", "id"))?,
                fiat_currency: profile["fiat_currency"]
                    .as_str()
                    .ok_or_else(|| Refusal::new("KeyError", "fiat_currency"))?,
                decimal_context: self.decimal_context,
            },
        )
    }
}

/// Compose the real ordered R4-10 repairs/pricing with the journal pipeline.
/// The caller resolves scope through the supplied callback under the writer
/// reservation. Production sources supply entropy and sample each lifecycle
/// clock at its reference call point; oracle sources provide exact tapes.
pub fn process_native<'a>(
    conn: &'a Connection,
    resolve: impl FnMut(&Connection) -> Result<Value, Refusal>,
    decimal_context: kassiber_pydecimal::Context,
    sources: &impl kassiber_transfers::Sources,
    progress: &mut impl FnMut(&'static str),
    writes: &mut impl storage::WriteObserver,
) -> Result<Committed<'a>, Refusal> {
    process_with_clocks(
        conn,
        &mut NativePrebuild {
            resolve,
            sources,
            decimal_context,
        },
        &mut || sources.now_precise(conn).map_err(prebuild::app),
        &mut || sources.now(conn).map_err(prebuild::app),
        &mut || sources.uuid(conn).map_err(prebuild::app),
        progress,
        writes,
    )
}

pub struct MissingPrebuild;
impl Prebuild for MissingPrebuild {
    fn resolve(&mut self, _: &Connection) -> Result<Value, Refusal> {
        Err(Refusal::new(
            "NotImplementedError",
            "R4-10 journal pre-build is not integrated",
        ))
    }
    fn repair(&mut self, _: &Connection, _: &mut Value) -> Result<Repairs, Refusal> {
        Err(Refusal::new(
            "NotImplementedError",
            "R4-10 journal pre-build is not integrated",
        ))
    }
    fn price(&mut self, _: &Connection, _: &Value) -> Result<usize, Refusal> {
        Err(Refusal::new(
            "NotImplementedError",
            "R4-10 journal pre-build is not integrated",
        ))
    }
}

/// A committed result. The transport publishes and flushes `payload` before
/// consuming this handle; dropping it restores the prior checkpoint setting.
pub struct Committed<'a> {
    pub payload: Value,
    /// The transport value in Python insertion order, prepared before COMMIT.
    pub ordered_payload: kassiber_pyjson::Value,
    checkpoint: storage::DeferredCheckpoint<'a>,
}
impl Committed<'_> {
    pub fn after_reply(self) -> Result<(), Refusal> {
        self.checkpoint.after_reply().map_err(Into::into)
    }
}

struct Transaction<'a>(&'a Connection);
impl Drop for Transaction<'_> {
    fn drop(&mut self) {
        if !self.0.is_autocommit() {
            let _ = self.0.execute_batch("ROLLBACK");
        }
    }
}

pub fn process<'a>(
    conn: &'a Connection,
    prebuild: &mut impl Prebuild,
    clock: &str,
    ids: &mut impl FnMut() -> Result<String, Refusal>,
    progress: &mut impl FnMut(&'static str),
    writes: &mut impl storage::WriteObserver,
) -> Result<Committed<'a>, Refusal> {
    process_with_clocks(
        conn,
        prebuild,
        &mut || Ok(clock.to_owned()),
        &mut || Ok(clock.to_owned()),
        ids,
        progress,
        writes,
    )
}

fn process_with_clocks<'a>(
    conn: &'a Connection,
    prebuild: &mut impl Prebuild,
    build_clock: &mut impl FnMut() -> Result<String, Refusal>,
    now: &mut impl FnMut() -> Result<String, Refusal>,
    ids: &mut impl FnMut() -> Result<String, Refusal>,
    progress: &mut impl FnMut(&'static str),
    writes: &mut impl storage::WriteObserver,
) -> Result<Committed<'a>, Refusal> {
    require_idle(conn)?;
    let checkpoint = storage::DeferredCheckpoint::enter(conn)?;
    build_in_transaction(
        conn,
        prebuild,
        Clocks {
            build_clock,
            now,
            ids,
        },
        progress,
        writes,
        checkpoint,
        true,
    )
}

fn require_idle(conn: &Connection) -> Result<(), Refusal> {
    if !conn.is_autocommit() {
        return Err(Refusal::new(
            "RuntimeError",
            "journal processing requires an idle connection",
        ));
    }
    Ok(())
}

struct Clocks<'a> {
    build_clock: &'a mut dyn FnMut() -> Result<String, Refusal>,
    now: &'a mut dyn FnMut() -> Result<String, Refusal>,
    ids: &'a mut dyn FnMut() -> Result<String, Refusal>,
}

// Only the refresh coordinator may join its own pending pairing transaction.
// Public plain processing continues to refuse an ambient caller transaction.
fn build_in_transaction<'a>(
    conn: &'a Connection,
    prebuild: &mut impl Prebuild,
    clocks: Clocks<'_>,
    progress: &mut impl FnMut(&'static str),
    writes: &mut impl storage::WriteObserver,
    checkpoint: storage::DeferredCheckpoint<'a>,
    reserve_writer: bool,
) -> Result<Committed<'a>, Refusal> {
    let Clocks {
        build_clock,
        mut now,
        mut ids,
    } = clocks;
    let cache = storage::RebuildCache::enter(conn)?;
    writes.check()?;
    if reserve_writer {
        progress("writer_wait");
        writes.check()?;
        conn.execute_batch("BEGIN IMMEDIATE")?;
    } else if conn.transaction_state(Some("main"))?
        != kassiber_store::rusqlite::TransactionState::Write
    {
        return Err(Refusal::new(
            "RuntimeError",
            "journal refresh requires its reserved writer",
        ));
    }
    let _transaction = Transaction(conn);
    progress("preparing");
    writes.check()?;
    let mut profile = prebuild.resolve(conn)?;
    kassiber_custody::journal_support::require_tax_processing(&profile)
        .map_err(|(family, message)| Refusal::new(family, message))?;
    conn.execute_batch("SAVEPOINT journals_process")?;
    progress("repairing");
    writes.check()?;
    let repairs = prebuild.repair(conn, &mut profile)?;
    progress("pricing");
    writes.check()?;
    let auto_priced = prebuild.price(conn, &profile)?;
    progress("building");
    writes.check()?;
    let clock = build_clock()?;
    let span = crate::timing::span("snapshot_load");
    let loaded = loader::load(conn, codec::from_json(&profile)?, &clock).map_err(load_error)?;
    let snapshot = loaded.into_custody().map_err(load_error)?;
    drop(span);
    let span = crate::timing::span("custody");
    let stages = kassiber_custody::run_loaded(&snapshot, true).map_err(custody_error)?;
    drop(span);
    let span = crate::timing::span("render_input");
    let wallets = wallet_refs(&snapshot);
    let custody = render_inputs(&snapshot, &stages, &wallets)?;
    let refs = wallets
        .iter()
        .map(|w| (w["id"].clone(), w.clone()))
        .collect::<Vec<_>>();
    drop(span);
    let span = crate::timing::span("fusion_engine");
    let request_span = crate::timing::span("fusion_request");
    let outcome = ledger_request::build_request(&profile, &custody.projection, &refs, "ledger")
        .map_err(fusion_error)?;
    drop(request_span);
    let engine_span = crate::timing::span("engine_boundary");
    let response = match outcome {
        ledger_request::Outcome::Empty => boundary::empty_response(),
        ledger_request::Outcome::Request(request) => crate::engine::ledger(&request)?,
    };
    drop(engine_span);
    drop(span);
    let render_span = crate::timing::span("render_boundary");
    let operands = custody.operands(&snapshot, &stages);
    let state = boundary::build_operands(&profile, &operands, &wallets, &response, String::new)?;
    drop(render_span);
    progress("storing");
    writes.check()?;
    let span = crate::timing::span("persistence_commit");
    let stored_at = now()?;
    let stored = persistence::store_ledger_state_with_clock(
        conn, &profile, &state, &stored_at, &mut now, &mut ids, writes,
    )?;
    let mut quantity_counts = serde_json::to_value(&stored.quantity_counts)
        .map_err(|error| Refusal::new("ValueError", error.to_string()))?;
    quantity_counts["differences"] = json!(crate::array(&state.quantity_differences).len());
    quantity_counts["blocked"] = json!(!crate::array(&state.custody_quantity["issues"]).is_empty());
    quantity_counts["blocked_from"] =
        state.custody_quantity["tax_eligibility"]["blocked_from"].clone();
    let mut payload = json!({
        "profile": profile["label"], "entries_created": state.entries.len(),
        "quarantined": stored.quarantines.len(),
        "transfers_detected": crate::array(state.custody_transfers).len(),
        "cross_asset_pairs": crate::array(state.cross_asset_pairs).len(),
        "auto_priced": auto_priced, "processed_transactions": stored.processed_transactions,
        "processed_at": stored.processed_at,
        "custody_quantity": quantity_counts,
    });
    if crate::truth(&state.direct_swap_payouts) {
        payload["direct_swap_payouts"] = json!(crate::array(&state.direct_swap_payouts).len());
    }
    for (key, value) in [
        ("warnings", state.warnings),
        (
            "custody_component_blockers",
            state.custody_component_blockers,
        ),
    ] {
        if crate::truth(value) {
            payload[key] = value.clone();
        }
    }
    for (key, value) in [
        ("source_overlap_repair", repairs.source_overlap_repair),
        ("source_overlap_warning", repairs.source_overlap_warning),
    ] {
        if let Some(value) = value {
            payload[key] = value;
        }
    }
    if repairs.custody_pair_evidence_repairs != 0 {
        payload["custody_pair_evidence_repairs"] = json!(repairs.custody_pair_evidence_repairs);
    }
    if stored.filed_report_impacts_resolved != 0 {
        payload["filed_report_impacts_resolved"] = json!(stored.filed_report_impacts_resolved);
    }
    let ordered_payload = ordered::payload(&payload)?;
    writes.check()?;
    conn.execute_batch("RELEASE journals_process; COMMIT")?;
    drop(span);
    progress("complete");
    cache.restore()?;
    Ok(Committed {
        payload,
        ordered_payload,
        checkpoint,
    })
}

fn load_error(error: loader::Error) -> Refusal {
    match error {
        loader::Error::Reference(error) => error,
        other => Refusal::new("RuntimeError", other.to_string()),
    }
}
fn custody_error(error: kassiber_custody::Refusal) -> Refusal {
    let family = match error.code.as_str() {
        "ValueError" | "TypeError" | "KeyError" | "OverflowError" => error.code.as_str(),
        _ => "AppError",
    };
    let mut refusal = Refusal::new(family, error.message);
    refusal.hint = error.hint;
    refusal.details = error.details;
    if family == "AppError" {
        refusal.code = Some(error.code);
    }
    refusal
}
fn fusion_error(error: ledger_request::Refusal) -> Refusal {
    let mut refusal = Refusal::new(error.family, error.message);
    refusal.code = error.code.map(str::to_owned);
    refusal.hint = error.hint.map(String::from);
    if let (Some(order), Some(map)) = (error.details_order, error.details.as_object()) {
        refusal.ordered_details = order
            .iter()
            .filter_map(|key| Some((*key, map.get(*key)?)))
            .map(|(key, value)| {
                Ok((
                    kassiber_pyjson::Key::from(key),
                    kassiber_custody::codec::from_json(value)?,
                ))
            })
            .collect::<Result<Vec<_>, String>>()
            .ok()
            .filter(|pairs| pairs.len() == map.len())
            .map(kassiber_pyjson::Value::Dict);
    }
    refusal.details = *error.details;
    refusal.invariant = error.invariant.map(str::to_owned);
    refusal.error_class = error.invariant.map(|_| "RequestError".into());
    refusal
}

fn wallet_refs(snapshot: &Snapshot) -> Vec<Value> {
    snapshot
        .wallets
        .iter()
        .map(|w| {
            Value::Object(
                w.iter()
                    .filter(|(k, _)| {
                        !["config_json", "config_text_mentions_samourai"].contains(&k.as_str())
                    })
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            )
        })
        .collect()
}

struct RenderContext {
    quantity: Value,
    projection: Value,
    native_reconciliations: Value,
    roles: Value,
    loan_legs: Vec<Value>,
    transfers: Value,
    ignored: Value,
    accounting: Value,
    warnings: Value,
    rates: Value,
}

impl RenderContext {
    fn operands<'a>(
        &'a self,
        snapshot: &'a Snapshot,
        stages: &'a Stages,
    ) -> boundary::RenderOperands<'a> {
        boundary::RenderOperands {
            rows: snapshot.rows(),
            owned_index: snapshot.owned_index.as_ref().unwrap_or(&Value::Null),
            loan_legs: &self.loan_legs,
            channel_roles: &self.roles,
            components: crate::array(&stages.components["components"]),
            component_blockers: &stages.components["blockers"],
            native_reconciliations: crate::array(&self.native_reconciliations),
            quantity: &self.quantity,
            direct_payout_records: crate::array(&self.quantity["reviewed_direct_payouts"]),
            transfers: &self.transfers,
            projection: &self.projection,
            ignored: &self.ignored,
            accounting_ignored: &self.accounting,
            warnings: &self.warnings,
            rates: &self.rates,
        }
    }
}

fn render_inputs(
    snapshot: &Snapshot,
    stages: &Stages,
    wallets: &[Value],
) -> Result<RenderContext, Refusal> {
    let span = kassiber_custody::timing::span("render.quantity");
    let quantity = stages.quantity.json_with(false, stage_value)?;
    drop(span);
    let projection = stages
        .finalized_projection
        .as_ref()
        .ok_or_else(|| Refusal::new("RuntimeError", "journal processing requires finalization"))?;
    let span = kassiber_custody::timing::span("render.context");
    let (roles, ignored, accounting) =
        kassiber_custody::interpretation::journal_context(snapshot, stages)?;
    drop(span);
    let refs = Value::Object(
        wallets
            .iter()
            .map(|w| (crate::string(&w["id"]), w.clone()))
            .collect(),
    );
    let transfers = kassiber_custody::quantity::transfers::canonical_internal_transfer_rows(
        &quantity["projection"],
        crate::array(&quantity["tax_eligibility"]["eligible_decisions"]),
        &refs,
    )?;
    let native_reconciliations = match &stages.interpretation {
        kassiber_pyjson::Value::Dict(items) => items.iter().find_map(|(key, value)|
            matches!(key, kassiber_pyjson::Key::Str(key) if key.is("native_component_reconciliations"))
                .then_some(value)),
        _ => None,
    };
    let native_reconciliations = native_reconciliations
        .map(stage_value)
        .transpose()?
        .unwrap_or(Value::Null);
    let span = kassiber_custody::timing::span("render.projection");
    let projection = projection.json_with(snapshot.rows(), stage_value)?;
    drop(span);
    Ok(RenderContext {
        quantity,
        projection,
        native_reconciliations,
        roles,
        loan_legs: snapshot
            .loan_legs
            .iter()
            .map(|row| Value::Object(row.clone().into_map()))
            .collect(),
        transfers,
        ignored: json!(ignored),
        accounting: json!(accounting),
        warnings: json!(warnings(snapshot, wallets)),
        rates: rates(snapshot)?,
    })
}

fn stage_value(value: &kassiber_pyjson::Value) -> Result<Value, Refusal> {
    use kassiber_pyjson::{Key, Value as Py};
    Ok(match value {
        Py::Null => Value::Null,
        Py::Bool(v) => json!(v),
        Py::Int(v) => serde_json::from_str(&v.to_string())
            .map_err(|e| Refusal::new("ValueError", e.to_string()))?,
        Py::Float(v) if v.is_infinite() => serde_json::from_str(if v.is_sign_negative() {
            "-1e999"
        } else {
            "1e999"
        })
        .expect("infinite scalar transport"),
        Py::Float(v) if v.is_nan() => {
            return Err(Refusal::new(
                "NotImplementedError",
                "R4-11 NaN stage transport is not integrated",
            ))
        }
        Py::Float(v) => json!(v),
        Py::Decimal(v) => json!({"$decimal":v.to_string()}),
        Py::Bytes(_) => codec::to_json(value),
        Py::Str(v) => json!(v.as_str().ok_or_else(|| Refusal::new(
            "NotImplementedError",
            "R4-11 surrogate stage transport is not integrated"
        ))?),
        Py::List(v) => Value::Array(v.iter().map(stage_value).collect::<Result<_, _>>()?),
        Py::Dict(v) => Value::Object(
            v.iter()
                .map(|(key, value)| {
                    let Key::Str(key) = key else {
                        return Err(Refusal::new(
                            "NotImplementedError",
                            "R4-11 non-string stage keys are not integrated",
                        ));
                    };
                    let key = key.as_str().ok_or_else(|| {
                        Refusal::new(
                            "NotImplementedError",
                            "R4-11 surrogate stage transport is not integrated",
                        )
                    })?;
                    Ok((key.to_owned(), stage_value(value)?))
                })
                .collect::<Result<_, Refusal>>()?,
        ),
    })
}

fn rates(snapshot: &Snapshot) -> Result<Value, Refusal> {
    use kassiber_pydecimal::{Context, Decimal};
    use std::str::FromStr;
    let mut rates = serde_json::Map::new();
    for row in &snapshot.rate_rows {
        let asset = crate::string(&row["asset"]);
        if rates.contains_key(&asset) {
            continue;
        }
        let parse = |exact: &str, legacy: &str| -> Result<Option<Decimal>, Refusal> {
            let value = row
                .get(exact)
                .filter(|v| !v.is_null())
                .or_else(|| row.get(legacy).filter(|v| !v.is_null()));
            value
                .map(|v| {
                    let raw = if v == "" {
                        "0".into()
                    } else {
                        crate::string(v)
                    };
                    Decimal::from_str(&raw).map_err(Refusal::from)
                })
                .transpose()
        };
        let rate = parse("fiat_rate_exact", "fiat_rate")?;
        let value = parse("fiat_value_exact", "fiat_value")?;
        let rate = match (rate, value) {
            (Some(rate), _) => Some(rate),
            (None, Some(value)) if crate::truth(&row["amount"]) => {
                let amount = codec::integer(&row["amount"])?;
                let amount = Decimal::from_str(&amount.to_string())?;
                let btc = amount.div(&Decimal::from_str("100000000000")?, &Context::RP2)?;
                Some(value.div(&btc, &Context::RP2)?)
            }
            _ => None,
        };
        if let Some(rate) = rate {
            rates.insert(asset, json!({"$decimal":rate.to_string()}));
        }
    }
    Ok(Value::Object(rates))
}

fn warnings(snapshot: &Snapshot, wallets: &[Value]) -> Vec<Value> {
    let mut labels = BTreeMap::<String, Vec<String>>::new();
    for w in wallets {
        if crate::truth(&w["label"]) {
            labels
                .entry(crate::string(&w["label"]))
                .or_default()
                .push(crate::string(&w["id"]));
        }
    }
    let mut warnings = Vec::new();
    for (label, mut ids) in labels {
        if ids.len() < 2 {
            continue;
        }
        ids.sort();
        warnings.push(json!({"code":"duplicate_wallet_label", "label":label,"wallet_ids":ids,
            "message":format!("{} wallets share the label '{}'. Reports key holdings by wallet label, so their balances merge and a derived self-transfer can be attributed to the wrong wallet. Rename them to be unique.", ids.len(),label)}));
    }
    warnings.extend(
        snapshot
            .ownership_warnings
            .iter()
            .map(|m| json!({"code":"ownership_index","message":m})),
    );
    warnings
}
