//! Importing a chat's files into the book (#682, #698):
//! `_import_attachment_ai_payload`, `_import_document_ai_payload` and the
//! restricted-access projections of `core.attachment_import` and
//! `core.document_chat_import`.
//!
//! The file comes from the chat's staging grant, never from an argument, and
//! a document's rows from the draft the reader stored for it. Planning is a
//! read that changes nothing; applying runs only after once-only consent and
//! only when the plan's digest, the file's (or draft's) SHA-256 and the
//! book's input version still hold. The plans run the desktop's own import
//! path in a savepoint, so what the user approved is what lands. A restricted
//! provider gets counts, the column plan and the target, never the file's
//! dates, amounts, filename or row ids.

use kassiber_envelope::{dict, get, s, set, AppError};
use kassiber_pyjson::{Key, Value};

use crate::executor::{assert_scope, Book, ReadHost, Runtime};
use crate::ToolError;

/// `_IMPORT_ATTACHMENT_REQUEST_FIELDS`.
const ATTACHMENT_REQUEST_FIELDS: [&str; 4] = [
    "wallet",
    "new_wallet",
    "column_map",
    "transfers_only_confirmed",
];
/// `_IMPORT_ATTACHMENT_EXPECTED_FIELDS`.
const ATTACHMENT_EXPECTED_FIELDS: [&str; 3] = [
    "plan_digest",
    "expected_content_sha256",
    "expected_input_version",
];

/// `{key: arguments[key] for key in fields if key in arguments}`.
fn present(arguments: &Value, fields: &[&str]) -> Value {
    Value::Dict(
        fields
            .iter()
            .filter_map(|key| get(arguments, key).map(|value| (Key::from(*key), value.clone())))
            .collect(),
    )
}

/// `{key: arguments.get(key) for key in fields}`.
fn expected(arguments: &Value, fields: &[&str]) -> Value {
    Value::Dict(
        fields
            .iter()
            .map(|key| {
                (
                    Key::from(*key),
                    get(arguments, key).cloned().unwrap_or(Value::Null),
                )
            })
            .collect(),
    )
}

/// `_import_attachment_ai_payload(conn, runtime, arguments, apply=...)`.
pub fn import_attachment_for_ai(
    book: &mut dyn Book,
    state: &Value,
    arguments: &Value,
    applying: bool,
) -> Result<Value, ToolError> {
    let chat_file = crate::chat::files::ai_chat_file(
        state,
        arguments,
        Some(&crate::chat::files::CHAT_TABLE_KINDS),
    )?;
    let source_file = get(&chat_file, "source_file")
        .map(kassiber_envelope::py_str)
        .unwrap_or_default();
    let request = present(arguments, &ATTACHMENT_REQUEST_FIELDS);
    crate::file_analysis::reject_chat_asset_hints(crate::file_analysis::column_map_arg(&request)?)?;
    let expected = applying.then(|| expected(arguments, &ATTACHMENT_EXPECTED_FIELDS));
    // `resolve_scope(conn, None, None)` before the import, as Python.
    book.active_profile()?;
    let payload = book.attachment_import(&source_file, &request, expected.as_ref())?;
    if crate::executor::full_access(state) {
        Ok(payload)
    } else {
        Ok(withhold_cell_values(&payload))
    }
}

/// `_chat_document_sessions(runtime)`: a document draft lives with the
/// daemon's document grants, which only a desktop or CLI chat has.
fn require_document_sessions(state: &Value) -> Result<(), ToolError> {
    if get(state, "document_import_sessions") == Some(&Value::Bool(true)) {
        return Ok(());
    }
    Err(AppError::new(
        "ai_attachment_missing",
        "Documents can only be read in a desktop or CLI chat",
    )
    .into())
}

/// `_ROW_FIELDS`: what the model may see of a row it read; never the page
/// text.
const ROW_FIELDS: [&str; 5] = ["id", "status", "flags", "confidence", "record"];

