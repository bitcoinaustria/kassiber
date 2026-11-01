use crate::argparse::Namespace;
use kassiber_envelope::{dict, get, s, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;

pub fn execute(book: &mut dyn Book, command: &str, args: &Namespace) -> Result<Value, AppError> {
    let mut payload = Vec::new();
    let mode = command.trim_start_matches("chain-analysis.");
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
                let document = if let Some(path) = raw.strip_prefix('@') {
                    let path =
                        crate::render::expand_user(path, std::env::var("HOME").ok().as_deref())
                            .map_err(|_| {
                                AppError::new("validation", "Invalid chain analysis JSON document")
                            })?;
                    let metadata = std::fs::metadata(&path).map_err(|_| {
                        AppError::new("validation", "Invalid chain analysis JSON document")
                    })?;
                    if metadata.len() > 10_000_000 {
                        return Err(AppError::new(
                            "validation",
                            "Invalid chain analysis JSON document",
                        ));
                    }
                    std::fs::read_to_string(path).map_err(|_| {
                        AppError::new("validation", "Invalid chain analysis JSON document")
                    })?
                } else {
                    raw.to_owned()
                };
                let mut value = kassiber_pytext::loads(&document).map_err(|_| {
                    AppError::new("validation", "Invalid chain analysis JSON document")
                })?;
                if ["kind", "schema_version", "data"]
                    .iter()
                    .all(|k| get(&value, k).is_some())
                {
                    value = get(&value, "data").unwrap().clone();
                }
                if !matches!(value, Value::Dict(_)) {
                    return Err(AppError::new(
                        "validation",
                        "Invalid chain analysis JSON document",
                    ));
                }
                payload.push(("scenario", value));
            }
        }
        mode
    };
    book.chain_analysis_reads(
        args.get("workspace"),
        args.get("profile"),
        operation,
        &dict(payload),
    )
}
