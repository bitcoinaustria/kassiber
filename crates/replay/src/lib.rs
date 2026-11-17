//! Journal replay bundles (docs/reference/replay-bundle.md): one journal
//! run's pseudonymized inputs and canonical outputs in the backups' age
//! passphrase envelope, and the verify that re-runs them without a book.
//!
//! Nothing here performs network I/O. Export writes only the sealed bytes the
//! caller asks for; no plaintext copy reaches disk.
pub mod outputs;
pub mod projection;
pub mod pseudonym;
pub mod registry;
mod swap;
pub mod typed;

use kassiber_envelope::AppError;
use kassiber_journals::reproduce::{self, Table};
use kassiber_journals::Refusal;
use pseudonym::{sha256_hex, Pseudonymizer};
use serde_json::{json, Map, Value};
use std::path::Path;

pub const FORMAT: &str = "kassiber.replay-bundle";
pub const VERSION: u64 = 1;
/// The bundle file extension, without the dot.
pub const EXTENSION: &str = "kassiber-replay";
pub const MIN_PASSPHRASE_CHARS: usize = 8;

/// What the user confirms before export.
pub const DISCLOSURE: &str = "A replay bundle holds the exact amounts, dates, exchange rates and tax settings of this book's transactions, with transaction ids, addresses, outpoints, labels and notes replaced by pseudonyms. It holds no descriptors, xpubs, keys, credentials or server addresses. Anyone with the file and its passphrase can read those amounts: share it only with someone you trust, and send the passphrase separately.";

/// Written into every manifest.
pub const WATERMARK: &str = "Kassiber replay bundle for reproducing journal results. It holds exact amounts, dates, rates and tax settings of one book; identifiers and labels are pseudonymized. It is not a backup and not a tax report.";

const INCLUDES: &[&str] = &[
    "book tax settings",
    "wallet ids, kinds, accounts and network facts",
    "transactions: amounts, fees, dates, kinds and pricing",
    "exchange rates the journals used",
    "reviewed custody components, gap reviews and pair dismissals",
    "ownership index and evidence snapshots",
    "journal entries, holdings, quantity balances, per-year tax summary and quarantines",
];

const EXCLUDES: &[&str] = &[
    "descriptors and extended public keys",
    "Silent Payments material and SLIP77 blinding keys",
    "credentials, backend names and URLs",
    "BDK/LWK observer state",
    "chat content, attachments, metadata notes and tags",
    "raw transaction ids, addresses and outpoints (pseudonymized)",
    "labels, notes and descriptions (pseudonymized)",
];

fn app(refusal: Refusal) -> AppError {
    kassiber_journals::service::application_error(refusal)
}

fn corrupt(message: impl Into<String>) -> AppError {
    AppError::new("replay_bundle_corrupt", message)
        .with_hint("The file is damaged or not a Kassiber replay bundle; ask for a fresh export.")
}

pub fn check_passphrase(passphrase: &str) -> Result<(), AppError> {
    if passphrase.chars().count() < MIN_PASSPHRASE_CHARS {
        return Err(AppError::new(
            "invalid_passphrase",
            format!("The bundle passphrase needs at least {MIN_PASSPHRASE_CHARS} characters"),
        ));
    }
    Ok(())
}

fn tables_json(tables: &[Table]) -> Value {
    json!({"tables": tables.iter().map(Table::to_json).collect::<Vec<_>>()})
}

fn tables_from(value: &Value) -> Result<Vec<Table>, String> {
    value["tables"]
        .as_array()
        .ok_or("replay outputs without tables")?
        .iter()
        .map(Table::from_json)
        .collect()
}

fn part(text: &str) -> Value {
    json!({"sha256": sha256_hex(text.as_bytes()), "bytes": text.len()})
}

/// One sealed bundle and the manifest it carries.
pub struct Export {
    pub sealed: Vec<u8>,
    pub manifest: Value,
}

/// Build and seal a replay bundle of the selected profile. The book is only
/// read; refused while its journals are not current.
/// The reproducibility check's statuses, as the manifest records them.
const CHECK_STATUSES: &[&str] = &[
    "identical",
    "different",
    "stale",
    "not_processed",
    "no_transactions",
    "refused",
];