/// `core.document_chat_import.project_draft(draft, full_access=...)`: with
/// full access each row's values, status, flags and confidence; a restricted
/// provider gets the counts only. The local path, the page text the reader
/// quoted and its source regions never go to the model.
pub fn project_draft(draft: &Value, full_access: bool) -> Value {
    let field = |key: &str| get(draft, key).cloned().unwrap_or(Value::Null);
    let source = get(draft, "source").filter(|source| matches!(source, Value::Dict(_)));
    let mut keys = vec!["kind", "sha256", "size_bytes", "pdf"];
    if full_access {
        keys.push("filename");
    }
    let kept = keys
        .into_iter()
        .filter_map(|key| {
            source
                .and_then(|source| get(source, key))
                .map(|value| (Key::from(key), value.clone()))
        })
        .collect();
    let mut projected = vec![
        (Key::from("source"), Value::Dict(kept)),
        (Key::from("model"), field("model")),
        (
            Key::from("confidence_threshold"),
            field("confidence_threshold"),
        ),
        (Key::from("summary"), field("summary")),
    ];
    if full_access {
        let rows = match get(draft, "rows") {
            Some(Value::List(rows)) => rows
                .iter()
                .filter(|row| matches!(row, Value::Dict(_)))
                .map(|row| {
                    Value::Dict(
                        ROW_FIELDS
                            .iter()
                            .filter_map(|key| {
                                get(row, key).map(|value| (Key::from(*key), value.clone()))
                            })
                            .collect(),
                    )
                })
                .collect(),
            _ => Vec::new(),
        };
        projected.push((Key::from("rows"), Value::List(rows)));
        projected.push((Key::from("untrusted_document_text"), Value::Bool(true)));
    } else {
        projected.push((Key::from("row_values_withheld"), Value::Bool(true)));
    }
    Value::Dict(projected)
}

/// `_read_chat_document_for_ai(runtime, arguments)`: draft rows from a
/// chat's image or PDF with a local vision model. The provider is resolved
/// and the draft stored on the request loop; the model call runs between
/// the two on the calling thread (the chat's worker), so the daemon keeps
/// serving other requests while the model reads. The draft is kept like
/// the desktop's document preview, and its id is what the plan and apply
/// tools take.
pub fn read_chat_document(
    arguments: &Value,
    runtime: &mut Runtime<'_>,
    host: &mut dyn ReadHost,
) -> Result<Value, ToolError> {
    let chat_file = crate::chat::files::ai_chat_file(
        runtime.state,
        arguments,
        Some(&crate::chat::files::CHAT_DOCUMENT_KINDS),
    )?;
    require_document_sessions(runtime.state)?;
    let text = |value: &Value, key: &str| get(value, key).map(kassiber_envelope::py_str);
    let source_file = text(&chat_file, "source_file").unwrap_or_default();
    let token = text(&chat_file, "token").unwrap_or_default();
    let requested = get(arguments, "provider").cloned().unwrap_or(Value::Null);
    let data_root = runtime.data_root.to_owned();
    let prepared = {
        let state = &*runtime.state;
        host.on_request_loop(&mut |book| {
            assert_scope(book, &data_root, state)?;
            book.document_reader(&requested)
        })?
    };
    let provider = get(&prepared, "provider").cloned().unwrap_or(Value::Null);
    let fiat = text(&prepared, "fiat_currency").filter(|fiat| !fiat.is_empty());
    let pages = get(arguments, "pages").cloned().unwrap_or(Value::Null);
    let (draft, detected) = host.read_document(&provider, &source_file, &pages, fiat.as_deref())?;
    let stored = {
        let state = &*runtime.state;
        host.on_request_loop(&mut |book| {
            assert_scope(book, &data_root, state)?;
            book.store_document_draft(&token, &draft, &provider, detected.as_deref())
                .map(s)
        })?
    };
    let mut projected = project_draft(&draft, crate::executor::full_access(runtime.state));
    set(
        &mut projected,
        "file",
        get(&chat_file, "id").cloned().unwrap_or(Value::Null),
    );
    set(&mut projected, "draft", stored);
    Ok(projected)
}

/// `_import_document_ai_payload(conn, runtime, arguments, apply=...)`: plan
/// (read-only) or apply importing a document draft's ready rows.
pub fn import_document_for_ai(
    book: &mut dyn Book,
    state: &Value,
    arguments: &Value,
    applying: bool,
) -> Result<Value, ToolError> {
    require_document_sessions(state)?;
    let draft = crate::mutate::required_str_arg(arguments, "draft", "Document draft")?;
    let draft = crate::pyval::lossy(&draft);
    let request = present(arguments, &["wallet", "new_wallet"]);
    let expected = applying.then(|| {
        expected(
            arguments,
            &[
                "plan_digest",
                "expected_source_sha256",
                "expected_input_version",
            ],
        )
    });
    let payload = book.document_import(&draft, &request, expected.as_ref())?;
    if crate::executor::full_access(state) {
        Ok(payload)
    } else {
        Ok(withhold_values(&payload))
    }
}

