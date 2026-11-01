use crate::argparse::Namespace;
use kassiber_envelope::{dict, get, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;

pub const COMMANDS: &[&str] = &[
    "chain-analysis.labels.upsert",
    "chain-analysis.labels.import",
    "chain-analysis.labels.delete",
    "chain-analysis.cases.save",
    "chain-analysis.cases.delete",
    "chain-analysis.datasets.revoke",
    "chain-analysis.datasets.discard",
    "chain-analysis.watches.preview",
    "chain-analysis.watches.create",
    "chain-analysis.watches.list",
    "chain-analysis.watches.configure",
    "chain-analysis.watches.delete",
    "chain-analysis.watches.inbox",
    "chain-analysis.watches.acknowledge",
    "chain-analysis.watches.evaluate",
    "chain-analysis.sources.plan",
    "chain-analysis.sources.authorize",
    "chain-analysis.sources.list",
    "chain-analysis.sources.revoke",
    "chain-analysis.sources.run",
];

fn document(raw: &str) -> Result<Value, AppError> {
    let invalid = || AppError::new("validation", "Invalid chain analysis JSON document");
    let raw = if let Some(path) = raw.strip_prefix('@') {
        let path = crate::render::expand_user(path, std::env::var("HOME").ok().as_deref())
            .map_err(|_| invalid())?;
        if std::fs::metadata(&path).map_err(|_| invalid())?.len() > 10_000_000 {
            return Err(invalid());
        }
        std::fs::read_to_string(path)
            .map_err(|_| invalid())?
            .replace("\r\n", "\n")
            .replace('\r', "\n")
    } else {
        raw.to_owned()
    };
    let parsed = kassiber_pytext::loads(&raw).map_err(|_| invalid())?;
    let parsed = if ["kind", "schema_version", "data"]
        .iter()
        .all(|key| get(&parsed, key).is_some())
    {
        get(&parsed, "data").unwrap().clone()
    } else {
        parsed
    };
    if !matches!(parsed, Value::Dict(_)) {
        return Err(invalid());
    }
    Ok(parsed)
}

pub fn execute(book: &mut dyn Book, path: &str, ns: &Namespace) -> Result<Value, AppError> {
    // `cli.chain_analysis.dispatch`: the attributes each leaf forwards.
    let allowed: &[&str] = match path {
        "chain-analysis.watches.inbox" => &["limit", "before"],
        _ => &[
            "id",
            "other_id",
            "limit",
            "cursor",
            "after_id",
            "title",
            "expected_snapshot_id",
            "expected_revision",
        ],
    };
    let mut args = dict(allowed.iter().filter_map(|key| {
        ns.get(key)
            .filter(|v| v != &&Value::Null)
            .map(|v| (*key, v.clone()))
    }));
    let present = |key: &str| ns.text(key).filter(|raw| !raw.is_empty());
    if let Some(raw) = present("document") {
        args = document(raw)?;
    }
    for key in ["query", "plan"] {
        if let Some(raw) = present(key) {
            kassiber_envelope::set(&mut args, key, document(raw)?);
        }
    }
    book.chain_analysis_ops(
        ns.get("workspace"),
        ns.get("profile"),
        path.strip_prefix("chain-analysis.").unwrap(),
        &args,
    )
}