pub fn export(
    conn: &kassiber_store::rusqlite::Connection,
    workspace: Option<&kassiber_pyjson::Value>,
    profile: Option<&kassiber_pyjson::Value>,
    passphrase: &str,
    created_at: &str,
    app_version: &str,
    sources: &impl kassiber_transfers::Sources,
) -> Result<Export, AppError> {
    check_passphrase(passphrase)?;
    let source = reproduce::bundle_source(conn, workspace, profile).map_err(app)?;
    if source.status != "current" {
        return Err(AppError::new(
            "journals_stale",
            format!("Journals are not current: {}", source.reason),
        )
        .with_hint("Process journals first, so the bundle matches the figures you see."));
    }
    let real = reproduce::snapshot_tables(source.snapshot.clone()).map_err(app)?;
    let pseudonymizer =
        Pseudonymizer::random().map_err(|message| AppError::new("internal_error", message))?;
    let snapshot = serde_json::to_value(&source.snapshot)
        .map_err(|error| AppError::new("internal_error", error.to_string()))?;
    let projection = projection::Projection::new(&pseudonymizer);
    let mut inputs = projection
        .snapshot(&snapshot)
        .map_err(|message| AppError::new("internal_error", message))?;
    rebind_observations(&snapshot, &mut inputs, &projection);
    let decode = |inputs: &Value| -> Result<kassiber_custody::snapshot::Snapshot, AppError> {
        serde_json::from_value(inputs.clone())
            .map_err(|error| AppError::new("internal_error", error.to_string()))
    };
    rebind_commitments(&source.snapshot, &decode(&inputs)?, &mut inputs);
    let pseudonymized = decode(&inputs)?;
    let generated = projection.generated();
    // The recomputation from projected inputs, checked against the values the
    // projection generated: this is what the bundle records.
    let outputs = outputs::project(
        &reproduce::snapshot_tables(pseudonymized).map_err(app)?,
        &outputs::Mode::Validate(&generated),
    )
    .map_err(unsupported)?;
    // The book's own run and its stored figures, through the same closed
    // schema and the projection's keyed ids.
    let mapped = outputs::project(&real, &outputs::Mode::Map(&projection)).map_err(unsupported)?;
    let profile_id = source.profile["id"].as_str().unwrap_or_default().to_owned();
    let stored = outputs::project(
        &reproduce::stored_outputs(conn, &profile_id).map_err(app)?,
        &outputs::Mode::Map(&projection),
    )
    .map_err(unsafe_stored)?;
    drop(projection);
    drop(pseudonymizer);
    // Does the projected run tell the same story as the book's own?
    let fidelity = reproduce::diff(&masked(&mapped)?, &masked(&outputs)?, 5);
    ensure_faithful(&fidelity)?;
    let local = reproduce::check(conn, workspace, profile, 1, sources).map_err(app)?;
    let inputs_text = serde_json::to_string(&inputs)
        .map_err(|error| AppError::new("internal_error", error.to_string()))?;
    let outputs_text = serde_json::to_string(&tables_json(&outputs))
        .map_err(|error| AppError::new("internal_error", error.to_string()))?;
    let stored = tables_json(&stored);
    let stored_text = serde_json::to_string(&stored)
        .map_err(|error| AppError::new("internal_error", error.to_string()))?;
    let stored_check = discrepancy(&stored, &tables_json(&outputs), 1)?;
    // The manifest is generated like the parts: book values come from the
    // projected inputs (parsed and printed again, or pseudonymized), the rest
    // are constants. Nothing is copied from the book.
    let settings = &inputs["profile"];
    let country = settings["tax_country"].as_str().unwrap_or_default();
    let registry = kassiber_tax_countries::lookup(country);
    let created_at = typed::time(created_at)
        .ok_or_else(|| AppError::new("internal_error", "invalid export time"))?;
    let local_status = CHECK_STATUSES
        .iter()
        .find(|status| local["status"] == **status)
        .copied();
    let local_differing = local["differing"].as_u64();
    let mut counts = Map::new();
    let snapshot = &source.snapshot;
    for (name, count) in [
        ("wallets", snapshot.wallets.len()),
        ("transactions", snapshot.transactions.len()),
        ("rate_rows", snapshot.rate_rows.len()),
        ("components", snapshot.components.len()),
        ("gap_reviews", snapshot.gap_reviews.len()),
        ("pair_dismissals", snapshot.pair_dismissals.len()),
        ("evidence_snapshots", snapshot.evidence_snapshots.len()),
    ] {
        counts.insert(name.into(), json!(count));
    }
    for table in &outputs {
        counts.insert(table.name.clone(), json!(table.rows.len()));
    }
    let manifest = json!({
        "format": FORMAT,
        "version": VERSION,
        "watermark": WATERMARK,
        "created_at": created_at,
        "app_version": app_version,
        "journal_semantics_version": kassiber_store::semantics::JOURNAL_CALCULATION_SEMANTICS_VERSION,
        "schema_fingerprint": kassiber_store::schema_fingerprint(conn)
            .map_err(|error| AppError::new("internal_error", error.to_string()))?,
        "tax": {
            "country": registry.map_or(settings["tax_country"].clone(), |c| json!(c.code)),
            "rules": registry.map(|c| format!("{:?}", c.rules)),
            "preview": registry.map(|c| c.preview),
            "gains_algorithm": settings["gains_algorithm"],
            "fiat_currency": settings["fiat_currency"],
            "long_term_days": settings["tax_long_term_days"],
            "cost_basis_pool_scope": settings["cost_basis_pool_scope"],
            "require_coarse_review": settings["require_coarse_review"],
            "bitcoin_rail_carrying_value": settings["bitcoin_rail_carrying_value"],
        },
        "journal_input_version": settings["journal_input_version"],
        "as_of": inputs["as_of"],
        "counts": counts,
        "includes": INCLUDES,
        "excludes": EXCLUDES,
        "local_check": {"status": local_status, "differing": local_differing},
        "stored_check": {"status": stored_check.0, "differing": stored_check.1.differing},
        "pseudonymization": {
            "method": "HMAC-SHA-256 with a random per-bundle key, discarded after export",
            "faithful": fidelity.identical(),
            "differing": fidelity.differing,
            "differences": fidelity.differences,
        },
        "parts": {"inputs": part(&inputs_text), "outputs": part(&outputs_text), "stored": part(&stored_text)},
    });
    let document = json!({
        "format": FORMAT,
        "version": VERSION,
        "manifest": manifest,
        "parts": {"inputs": inputs_text, "outputs": outputs_text, "stored": stored_text},
    });
    let plaintext = serde_json::to_vec(&document)
        .map_err(|error| AppError::new("internal_error", error.to_string()))?;
    let sealed = kassiber_backup::seal_with_passphrase(&plaintext, passphrase)?;
    Ok(Export { sealed, manifest })
}