/// `core.attachment_import.withhold_cell_values(payload)`: counts, the column
/// plan, warnings and blockers stay; dates, amounts and the filename are
/// values from the file and do not cross to the model.
pub fn withhold_cell_values(payload: &Value) -> Value {
    let mut projected = payload.clone();
    if let Some(Value::Dict(attachment)) = get(payload, "attachment") {
        let kept: Vec<(Key, Value)> = attachment
            .iter()
            .filter(|(key, _)| *key != Key::from("filename"))
            .cloned()
            .collect();
        set(&mut projected, "attachment", Value::Dict(kept));
    }
    if get(payload, "date_range").is_some() {
        set(&mut projected, "date_range", Value::Null);
    }
    if let Some(totals) = get(payload, "totals") {
        let rows = match totals {
            Value::List(rows) => rows.as_slice(),
            _ => &[],
        };
        let field = |row: &Value, key: &str| get(row, key).cloned().unwrap_or(Value::Null);
        set(
            &mut projected,
            "totals",
            Value::List(
                rows.iter()
                    .map(|row| {
                        dict([
                            ("asset", field(row, "asset")),
                            ("direction", field(row, "direction")),
                            ("rows", field(row, "rows")),
                        ])
                    })
                    .collect(),
            ),
        );
    }
    if let Some(after @ Value::Dict(_)) = get(payload, "wallet_after") {
        let warnings = match get(after, "scan_bounds") {
            Some(bounds) if kassiber_envelope::truthy(bounds) => get(bounds, "warnings")
                .cloned()
                .unwrap_or(Value::List(Vec::new())),
            _ => Value::List(Vec::new()),
        };
        set(
            &mut projected,
            "wallet_after",
            dict([
                (
                    "completeness_class",
                    get(after, "completeness_class")
                        .cloned()
                        .unwrap_or(Value::Null),
                ),
                ("warnings", warnings),
            ]),
        );
    }
    set(&mut projected, "cell_values_withheld", Value::Bool(true));
    projected
}

