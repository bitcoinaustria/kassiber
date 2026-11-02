//! Pure custody computation. No storage, network, clock, logging or Python calls.
pub mod codec;
pub mod components;
pub mod confirmation;
pub mod evidence;
mod fields;
pub mod finalized_projection;
pub mod gaps;
mod gates;
mod graph;
pub mod interpretation;
pub mod journal_support;
/// `core.onchain` ownership graph parsing shared with desktop projections.
pub mod onchain {
    use kassiber_pyjson::Value as Py;
    use serde_json::Value;

    /// `parse_ownership_tx(raw_json)`: a stored observation's graph with
    /// partially valued legs, or `None` when it carries no graph.
    pub fn parse_ownership_tx(raw: &Value) -> Result<Option<Py>, String> {
        crate::graph::stored::parse(raw)
    }

    /// `merge_ownership_txs(items)`: complementary observations of one
    /// transaction, with disagreeing legs listed as evidence conflicts.
    pub fn merge_ownership_txs(items: &[Py]) -> Option<Py> {
        crate::graph::stored::merge(items)
    }
}
pub mod ledger_request;
mod liquid;
mod par;
mod python_text;
pub mod quantity;
pub mod snapshot;
pub mod source_funds;
pub mod timing;

pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const STAGES: [&str; 7] = [
    "components",
    "evidence",
    "interpretation",
    "gaps",
    "quantity",
    "finalized_projection",
    "fusion",
];

/// A refused custody operation: the refusing stage, a stable code and its
/// message. Native callers also retain application hint/details; the v1 JSON
/// refusal document keeps its established three-field error shape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub stage: &'static str,
    pub code: String,
    pub message: String,
    pub hint: Option<String>,
    pub details: serde_json::Value,
}

impl Refusal {
    fn new(stage: &'static str, code: impl Into<String>, message: impl Into<String>) -> Self {
        Refusal {
            stage,
            code: code.into(),
            message: message.into(),
            hint: None,
            details: serde_json::Value::Null,
        }
    }

    /// A refusal whose message is its code, unless the code carries one.
    fn code(stage: &'static str, code: impl Into<String>) -> Self {
        let code = code.into();
        // A `TypeError: message` code carries Python's exception text.
        match code.split_once(": ") {
            Some((family @ "TypeError", message)) => Refusal::new(stage, family, message),
            _ => Refusal::new(stage, code.clone(), code),
        }
    }

    /// The operation's refusal document.
    pub fn document(&self) -> String {
        serde_json::json!({"schema_version":1,"engine_version":ENGINE_VERSION,"error":{"stage":self.stage,"code":self.code,"message":self.message}}).to_string()
    }
}

/// Every computed stage of one custody operation, in its stage model.
pub struct Stages {
    pub components: serde_json::Value,
    pub evidence: evidence::Evidence,
    pub interpretation: kassiber_pyjson::Value,
    pub gaps: serde_json::Value,
    pub quantity: quantity::runtime::QuantityState,
    /// `None` for a decision-only operation, which skips finalization.
    pub finalized_projection: Option<finalized_projection::FinalizedProjection>,
}

/// Versioned JSON operation; missing stages are explicit and never fabricated.
pub fn custody(request: &str) -> String {
    custody_with_finalized(request, true)
}

/// Decision-only callers explicitly skip finalization, including its refusals.
/// The control is outside snapshot v1; the snapshot remains raw facts only.
pub fn custody_with_finalized(request: &str, finalized: bool) -> String {
    let snapshot = match parse(request) {
        Ok(snapshot) => snapshot,
        Err(refusal) => return refusal.document(),
    };
    let (rows, stages) = match run_with_rows(&snapshot, finalized) {
        Ok(result) => result,
        Err(refusal) => return refusal.document(),
    };
    let _span = timing::span("response");
    let large = snapshot.transactions.len() >= 256;
    let text = response(&rows, &stages, large);
    // Freeing a large book's values takes a noticeable share of the
    // operation and changes no output; release them off the caller's path
    // when a thread is available.
    if large {
        let values = (snapshot, rows, stages);
        let _ = std::thread::Builder::new().spawn(move || drop(values));
    }
    text
}