/// Write sealed bundle bytes to a new owner-only file; never overwrites.
pub fn write_new(path: &Path, sealed: &[u8]) -> Result<(), AppError> {
    use std::io::Write;
    let io = |error: std::io::Error| match error.kind() {
        std::io::ErrorKind::AlreadyExists => {
            AppError::new("file_exists", "The bundle file already exists")
                .with_hint("Choose a new file name; Kassiber does not overwrite files.")
        }
        _ => AppError::new("io_error", format!("Could not write the bundle: {error}")),
    };
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(io)?;
    let written = file.write_all(sealed).and_then(|()| file.sync_all());
    if let Err(error) = written {
        drop(file);
        let _ = std::fs::remove_file(path);
        return Err(io(error));
    }
    Ok(())
}

/// Decrypt a bundle in memory, check its parts against the manifest,
/// recompute its outputs from its inputs and diff them with the recorded
/// ones. Needs no book, configuration or network.
pub fn verify(sealed: &[u8], passphrase: &str, limit: usize) -> Result<Value, AppError> {
    let plaintext = kassiber_backup::open_with_passphrase(sealed, passphrase)?;
    let document: Value =
        serde_json::from_slice(&plaintext).map_err(|_| corrupt("The bundle is not valid JSON"))?;
    if document["format"] != FORMAT {
        return Err(corrupt("The file is not a Kassiber replay bundle"));
    }
    if document["version"] != VERSION {
        return Err(AppError::new(
            "replay_bundle_unsupported",
            format!(
                "Replay bundle version {} is not supported",
                document["version"]
            ),
        )
        .with_hint("Verify it with the Kassiber version that wrote it, or a newer one."));
    }
    let manifest = &document["manifest"];
    let mut texts = Vec::new();
    for name in ["inputs", "outputs", "stored"] {
        let text = document["parts"][name]
            .as_str()
            .ok_or_else(|| corrupt(format!("The bundle has no {name} part")))?;
        let expected = &manifest["parts"][name];
        if expected["sha256"] != sha256_hex(text.as_bytes()) || expected["bytes"] != text.len() {
            return Err(
                corrupt(format!("The bundle's {name} do not match its manifest")).with_details(
                    kassiber_envelope::dict([("part", kassiber_envelope::s(name))]),
                ),
            );
        }
        texts.push(text);
    }
    let snapshot: kassiber_custody::snapshot::Snapshot = serde_json::from_str(texts[0])
        .map_err(|error| corrupt(format!("Unreadable bundle inputs: {error}")))?;
    let recorded = serde_json::from_str::<Value>(texts[1])
        .map_err(|_| corrupt("Unreadable bundle outputs"))
        .and_then(|value| tables_from(&value).map_err(corrupt))?;
    let mut report = Map::new();
    report.insert(
        "bundle".into(),
        json!({
            "created_at": manifest["created_at"],
            "app_version": manifest["app_version"],
            "journal_semantics_version": manifest["journal_semantics_version"],
            "tax": manifest["tax"],
            "counts": manifest["counts"],
            "local_check": manifest["local_check"],
            "stored_check": manifest["stored_check"],
            "pseudonymization": manifest["pseudonymization"],
        }),
    );
    report.insert(
        "journal_semantics_version".into(),
        json!(kassiber_store::semantics::JOURNAL_CALCULATION_SEMANTICS_VERSION),
    );
    let canonical = |tables: Vec<Table>| {
        outputs::project(&tables, &outputs::Mode::Verify)
            .map_err(|refused| Refusal::from(unsupported(refused)))
    };
    let recomputed = match reproduce::snapshot_tables(snapshot.clone()).and_then(canonical) {
        Ok(tables) => tables,
        Err(refusal) => {
            report.insert("status".into(), json!("refused"));
            report.insert(
                "error".into(),
                json!({"code": refusal.code.clone().unwrap_or_else(|| refusal.family.clone()), "message": refusal.message}),
            );
            return Ok(Value::Object(report));
        }
    };
    let compared = reproduce::diff(&recorded, &recomputed, limit);
    let deterministic = reproduce::snapshot_tables(snapshot)
        .and_then(canonical)
        .map(|again| reproduce::diff(&recomputed, &again, 1).identical())
        .ok();
    if compared.identical() {
        report.insert("status".into(), json!("identical"));
        report.insert(
            "reason".into(),
            json!("recomputing the bundle's inputs reproduces its recorded outputs"),
        );
    } else {
        report.insert("status".into(), json!("different"));
        report.insert(
            "reason".into(),
            json!("recomputing the bundle's inputs differs from its recorded outputs"),
        );
    }
    report.insert("deterministic".into(), json!(deterministic));
    report.extend(compared.fields());
    // The stored figures the bundle carries against this recomputation.
    let stored: Value =
        serde_json::from_str(texts[2]).map_err(|_| corrupt("Unreadable stored outputs"))?;
    let (status, stored) = discrepancy(&stored, &tables_json(&recomputed), limit)?;
    let mut section = Map::new();
    section.insert("status".into(), json!(status));
    section.insert(
        "reason".into(),
        json!(if status == "identical" {
            "the book's stored journals matched this recomputation"
        } else {
            "the book's stored journals differ from this recomputation"
        }),
    );
    section.extend(stored.fields());
    report.insert("stored".into(), Value::Object(section));
    Ok(Value::Object(report))
}