/// `core.document_chat_import.withhold_values(payload)`.
pub fn withhold_values(payload: &Value) -> Value {
    let mut projected = withhold_cell_values(payload);
    if let Value::Dict(pairs) = &mut projected {
        pairs.retain(|(key, _)| *key != Key::from("row_ids"));
    }
    projected
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::Maintenance;

    /// A book that answers the reader's two request-loop steps.
    #[derive(Default)]
    struct Drafts {
        stored: Vec<(String, Value, Option<String>)>,
    }

    impl Book for Drafts {
        fn database_in_scope(&mut self, _: &str) -> Result<bool, ToolError> {
            Ok(true)
        }
        fn context(&mut self) -> Result<(Value, Value), ToolError> {
            Ok((s("w"), s("p")))
        }
        fn maintain_for_read(&mut self, _: bool) -> Result<Maintenance, ToolError> {
            unreachable!()
        }
        fn read(&mut self, _: &str, _: &Value, _: &'static str) -> Result<Value, ToolError> {
            unreachable!()
        }
        fn document_reader(&mut self, provider: &Value) -> Result<Value, ToolError> {
            assert_eq!(provider, &Value::Null);
            Ok(dict([
                ("fiat_currency", s("EUR")),
                ("provider", dict([("name", s("local"))])),
            ]))
        }
        fn store_document_draft(
            &mut self,
            token: &str,
            draft: &Value,
            _: &Value,
            detected: Option<&str>,
        ) -> Result<String, ToolError> {
            self.stored
                .push((token.to_owned(), draft.clone(), detected.map(str::to_owned)));
            Ok("draft-1".into())
        }
    }

    /// Records whether the model call ran while the request loop was held.
    #[derive(Default)]
    struct Host {
        book: Drafts,
        on_loop: bool,
        reads: Vec<bool>,
    }

    impl ReadHost for Host {
        fn on_request_loop(
            &mut self,
            work: &mut dyn FnMut(&mut dyn Book) -> Result<Value, ToolError>,
        ) -> Result<Value, ToolError> {
            self.on_loop = true;
            let result = work(&mut self.book);
            self.on_loop = false;
            result
        }
        fn read_document(
            &mut self,
            _: &Value,
            source_file: &str,
            _: &Value,
            fiat: Option<&str>,
        ) -> Result<(Value, Option<String>), ToolError> {
            assert_eq!((source_file, fiat), ("/staged/receipt.png", Some("EUR")));
            self.reads.push(self.on_loop);
            Ok((
                dict([
                    (
                        "source",
                        dict([
                            ("kind", s("image")),
                            ("filename", s("receipt.png")),
                            ("path", s("/staged/receipt.png")),
                        ]),
                    ),
                    (
                        "rows",
                        Value::List(vec![dict([
                            ("id", s("row-1")),
                            ("status", s("ready")),
                            ("evidence_text", s("BTC 0.01 on the page")),
                        ])]),
                    ),
                    ("summary", dict([("rows", Value::from(1))])),
                ]),
                Some("chat_completions".into()),
            ))
        }
    }

    fn chat_state(full_access: bool) -> Value {
        dict([
            ("provider_full_access", Value::Bool(full_access)),
            ("document_import_sessions", Value::Bool(true)),
            (
                "chat_files",
                Value::List(vec![dict([
                    ("id", s("file-1")),
                    ("source_file", s("/staged/receipt.png")),
                    ("filename", s("receipt.png")),
                    ("kind", s("image")),
                    ("token", s("grant")),
                ])]),
            ),
        ])
    }

    #[test]
    fn the_reader_calls_the_model_off_the_request_loop() {
        let mut host = Host::default();
        let mut state = chat_state(false);
        let mut runtime = Runtime {
            data_root: "/books/a",
            state: &mut state,
        };
        let read = read_chat_document(&dict::<&str>([]), &mut runtime, &mut host).unwrap();
        assert_eq!(host.reads, vec![false]);
        let (token, _, detected) = &host.book.stored[0];
        assert_eq!(
            (token.as_str(), detected.as_deref()),
            ("grant", Some("chat_completions"))
        );
        // Restricted: counts, never the rows, the page text, the name or the path.
        let text = format!("{read:?}");
        for private in ["row-1", "BTC 0.01", "receipt.png", "/staged"] {
            assert!(!text.contains(private), "{private} leaked: {text}");
        }
        assert_eq!(get(&read, "row_values_withheld"), Some(&Value::Bool(true)));
        assert_eq!(get(&read, "draft"), Some(&s("draft-1")));
        assert_eq!(get(&read, "file"), Some(&s("file-1")));
        // Full access gets the rows' fields, still without the page text.
        let mut state = chat_state(true);
        let mut runtime = Runtime {
            data_root: "/books/a",
            state: &mut state,
        };
        let read = read_chat_document(&dict::<&str>([]), &mut runtime, &mut host).unwrap();
        let text = format!("{read:?}");
        assert!(text.contains("row-1") && text.contains("receipt.png"));
        assert!(!text.contains("BTC 0.01") && !text.contains("/staged"));
        // Outside a chat there are no grants to keep a draft with.
        let mut state = chat_state(true);
        kassiber_envelope::set(&mut state, "document_import_sessions", Value::Null);
        let mut runtime = Runtime {
            data_root: "/books/a",
            state: &mut state,
        };
        let ToolError::App(error) =
            read_chat_document(&dict::<&str>([]), &mut runtime, &mut host).unwrap_err()
        else {
            panic!()
        };
        assert_eq!(error.code, "ai_attachment_missing");
        assert_eq!(host.reads.len(), 2);
    }

    #[test]
    fn a_restricted_provider_gets_no_value_from_the_file() {
        let plan = dict([
            (
                "attachment",
                dict([
                    ("content_sha256", s("ab")),
                    ("filename", s("Kraken_max.csv")),
                ]),
            ),
            (
                "date_range",
                dict([("first", s("2025-01-01T00:00:00Z")), ("last", Value::Null)]),
            ),
            (
                "totals",
                Value::List(vec![dict([
                    ("asset", s("BTC")),
                    ("direction", s("inbound")),
                    ("rows", Value::from(2)),
                    ("amount_msat", Value::from(300_000_000)),
                    ("fee_msat", Value::from(0)),
                ])]),
            ),
            (
                "wallet_after",
                dict([
                    ("completeness_class", s("manual")),
                    (
                        "scan_bounds",
                        dict([
                            ("first_transaction_at", s("2025-01-01T00:00:00Z")),
                            ("warnings", Value::List(vec![s("undated_transactions")])),
                        ]),
                    ),
                ]),
            ),
            ("row_ids", Value::List(vec![s("row-1")])),
            ("ready", Value::Bool(true)),
        ]);
        let projected = withhold_values(&plan);
        let text = format!("{projected:?}");
        for value in [
            "Kraken_max",
            "2025-01-01",
            "300000000",
            "row-1",
            "amount_msat",
        ] {
            assert!(!text.contains(value), "{value} leaked: {text}");
        }
        assert_eq!(
            get(&projected, "cell_values_withheld"),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            get(&projected, "wallet_after"),
            Some(&dict([
                ("completeness_class", s("manual")),
                ("warnings", Value::List(vec![s("undated_transactions")])),
            ]))
        );
        // The attachment keeps its digest; the plan keeps its verdict.
        assert_eq!(
            get(get(&projected, "attachment").unwrap(), "content_sha256"),
            Some(&s("ab"))
        );
        assert_eq!(get(&projected, "ready"), Some(&Value::Bool(true)));
        let applied = withhold_cell_values(&dict([("applied", Value::Bool(true))]));
        assert_eq!(
            applied,
            dict([
                ("applied", Value::Bool(true)),
                ("cell_values_withheld", Value::Bool(true)),
            ])
        );
    }
}