/// The v1 snapshot in `request`, admitted by its schema checks.
pub fn parse(request: &str) -> Result<snapshot::BookSnapshot, Refusal> {
    let _span = timing::span("parse");
    serde_json::from_str::<snapshot::BookSnapshot>(request)
        .map_err(|e| e.to_string())
        .and_then(|snapshot| {
            snapshot.validate()?;
            Ok(snapshot)
        })
        .map_err(|message| Refusal::new("snapshot", "custody_snapshot_invalid", message))
}

/// The Python loader preserves these earlier refusals when receipt-source
/// acquisition fails. Do not run rates, evidence or interpretation here.
pub fn receipt_source_error_precedence(snapshot: &snapshot::Snapshot) -> Result<(), Refusal> {
    let typed = snapshot
        .typed()
        .map_err(|message| Refusal::new("snapshot", "custody_snapshot_invalid", message))?;
    gates::require_accounting(&typed)?;
    components::compute(snapshot)
        .map_err(|error| Refusal::new("components", error.code, error.message))?;
    Ok(())
}

/// Admission and every stage, in Python's order. The first refusal ends the
/// operation; decision-only operations skip finalization.
pub fn run(snapshot: &snapshot::BookSnapshot, finalized: bool) -> Result<Stages, Refusal> {
    run_with_rows(snapshot, finalized).map(|(_, stages)| stages)
}

/// `run`, also returning the stage snapshot whose rows the stages index.
pub fn run_with_rows(
    snapshot: &snapshot::BookSnapshot,
    finalized: bool,
) -> Result<(snapshot::Snapshot, Stages), Refusal> {
    struct Memos;
    impl Drop for Memos {
        fn drop(&mut self) {
            codec::clear_memos();
        }
    }
    // Includes operation-local memo teardown; excludes parsing, oracle output and fusion.
    let _total = timing::span("custody.total");
    let _memos = Memos;
    let admission = timing::span("admission");
    let gate = timing::span("admission.accounting");
    gates::require_accounting(snapshot)?;
    drop(gate);
    let gate = timing::span("admission.confirmation");
    // Python captures confirmation provenance before rates and channel
    // admission, retaining the original rows rather than enriched rows.
    let projection_rows = finalized_projection::runtime::typed_confirmations(snapshot)
        .map_err(|code| Refusal::code("components", code))?;
    drop(gate);
    let gate = timing::span("admission.rates_channels");
    gates::require_rates(snapshot)
        .map_err(|(code, message)| Refusal::new("components", code, message))?;
    interpretation::admit_typed_channels(snapshot)
        .map_err(|code| Refusal::code("components", code))?;
    drop(gate);
    drop(admission);
    let boundary = timing::span("boundary");
    let snapshot = snapshot.stages();
    drop(boundary);
    let stages = run_admitted(&snapshot, finalized, projection_rows)?;
    Ok((snapshot, stages))
}

/// Native loader admission preserves legacy scalars until their consumer.
/// Typed accounting gates share the JSON adapter's implementation; stages
/// retain the original shared-column rows rather than round-tripping them.
pub fn run_loaded(snapshot: &snapshot::Snapshot, finalized: bool) -> Result<Stages, Refusal> {
    let _total = timing::span("custody.total");
    struct Memos;
    impl Drop for Memos {
        fn drop(&mut self) {
            codec::clear_memos();
        }
    }
    let _memos = Memos;
    let admission = timing::span("admission");
    snapshot
        .validate_loaded()
        .map_err(|message| Refusal::new("snapshot", "custody_snapshot_invalid", message))?;
    // The accounting gates read the typed wallet, network and rate rows the
    // v1 text round trip would produce; transactions stay records.
    let facts = snapshot
        .admission_facts()
        .map_err(|message| Refusal::new("snapshot", "custody_snapshot_invalid", message))?;
    gates::require_accounting_facts(&facts.book_network, &facts.wallets, &snapshot.profile)?;
    let projection_rows = finalized_projection::runtime::loaded_confirmations(snapshot)
        .map_err(|code| Refusal::code("components", code))?;
    gates::require_rate_rows(&facts.rate_rows)
        .map_err(|(code, message)| Refusal::new("components", code, message))?;
    interpretation::admit_channels(snapshot).map_err(|code| Refusal::code("components", code))?;
    drop(admission);
    run_admitted(snapshot, finalized, projection_rows)
}