/// Hex runs of 40 or more characters masked as `#`, in keys and strings.
fn without_digests(value: &Value) -> Value {
    fn mask(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut run = String::new();
        let flush = |run: &mut String, out: &mut String| {
            if run.len() >= 40 && run.bytes().any(|b| b.is_ascii_alphabetic()) {
                out.extend(std::iter::repeat_n('#', run.len()));
            } else {
                out.push_str(run);
            }
            run.clear();
        };
        for c in text.chars() {
            if c.is_ascii_hexdigit() {
                run.push(c);
            } else {
                flush(&mut run, &mut out);
                out.push(c);
            }
        }
        flush(&mut run, &mut out);
        out
    }
    match value {
        Value::String(text) => Value::String(mask(text)),
        Value::Array(items) => Value::Array(items.iter().map(without_digests).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (mask(k), without_digests(v)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Rows whose chain observation was current before pseudonymization get
/// digests of their pseudonymized content, so they stay current: the
/// engine recomputes both digests from the row and compares.
fn rebind_observations(
    original: &Value,
    pseudonymized: &mut Value,
    projection: &projection::Projection<'_>,
) {
    fn rebind(original: &Value, row: &mut Value) {
        if original.get("observation_graph_hash").is_none()
            || !kassiber_custody::evidence::current_observation(original).unwrap_or(false)
        {
            return;
        }
        if let Ok((graph, _)) = kassiber_custody::evidence::graph_hash(&row["raw_json"]) {
            row["observation_graph_hash"] = Value::String(graph);
        }
        if let Ok((quantity, _)) = kassiber_custody::evidence::observed_hash(row) {
            row["observation_quantity_hash"] = Value::String(quantity);
        }
    }
    let paired = |original: &Value, pseudonymized: &mut Value| {
        if let (Some(before), Some(after)) = (original.as_array(), pseudonymized.as_array_mut()) {
            for (before, after) in before.iter().zip(after.iter_mut()) {
                rebind(before, after);
            }
        }
    };
    paired(
        &original["transactions"],
        &mut pseudonymized["transactions"],
    );
    paired(
        &original["gap_worklist"]["rows"],
        &mut pseudonymized["gap_worklist"]["rows"],
    );
    if let (Some(before), Some(after)) = (
        original["anchors"].as_object(),
        pseudonymized["anchors"].as_object_mut(),
    ) {
        for (key, row) in before {
            if let Some(target) = after.get_mut(&projection.id(key)) {
                rebind(row, target);
            }
        }
    }
}

/// Reviewed custody components bind their anchors' evidence by digest. For
/// each component whose evidence matched before pseudonymization, bind the
/// digests of the pseudonymized evidence, so the review still applies.
fn rebind_commitments(
    original: &kassiber_custody::snapshot::Snapshot,
    pseudonymized: &kassiber_custody::snapshot::Snapshot,
    inputs: &mut Value,
) {
    use kassiber_custody::components::review::{activation_evidence, component_evidence_status};
    for (index, (before, after)) in original
        .components
        .iter()
        .zip(&pseudonymized.components)
        .enumerate()
    {
        let matched = component_evidence_status(original, before)
            .is_ok_and(|status| status["status"] == "matched");
        if !matched || after.commitments.is_empty() {
            continue;
        }
        let Ok(evidence) = activation_evidence(pseudonymized, after) else {
            continue;
        };
        if evidence.commitments.len() != after.commitments.len() {
            continue;
        }
        let commitments = &mut inputs["components"][index]["commitments"];
        for (ordinal, (quantity, detail)) in evidence.commitments.iter().enumerate() {
            commitments[ordinal]["quantity_hash"] = json!(quantity);
            commitments[ordinal]["detail_hash"] = json!(detail);
        }
        let subject = after.header.get("id").cloned().unwrap_or(Value::Null);
        if let Some(rows) = inputs["evidence_snapshots"].as_array_mut() {
            let mut stored: Vec<&mut Value> = rows
                .iter_mut()
                .filter(|row| row["subject_id"] == subject)
                .collect();
            if stored.len() == evidence.snapshots.len() {
                for (row, fresh) in stored.iter_mut().zip(&evidence.snapshots) {
                    for field in ["payload_json", "detail_hash", "quantity_hash"] {
                        if !fresh[field].is_null() {
                            row[field] = fresh[field].clone();
                        }
                    }
                }
            }
        }
    }
}

/// Refuse a bundle whose projected inputs compute differently from the
/// book's own: it would not reproduce the user's numbers. The error names
/// tables, keys and columns only, never values.
pub fn ensure_faithful(fidelity: &reproduce::Diff) -> Result<(), AppError> {
    if fidelity.identical() {
        return Ok(());
    }
    let where_: Vec<Value> = fidelity
        .differences
        .iter()
        .map(|d| {
            json!({
                "table": d["table"],
                "change": d["change"],
                "columns": d["columns"].as_array().map(|columns| {
                    columns.iter().map(|c| c["column"].clone()).collect::<Vec<_>>()
                }),
            })
        })
        .collect();
    let details = kassiber_custody::codec::from_json(&json!({
        "differing": fidelity.differing,
        "differences": where_,
    }))
    .map_err(|message| AppError::new("internal_error", message))?;
    Err(AppError::new(
        "replay_bundle_unfaithful",
        format!(
            "The pseudonymized inputs compute {} journal row(s) differently from this book; the bundle would not reproduce your numbers",
            fidelity.differing
        ),
    )
    .with_hint("Nothing was written. Please report this export refusal; it names the affected tables but no values.")
    .with_details(details))
}

/// Stored versus recomputed canonical tables, derived digests masked (the
/// recomputation's come from pseudonymized content).
fn discrepancy(
    stored: &Value,
    recomputed: &Value,
    limit: usize,
) -> Result<(&'static str, reproduce::Diff), AppError> {
    let stored = tables_from(&without_digests(stored)).map_err(corrupt)?;
    let recomputed = tables_from(&without_digests(recomputed)).map_err(corrupt)?;
    let diff = reproduce::diff(&stored, &recomputed, limit);
    Ok((
        if diff.identical() {
            "identical"
        } else {
            "different"
        },
        diff,
    ))
}

fn masked(tables: &[Table]) -> Result<Vec<Table>, AppError> {
    tables_from(&without_digests(&tables_json(tables)))
        .map_err(|message| AppError::new("internal_error", message))
}

/// A stored output cell the closed schema refuses: named by table and
/// column, never by value.
fn unsafe_stored(refused: outputs::Unsafe) -> AppError {
    AppError::new(
        "replay_bundle_unsafe_stored",
        "A stored journal figure is not in a form the replay bundle can carry",
    )
    .with_hint("Process journals again and retry; if this persists, report the table and column.")
    .with_details(kassiber_envelope::dict([
        ("table", kassiber_envelope::s(refused.table.as_str())),
        ("column", kassiber_envelope::s(refused.column.as_str())),
    ]))
}

/// A computed figure outside the closed schema (an engine value it does
/// not list yet): the export is refused, naming no value.
fn unsupported(refused: outputs::Unsafe) -> AppError {
    AppError::new(
        "replay_bundle_unsupported",
        "This book produces a journal figure the replay bundle cannot carry yet",
    )
    .with_hint("Nothing was written. Please report the table and column.")
    .with_details(kassiber_envelope::dict([
        ("table", kassiber_envelope::s(refused.table.as_str())),
        ("column", kassiber_envelope::s(refused.column.as_str())),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kassiber_journals::reproduce::{Cell, Row};
    use std::collections::BTreeMap;

    fn table(value: i64) -> Vec<Table> {
        vec![Table::new(
            "journal_entries",
            vec![Row {
                key: "t|w|disposal|BTC".into(),
                transaction_id: None,
                wallet_id: None,
                year: None,
                columns: BTreeMap::from([("cost_basis".into(), Cell::Int(value))]),
            }],
        )]
    }

    #[test]
    fn an_unfaithful_projection_refuses_the_export_without_values() {
        assert!(ensure_faithful(&reproduce::diff(&table(1), &table(1), 5)).is_ok());
        let error =
            ensure_faithful(&reproduce::diff(&table(1234567), &table(7654321), 5)).unwrap_err();
        assert_eq!(error.code, "replay_bundle_unfaithful");
        let details =
            kassiber_pyjson::dumps(error.details.as_ref().unwrap(), &Default::default()).unwrap();
        assert!(
            details.contains("cost_basis") && details.contains("journal_entries"),
            "{details}"
        );
        assert!(
            !details.contains("1234567") && !details.contains("7654321"),
            "{details}"
        );
        // Composite keys carry ids, codes and asset names: opaque references only.
        assert!(
            !details.contains("t|w|disposal") && !details.contains("\"key\""),
            "{details}"
        );
    }
}
