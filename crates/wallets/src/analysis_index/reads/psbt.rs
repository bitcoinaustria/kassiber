//! Local PSBT analysis, comparison and conditional entropy. Inputs are caller
//! file bytes, request text or a staged grant; none is retained or sent.
use super::*;
use crate::psbt::{self, Payload};

enum Owned {
    Bytes(Vec<u8>),
    Json(Value),
}
impl Owned {
    fn payload(&self) -> Payload<'_> {
        match self {
            Owned::Bytes(v) => Payload::Bytes(v),
            Owned::Json(v) => Payload::Json(v),
        }
    }
}

/// `_psbt.read`: exactly one of the text argument or its staged token.
fn read(args: &Value, key: &str, local: &mut Local, scope: &[u8]) -> Result<Owned> {
    let token = format!("{key}_token");
    if get(args, key).is_some() && get(args, &token).is_some() {
        return Err(invalid("Use either selected source or PSBT text"));
    }
    if let Some(token) = get(args, &token) {
        let runtime = local.runtime.clone().unwrap_or_default();
        let mut stream = runtime.open(scope, token, "psbt")?;
        return Ok(Owned::Bytes(stream.read_up_to(runtime::PSBT_LIMIT + 1)?));
    }
    let Some(value) = get(args, key) else {
        return Err(invalid("Select a PSBT source"));
    };
    if let Some(index) = local.files.iter().position(|(k, _)| k == key) {
        return Ok(Owned::Bytes(local.files.remove(index).1));
    }
    Ok(Owned::Json(value.clone()))
}

pub(super) fn execute(
    conn: &Connection,
    profile: &str,
    operation: &str,
    args: &Value,
    local: &mut Local,
) -> Result<Value> {
    let scope = runtime::scope_key(conn, profile);
    if operation == "psbt.compare" {
        arguments(
            args,
            &[
                "before",
                "after",
                "before_token",
                "after_token",
                "network",
                "payjoin",
            ],
            &["network"],
        )?;
        let before = read(args, "before", local, &scope)?;
        let after = read(args, "after", local, &scope)?;
        return psbt::compare(
            &before.payload(),
            &after.payload(),
            field(args, "network"),
            get(args, "payjoin"),
        );
    }
    let mut fields = vec!["psbt", "psbt_token", "network"];
    if operation != "psbt.analyze" {
        fields.extend(["max_states", "max_duration_ms", "scenario"]);
    }
    arguments(args, &fields, &["network"])?;
    let source = read(args, "psbt", local, &scope)?;
    let (result, _) = psbt::analyze_parts(
        &source.payload(),
        field(args, "network"),
        "psbt:transaction",
    )?;
    if operation == "psbt.analyze" {
        return Ok(result);
    }
    let scenario = scenario(args)?;
    let states = limit(
        args,
        "max_states",
        200000,
        2000000,
        "max_states must be between 1 and 2000000",
    )?;
    let duration = limit(
        args,
        "max_duration_ms",
        1000,
        30000,
        "max_duration_ms must be between 1 and 30000",
    )?;
    let facts = field(&result, "transaction_facts").clone();
    let context = dict([
        ("schema_version", Value::Int(1.into())),
        ("subject", s("psbt:transaction")),
        ("snapshot_id", s(engine::digest(&result)?)),
        ("network", field(args, "network").clone()),
        ("source_kind", s("psbt")),
    ]);
    let mut request = context.clone();
    for (key, value) in [
        ("scenario", scenario.clone()),
        ("max_states", Value::Int(states.into())),
        ("max_duration_ms", Value::Int(duration.into())),
    ] {
        set(&mut request, key, value);
    }
    let compute = move |hooks: engine::entropy::Hooks<'_>| {
        let mut value = context;
        merge(
            &mut value,
            engine::entropy::analyze_transaction_entropy(
                &facts,
                "bitcoin",
                &Value::Int(states.into()),
                &Value::Int(duration.into()),
                &scenario,
                hooks,
            ),
        );
        value
    };
    if operation == "psbt.entropy" {
        return Ok(compute(engine::entropy::Hooks::default()));
    }
    // No input text or source grant is retained by the computation.
    start(conn, local, scope, request, compute)
}