fn run_admitted(
    snapshot: &snapshot::Snapshot,
    finalized: bool,
    projection_rows: Vec<serde_json::Value>,
) -> Result<Stages, Refusal> {
    let span = timing::span("components+migration");
    let components = components::compute(snapshot)
        .map_err(|error| Refusal::new("components", error.code, error.message))?;
    let migration = interpretation::migration_quarantines(snapshot)
        .map_err(|code| Refusal::code("components", code))?;
    drop(span);
    let span = timing::span("evidence");
    let evidence = evidence::stage(snapshot)
        .map_err(|error| Refusal::new("evidence", error.code, error.message))?;
    drop(span);
    let span = timing::span("interpretation");
    let interpretation = interpretation::stage(snapshot, &components, &evidence, migration)
        .map_err(|code| Refusal::code("interpretation", code))?;
    drop(span);
    let span = timing::span("gaps");
    let gaps = gaps::stage(snapshot, &components, &evidence, &interpretation)
        .map_err(|code| Refusal::code("gaps", code))?;
    drop(span);
    let span = timing::span("quantity");
    let quantity =
        quantity::runtime::stage(snapshot, &components, &evidence, &interpretation, &gaps)
            .map_err(|code| Refusal::code("quantity", code))?;
    drop(span);
    let span = timing::span("finalized_projection");
    let finalized_projection = if finalized {
        Some(
            finalized_projection::runtime::stage_from_state(
                snapshot,
                projection_rows,
                &quantity,
                &interpretation,
            )
            .map_err(|error| Refusal::new("finalized_projection", error.code, error.message))?,
        )
    } else {
        None
    };
    drop(span);
    Ok(Stages {
        components,
        evidence,
        interpretation,
        gaps,
        quantity,
        finalized_projection,
    })
}

/// The successful response document. `concurrent` encodes the stages on
/// separate threads, which large books use.
pub fn response(snapshot: &snapshot::Snapshot, stages: &Stages, concurrent: bool) -> String {
    let projection = stages.finalized_projection.as_ref().map(|projection| {
        projection
            .py(snapshot.rows())
            .expect("computed finalized projection")
    });
    let projection = projection.as_ref();
    let evidence = stages.evidence.value(&snapshot.transactions);
    let quantity = stages.quantity.py().expect("computed quantity stage");
    if let Some(text) = response_text(
        concurrent,
        &stages.components,
        &evidence,
        &stages.gaps,
        &stages.interpretation,
        &quantity,
        projection,
    ) {
        return text;
    }
    let response = serde_json::json!({
        "schema_version":1,
        "engine_version":ENGINE_VERSION,
        "stages":{"components":stages.components,"evidence":evidence,"gaps":stages.gaps},
        "skipped_stages":if projection.is_some() { vec!["fusion"] } else { vec!["finalized_projection", "fusion"] }
    });
    let mut response = codec::from_json(&response).expect("computed custody response");
    let mut stages_value = graph::stored::get(&response, "stages").clone();
    graph::stored::put(
        &mut stages_value,
        "interpretation",
        stages.interpretation.clone(),
    );
    graph::stored::put(&mut stages_value, "quantity", quantity);
    if let Some(projection) = projection {
        graph::stored::put(
            &mut stages_value,
            "finalized_projection",
            projection.clone(),
        );
    }
    graph::stored::put(&mut response, "stages", stages_value);
    kassiber_pyjson::dumps(&response, &kassiber_pyjson::Options::new())
        .expect("computed JSON values")
}

