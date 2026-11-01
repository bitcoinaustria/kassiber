use crate::argparse::Namespace;
use kassiber_envelope::{dict, get, s, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;
use kassiber_wallets::analysis_index::reads::Local;
use std::io::Read;

fn invalid_document() -> AppError {
    AppError::new("validation", "Invalid chain analysis JSON document")
}

/// `_json`: inline JSON or `@path`, unwrapping a machine envelope's data.
fn document(raw: &str) -> Result<Value, AppError> {
    let document = if let Some(path) = raw.strip_prefix('@') {
        let path = crate::render::expand_user(path, std::env::var("HOME").ok().as_deref())
            .map_err(|_| invalid_document())?;
        let metadata = std::fs::metadata(&path).map_err(|_| invalid_document())?;
        if metadata.len() > 10_000_000 {
            return Err(invalid_document());
        }
        std::fs::read_to_string(path).map_err(|_| invalid_document())?
    } else {
        raw.to_owned()
    };
    let mut value = kassiber_pytext::loads(&document).map_err(|_| invalid_document())?;
    if ["kind", "schema_version", "data"]
        .iter()
        .all(|k| get(&value, k).is_some())
    {
        value = get(&value, "data").unwrap().clone();
    }
    if !matches!(value, Value::Dict(_)) {
        return Err(invalid_document());
    }
    Ok(value)
}

/// Local PSBT files are read before the book scope is resolved, as in Python.
fn psbt(book: &mut dyn Book, action: &str, args: &Namespace) -> Result<Value, AppError> {
    let mut payload = vec![(
        "network",
        args.get("network").cloned().unwrap_or(Value::Null),
    )];
    let mut local = Local::default();
    let files: &[(&str, &str)] = if action == "compare" {
        &[("before", "before"), ("after", "after")]
    } else {
        &[("psbt", "file")]
    };
    for (key, name) in files {
        let filename = args.text(name).unwrap_or("");
        let unreadable = || {
            AppError::new(
                "chain_analysis_source_unavailable",
                "Cannot read selected PSBT file",
            )
        };
        let path = crate::render::expand_user(filename, std::env::var("HOME").ok().as_deref())
            .map_err(|_| unreadable())?;
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .and_then(|f| f.take(4 * 1024 * 1024 + 1).read_to_end(&mut bytes))
            .map_err(|_| unreadable())?;
        local.files.push(((*key).to_owned(), bytes));
        payload.push((key, Value::Null));
    }
    if let Some(raw) = args.text("payjoin") {
        payload.push(("payjoin", document(raw)?));
    }
    if action == "entropy" {
        for key in ["max_states", "max_duration_ms"] {
            payload.push((key, args.get(key).cloned().unwrap_or(Value::Null)));
        }
        if let Some(raw) = args.text("scenario") {
            payload.push(("scenario", document(raw)?));
        }
    }
    book.chain_analysis_reads(
        args.get("workspace"),
        args.get("profile"),
        &format!("psbt.{action}"),
        &dict(payload),
        &mut local,
    )
}

pub fn execute(book: &mut dyn Book, command: &str, args: &Namespace) -> Result<Value, AppError> {
    let mut payload = Vec::new();
    let mode = command.trim_start_matches("chain-analysis.");
    if let Some(action) = mode.strip_prefix("psbt.") {
        return psbt(book, action, args);
    }
    let operation = if ["overview", "trace", "path"].contains(&mode) {
        for key in [
            "subject",
            "target",
            "chain",
            "network",
            "direction",
            "depth",
            "node_limit",
            "edge_limit",
            "observer",
            "include_hypotheses",
            "min_amount_msat",
            "start",
            "end",
        ] {
            if let Some(value) = args
                .items()
                .iter()
                .find(|(k, v)| k == key && *v != Value::Null)
                .map(|(_, v)| v.clone())
            {
                payload.push((key, value));
            }
        }
        payload.push(("mode", s(mode)));
        payload.push(("include_relations", Value::Bool(!args.flag("no_relations"))));
        "query"
    } else {
        let fields: &[&str] = match mode {
            "entropy" => &[
                "subject",
                "chain",
                "network",
                "observer",
                "max_states",
                "max_duration_ms",
            ],
            "datasets.query" => &[
                "subject",
                "chain",
                "network",
                "dataset_id",
                "label",
                "observer",
                "limit",
                "cursor",
            ],
            _ => &["id", "other_id", "limit", "cursor"],
        };
        for key in fields {
            if let Some(value) = args
                .items()
                .iter()
                .find(|(k, v)| k == key && *v != Value::Null)
                .map(|(_, v)| v.clone())
            {
                payload.push((*key, value));
            }
        }
        if mode == "entropy" {
            if let Some(raw) = args.text("scenario") {
                payload.push(("scenario", document(raw)?));
            }
        }
        mode
    };
    book.chain_analysis_reads(
        args.get("workspace"),
        args.get("profile"),
        operation,
        &dict(payload),
        &mut Local::default(),
    )
}