/// The successful response exactly as `kassiber_pyjson::dumps` writes the
/// composed document (serde stages convert through `codec::from_json`, so
/// object keys are sorted), without materializing it; `None` defers to that
/// path for any value it would treat differently.
fn response_text(
    concurrent: bool,
    components: &serde_json::Value,
    evidence: &serde_json::Value,
    gaps: &serde_json::Value,
    interpretation: &kassiber_pyjson::Value,
    quantity: &kassiber_pyjson::Value,
    projection: Option<&kassiber_pyjson::Value>,
) -> Option<String> {
    enum Stage<'a> {
        Json(&'a serde_json::Value),
        Py(&'a kassiber_pyjson::Value),
    }
    // Stages sit two containers deep: the response, then its stage map.
    fn encode(stage: &Stage) -> Option<String> {
        let options = kassiber_pyjson::Options::new();
        let mut out = String::new();
        match stage {
            Stage::Json(value) => codec::dumps_json_into(value, &options, 2, &mut out)?,
            Stage::Py(value) => kassiber_pyjson::dumps_into(value, &options, 2, &mut out).ok()?,
        }
        Some(out)
    }
    let mut stages = vec![
        ("components", Stage::Json(components)),
        ("evidence", Stage::Json(evidence)),
        ("gaps", Stage::Json(gaps)),
    ];
    stages.push(("interpretation", Stage::Py(interpretation)));
    stages.push(("quantity", Stage::Py(quantity)));
    if let Some(projection) = projection {
        stages.push(("finalized_projection", Stage::Py(projection)));
    }
    // Stage texts are independent; encode a large book's concurrently when
    // threads are available, and in order otherwise.
    let texts: Vec<Option<String>> = if !concurrent {
        stages.iter().map(|(_, stage)| encode(stage)).collect()
    } else {
        std::thread::scope(|scope| {
            let workers: Vec<_> = stages
                .iter()
                .map(|(_, stage)| {
                    std::thread::Builder::new()
                        .spawn_scoped(scope, move || encode(stage))
                        .ok()
                })
                .collect();
            stages
                .iter()
                .zip(workers)
                .map(|((_, stage), worker)| match worker {
                    Some(worker) => worker
                        .join()
                        .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
                    None => encode(stage),
                })
                .collect()
        })
    };
    let options = kassiber_pyjson::Options::new();
    let mut out =
        String::with_capacity(texts.iter().flatten().map(String::len).sum::<usize>() + 256);
    out.push_str("{\"engine_version\": ");
    kassiber_pyjson::dumps_str_into(ENGINE_VERSION, &options, &mut out);
    out.push_str(", \"schema_version\": 1, \"skipped_stages\": ");
    out.push_str(if projection.is_some() {
        "[\"fusion\"]"
    } else {
        "[\"finalized_projection\", \"fusion\"]"
    });
    out.push_str(", \"stages\": {");
    for (index, ((name, _), text)) in stages.iter().zip(texts).enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        kassiber_pyjson::dumps_str_into(name, &options, &mut out);
        out.push_str(": ");
        out.push_str(&text?);
    }
    out.push_str("}}");
    Some(out)
}

/// Comparison-only fusion. The caller supplies the existing ledger engine;
/// custody itself retains no tax-engine dependency or persistence authority.
pub fn custody_with_fusion(request: &str, finalized: bool, ledger: fn(&str) -> String) -> String {
    let response = custody_with_finalized(request, finalized);
    let mut original = codec::loads(&response).expect("custody response");
    let response = codec::to_json(&original);
    if !finalized || response.get("error").is_some() {
        return codec::dumps(&original, true).expect("response");
    }
    let snapshot = codec::to_json(&codec::loads(request).expect("admitted snapshot"));
    let wallets = snapshot["wallets"]
        .as_array()
        .expect("wallets")
        .iter()
        .map(|wallet| (wallet["id"].clone(), wallet.clone()))
        .collect::<Vec<_>>();
    let fusion = match ledger_request::build_request(
        &snapshot["profile"],
        &response["stages"]["finalized_projection"],
        &wallets,
        "ledger",
    ) {
        Ok(ledger_request::Outcome::Empty) => serde_json::json!({"outcome":"empty"}),
        Ok(ledger_request::Outcome::Request(request)) => {
            let text = ledger_request::request_json(&request);
            let result: serde_json::Value =
                serde_json::from_str(&ledger(&text)).expect("ledger response");
            serde_json::json!({"outcome":"request","request":request,"request_text":text,"response":result})
        }
        Err(error) => serde_json::json!({"outcome":"refusal","error":error.document()}),
    };
    // The builder reads named JSON fields, but existing stage diagnostics may
    // contain Python surrogate code points. Preserve their lossless values.
    let mut stages = graph::stored::get(&original, "stages").clone();
    graph::stored::put(
        &mut stages,
        "fusion",
        codec::from_json(&fusion).expect("fusion"),
    );
    graph::stored::put(&mut original, "stages", stages);
    graph::stored::put(
        &mut original,
        "skipped_stages",
        kassiber_pyjson::Value::List(vec![]),
    );
    codec::dumps(&original, true).expect("fusion response")
}
