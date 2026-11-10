//! `core/attachment_import.py`: plan and apply a generic-ledger import of a
//! chat's staged file, bound to its plan.
//!
//! A caller that cannot be trusted with a path or with the book (the
//! assistant) proposes a target wallet and a column plan; this answers with
//! the exact effect of importing the staged file that way, and applies it only
//! when the book, the file bytes and the effect still match. The effect is
//! not predicted by a second parser: planning runs the desktop/CLI ledger
//! import (with the wallet's creation when asked) inside a savepoint and rolls
//! it back; applying runs it the same way and commits only when the
//! recomputed digest equals the approved one. The run is an ordinary import
//! batch that `imports rollback` can undo.
//!
//! Only file-backed wallets (`exchange-export`, `file-import`, `manual`) or a
//! new `custom` wallet may be targeted, and anything ambiguous blocks:
//! unrecognized columns, rejected rows, amounts whose unit the file does not
//! state, row kinds taken from the amount sign without the user's
//! confirmation, and an import with no new row.
use super::document::files::Temp;
use super::{analysis, ledger, Format, Identity, Request};
use crate::completeness;
use crate::lifecycle::{CreationIdentity, Operation};
use crate::{database_error, resolve};
use kassiber_envelope::{dict, get, py_str, s, truthy, AppError};
use kassiber_pyjson::{Key, Value as Py};
use kassiber_store::rusqlite::{Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::io::Read;

pub const PLAN_SCHEMA_VERSION: i64 = 1;
pub const SOURCE_FORMAT: &str = "generic_ledger";
pub const NEW_WALLET_KIND: &str = "custom";
pub const MAX_WALLET_LABEL_CHARS: usize = 120;
const SAVEPOINT: &str = "attachment_import_plan";
const PLAN_TOOL: &str = "ui_wallets_import_attachment_plan";
const UNKNOWN_OCCURRED_AT: &str = "1970-01-01T00:00:00Z";
/// Blockers that leave nothing to simulate.
const NO_SIMULATION: [&str; 6] = [
    "attachment_not_tabular",
    "header_is_preamble",
    "columns_not_recognized",
    "rows_rejected",
    "target_not_file_wallet",
    "wallet_label_taken",
];

/// A wallet's id and label.
pub type WalletRef = (String, String);

/// One plan or apply request.
pub struct AttachmentImport<'a> {
    pub profile_id: &'a str,
    /// The chat grant's staged file; never a model-supplied path.
    pub source_file: &'a str,
    /// `wallet` or `new_wallet`, `column_map`, `transfers_only_confirmed`.
    pub request: &'a Py,
    /// `plan_digest`, `expected_content_sha256`, `expected_input_version`
    /// from an earlier plan; `None` plans.
    pub expected: Option<&'a Py>,
}

/// Test-harness entropy: the import's ids and clock and the new wallet's.
#[derive(Default)]
pub struct Identities<'a> {
    pub import: Option<&'a Identity>,
    pub wallet: Option<&'a CreationIdentity>,
}

/// `_canonical(value)`: sorted keys, compact separators, ASCII.
pub fn canonical(value: &Py) -> Result<String, AppError> {
    kassiber_pyjson::dumps(
        value,
        &kassiber_pyjson::Options::new()
            .sort_keys(true)
            .separators(",", ":"),
    )
    .map_err(|error| AppError::new("internal_error", error.to_string()))
}

/// `_digest(value)`: SHA-256 of the canonical JSON, lowercase hex.
pub fn digest(value: &Py) -> Result<String, AppError> {
    Ok(Sha256::digest(canonical(value)?.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// `input_version(conn, profile_id)`.
pub fn input_version(conn: &Connection, profile_id: &str) -> Result<i64, AppError> {
    let version: Option<Option<i64>> = conn
        .query_row(
            "SELECT journal_input_version FROM profiles WHERE id = ?",
            [profile_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(database_error)?;
    Ok(version.flatten().unwrap_or(0))
}

/// `_blocker(code, message, **remediation)`.
pub fn blocker(code: &str, message: &str, remediation: Vec<(&str, Py)>) -> Py {
    let mut fields = vec![("code", s(code)), ("message", s(message))];
    if !remediation.is_empty() {
        fields.push(("remediation", dict(remediation)));
    }
    dict(fields)
}

fn validation(message: impl Into<String>) -> AppError {
    AppError::new("validation", message)
}

/// The target `_normalize_target` admits.
pub enum Target {
    Existing(String),
    Create {
        label: String,
        account: Option<String>,
    },
}

fn present(value: Option<&Py>) -> Option<&Py> {
    value.filter(|value| **value != Py::Null)
}

/// `_normalize_target(request)`.
pub fn normalize_target(request: &Py) -> Result<Target, AppError> {
    let wallet = present(get(request, "wallet"));
    let new_wallet = present(get(request, "new_wallet"));
    if wallet.is_none() == new_wallet.is_none() {
        return Err(
            validation("Name exactly one target: an existing wallet or a new wallet")
                .with_hint("Pass `wallet` (id or label) or `new_wallet` with a label."),
        );
    }
    if let Some(wallet) = wallet {
        return match wallet {
            Py::Str(_) if !kassiber_pytext::strip(&py_str(wallet)).is_empty() => Ok(
                Target::Existing(kassiber_pytext::strip(&py_str(wallet)).to_owned()),
            ),
            _ => Err(validation("wallet must be a non-empty string")),
        };
    }
    let new_wallet = new_wallet.expect("one target");
    if !matches!(new_wallet, Py::Dict(_)) {
        return Err(validation("new_wallet must be an object"));
    }
    let label = match get(new_wallet, "label") {
        Some(label @ Py::Str(_)) => {
            let text = py_str(label);
            let stripped = kassiber_pytext::strip(&text).to_owned();
            let length = stripped.chars().count();
            (1..=MAX_WALLET_LABEL_CHARS)
                .contains(&length)
                .then_some(stripped)
        }
        _ => None,
    };
    let Some(label) = label else {
        return Err(validation(format!(
            "new_wallet.label must be 1 to {MAX_WALLET_LABEL_CHARS} characters"
        )));
    };
    let account = match present(get(new_wallet, "account")) {
        None => None,
        Some(account @ Py::Str(_)) if !kassiber_pytext::strip(&py_str(account)).is_empty() => {
            Some(kassiber_pytext::strip(&py_str(account)).to_owned())
        }
        Some(_) => return Err(validation("new_wallet.account must be a non-empty string")),
    };
    Ok(Target::Create { label, account })
}

/// `_analysis_blockers(analysis, transfers_only_confirmed=...)`.
fn analysis_blockers(analysis: &Py, transfers_only_confirmed: bool) -> (Vec<Py>, Vec<Py>) {
    let mut blockers = Vec::new();
    let mut warnings = Vec::new();
    let flag = |key: &str| get(analysis, key).is_some_and(truthy);
    if get(analysis, "route") != Some(&s("generic_ledger")) {
        blockers.push(blocker(
            "attachment_not_tabular",
            "Photos and PDFs are read by the on-device document importer, not this tool.",
            vec![
                ("action", s("document_import")),
                ("desktop_only", Py::Bool(true)),
            ],
        ));
        return (blockers, warnings);
    }
    if flag("header_is_preamble") {
        blockers.push(blocker(
            "header_is_preamble",
            "The first row is a preamble, not column names; the user must remove it.",
            vec![
                ("action", s("strip_preamble")),
                ("desktop_only", Py::Bool(true)),
            ],
        ));
    } else if !flag("confident") {
        blockers.push(blocker(
            "columns_not_recognized",
            "The columns were not recognized. Map them with column_map first.",
            vec![
                ("action", s("supply_column_map")),
                ("tool", s("ui_wallets_analyze_file")),
            ],
        ));
    }
    let errors = match get(analysis, "errors") {
        Some(Py::Int(count)) => count.to_string(),
        _ => "0".into(),
    };
    if errors != "0" {
        blockers.push(blocker(
            "rows_rejected",
            &format!("{errors} row(s) would be rejected; the import stops at the first one."),
            vec![
                ("action", s("fix_mapping")),
                ("tool", s("ui_wallets_analyze_file")),
            ],
        ));
    }
    if flag("amount_units_unconfirmed") {
        blockers.push(blocker(
            "amount_units_unconfirmed",
            "The file does not state the amount unit and the amounts are too large to be BTC. The user must set the unit in the desktop or CLI import.",
            vec![
                ("action", s("confirm_amount_units")),
                ("desktop_only", Py::Bool(true)),
            ],
        ));
    }
    if flag("row_kinds_from_amount_sign") {
        if transfers_only_confirmed {
            warnings.push(s("transfers_only_confirmed"));
        } else {
            blockers.push(blocker(
                "row_kinds_from_amount_sign",
                "No Type or direction column: every row would be a plain deposit or withdrawal. Map the Type column, or set transfers_only_confirmed after the user confirms the export is transfers only.",
                vec![
                    ("action", s("map_row_kinds")),
                    ("tool", s("ui_wallets_analyze_file")),
                ],
            ));
        }
    }
    (blockers, warnings)
}

/// `completeness_class(kind, config)` of a wallet row.
fn wallet_class(conn: &Connection, wallet_id: &str) -> Result<Py, AppError> {
    let described = describe_one(conn, None, wallet_id)?;
    Ok(described.map_or(s("untracked"), |(klass, _)| klass))
}

fn describe_one(
    conn: &Connection,
    profile_id: Option<&str>,
    wallet_id: &str,
) -> Result<Option<(Py, Py)>, AppError> {
    let row: Option<(String, Py, Py)> = conn
        .query_row(
            "SELECT id, kind, config_json, profile_id FROM wallets WHERE id = ?",
            [wallet_id],
            |row| {
                let text = |index| -> kassiber_store::rusqlite::Result<Py> {
                    Ok(row.get::<_, Option<String>>(index)?.map_or(Py::Null, s))
                };
                Ok((row.get::<_, String>(3)?, text(1)?, text(2)?))
            },
        )
        .optional()
        .map_err(database_error)?;
    let Some((owner, kind, config_json)) = row else {
        return Ok(None);
    };
    let described = completeness::describe(
        conn,
        profile_id.unwrap_or(&owner),
        &[completeness::Wallet {
            id: wallet_id.to_owned(),
            kind,
            config_json,
        }],
    )?;
    Ok(described.get(wallet_id).cloned())
}

/// `completeness_class(kind, {"source_format": source_format})` of a new
/// wallet.
pub fn new_wallet_class(source_format: &str) -> &'static str {
    completeness::class_of(NEW_WALLET_KIND, source_format)
}

/// The target as the plan describes it, its blockers, and the existing
/// wallet's id and label (`_resolve_target`). `plan_tool` names the tool the
/// target blockers send the model back to.
pub fn resolve_target(
    conn: &Connection,
    profile_id: &str,
    target: &Target,
    plan_tool: &str,
    source_format: &str,
) -> Result<(Py, Vec<Py>, Option<WalletRef>), AppError> {
    match target {
        Target::Existing(reference) => {
            let wallet = resolve(conn, profile_id, reference)?;
            let klass = wallet_class(conn, &wallet.id)?;
            let kind: Option<String> = conn
                .query_row("SELECT kind FROM wallets WHERE id = ?", [&wallet.id], |r| {
                    r.get(0)
                })
                .map_err(database_error)?;
            let described = dict([
                ("action", s("use_existing")),
                ("wallet_id", s(wallet.id.as_str())),
                ("label", s(wallet.label.as_str())),
                ("kind", kind.map_or(Py::Null, s)),
                ("completeness_class", klass.clone()),
            ]);
            let mut blockers = Vec::new();
            let klass = py_str(&klass);
            if !completeness::FILE_IMPORT_TARGET_CLASSES.contains(&klass.as_str()) {
                blockers.push(blocker(
                    "target_not_file_wallet",
                    &format!(
                        "'{}' is a {klass} wallet; its history comes from its own source. Import the file into a file or exchange wallet instead.",
                        wallet.label
                    ),
                    vec![("action", s("choose_file_wallet")), ("tool", s(plan_tool))],
                ));
            }
            Ok((described, blockers, Some((wallet.id, wallet.label))))
        }
        Target::Create { label, account } => {
            let described = dict([
                ("action", s("create")),
                ("wallet_id", Py::Null),
                ("label", s(label.as_str())),
                ("kind", s(NEW_WALLET_KIND)),
                ("account", account.as_deref().map_or(Py::Null, s)),
                ("completeness_class", s(new_wallet_class(source_format))),
            ]);
            let taken = conn
                .query_row(
                    "SELECT 1 FROM wallets WHERE profile_id = ? AND label = ?",
                    [profile_id, label.as_str()],
                    |_| Ok(()),
                )
                .optional()
                .map_err(database_error)?
                .is_some();
            let mut blockers = Vec::new();
            if taken {
                blockers.push(blocker(
                    "wallet_label_taken",
                    &format!(
                        "A wallet named '{label}' already exists; target it or pick another label."
                    ),
                    vec![("action", s("choose_target")), ("tool", s(plan_tool))],
                ));
            }
            Ok((described, blockers, None))
        }
    }
}

/// `create_wallet(conn, None, None, label, "custom", account_ref=...,
/// config={"source_format": ...}, commit=False)`: the new wallet's id and
/// label, inside the caller's savepoint.
pub fn create_target_wallet(
    conn: &Connection,
    profile_id: &str,
    target: &Target,
    source_format: &str,
    identity: Option<&CreationIdentity>,
) -> Result<WalletRef, AppError> {
    let Target::Create { label, account } = target else {
        unreachable!("only a new wallet is created");
    };
    let workspace_id: String = conn
        .query_row(
            "SELECT workspace_id FROM profiles WHERE id = ?",
            [profile_id],
            |r| r.get(0),
        )
        .map_err(database_error)?;
    let created = crate::lifecycle::execute_with_creation_identity(
        conn,
        Operation::Create {
            workspace_id,
            profile_id: profile_id.to_owned(),
            label: label.clone(),
            kind: NEW_WALLET_KIND.into(),
            account: account.clone(),
            config: serde_json::json!({ "source_format": source_format }),
        },
        identity,
    )?;
    let text = |key| get(&created, key).map(py_str).unwrap_or_default();
    Ok((text("id"), text("label")))
}

/// `_batch_effects(conn, profile_id, wallet_id, batch_id)`.
pub fn batch_effects(
    conn: &Connection,
    profile_id: &str,
    wallet_id: &str,
    batch_id: Option<&str>,
) -> Result<Vec<(&'static str, Py)>, AppError> {
    let Some(batch) = batch_id.filter(|batch| !batch.is_empty()) else {
        return Ok(vec![
            (
                "date_range",
                dict([
                    ("first", Py::Null),
                    ("last", Py::Null),
                    ("undated", Py::from(0)),
                ]),
            ),
            ("totals", Py::List(vec![])),
            (
                "overlap",
                dict([
                    ("existing_rows_in_date_range", Py::from(0)),
                    ("external_ids_in_other_wallets", Py::from(0)),
                ]),
            ),
        ]);
    };
    let rows = "SELECT t.* FROM transactions t JOIN import_batch_transactions b \
                ON b.transaction_id = t.id WHERE b.batch_id = ?";
    let (first, last): (Option<String>, Option<String>) = conn
        .query_row(
            &format!(
                "SELECT MIN(occurred_at) AS first, MAX(occurred_at) AS last FROM ({rows}) WHERE occurred_at != ?"
            ),
            [batch, UNKNOWN_OCCURRED_AT],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(database_error)?;
    let count =
        |sql: &str, params: &[&dyn kassiber_store::rusqlite::ToSql]| -> Result<i64, AppError> {
            conn.query_row(sql, params, |r| r.get(0))
                .map_err(database_error)
        };
    let undated = count(
        &format!("SELECT COUNT(*) AS n FROM ({rows}) WHERE occurred_at = ?"),
        &[&batch, &UNKNOWN_OCCURRED_AT],
    )?;
    let mut statement = conn
        .prepare(&format!(
            "SELECT asset, direction, COUNT(*) AS n, SUM(amount) AS amount, SUM(fee) AS fee \
             FROM ({rows}) GROUP BY asset, direction ORDER BY asset, direction"
        ))
        .map_err(database_error)?;
    let totals = statement
        .query_map([batch], |r| {
            Ok(dict([
                ("asset", r.get::<_, Option<String>>(0)?.map_or(Py::Null, s)),
                (
                    "direction",
                    r.get::<_, Option<String>>(1)?.map_or(Py::Null, s),
                ),
                ("rows", Py::from(r.get::<_, i64>(2)?)),
                (
                    "amount_msat",
                    Py::from(r.get::<_, Option<i64>>(3)?.unwrap_or(0)),
                ),
                (
                    "fee_msat",
                    Py::from(r.get::<_, Option<i64>>(4)?.unwrap_or(0)),
                ),
            ]))
        })
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)?;
    let in_range = match (&first, &last) {
        (Some(first), Some(last)) => count(
            "SELECT COUNT(*) AS n FROM transactions \
             WHERE profile_id = ? AND wallet_id = ? AND excluded = 0 \
               AND occurred_at BETWEEN ? AND ? \
               AND id NOT IN (SELECT transaction_id FROM import_batch_transactions WHERE batch_id = ?)",
            &[&profile_id, &wallet_id, first, last, &batch],
        )?,
        _ => 0,
    };
    let elsewhere = count(
        &format!(
            "SELECT COUNT(DISTINCT mine.external_id) AS n FROM ({rows}) mine \
             JOIN transactions other \
               ON other.profile_id = ? AND other.external_id = mine.external_id \
              AND other.wallet_id != mine.wallet_id \
             WHERE mine.external_id IS NOT NULL AND mine.external_id != ''"
        ),
        &[&batch, &profile_id],
    )?;
    let text = |value: &Option<String>| value.as_deref().map_or(Py::Null, s);
    Ok(vec![
        (
            "date_range",
            dict([
                ("first", text(&first)),
                ("last", text(&last)),
                ("undated", Py::from(undated)),
            ]),
        ),
        ("totals", Py::List(totals)),
        (
            "overlap",
            dict([
                ("existing_rows_in_date_range", Py::from(in_range)),
                ("external_ids_in_other_wallets", Py::from(elsewhere)),
            ]),
        ),
    ])
}

/// `_file_snapshot(source_file)`: one read of the staged file's bytes,
/// hashed and copied under its own basename into a private directory.
struct FileSnapshot {
    path: std::path::PathBuf,
    sha256: String,
    size: usize,
    _temp: Temp,
}

fn snapshot(source_file: &str) -> Result<FileSnapshot, AppError> {
    let limit = analysis::MAX_ANALYZE_BYTES;
    let mut payload = Vec::new();
    std::fs::File::open(source_file)
        .and_then(|file| file.take(limit + 1).read_to_end(&mut payload))
        .map_err(|_| {
            AppError::new(
                "ai_attachment_missing",
                "The attached file can no longer be read",
            )
            .with_hint("Ask the user to attach the file again.")
        })?;
    if payload.len() as u64 > limit {
        return Err(validation("The attached file is too large to import")
            .with_details(dict([("max_bytes", Py::from(limit as i64))])));
    }
    let sha256 = Sha256::digest(&payload)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let temp = Temp::new("import")?;
    let name = basename(source_file);
    let path = temp
        .path()
        .join(if name.is_empty() { "import.csv" } else { name });
    std::fs::write(&path, &payload).map_err(super::document::files::io)?;
    Ok(FileSnapshot {
        path,
        sha256,
        size: payload.len(),
        _temp: temp,
    })
}

/// `os.path.basename(path)`.
pub fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn replan_details(extra: Vec<(&str, Py)>, tool: &str) -> Py {
    let mut fields = extra;
    fields.push((
        "remediation",
        dict([
            ("action", s("replan")),
            ("tool", s(tool)),
            ("target", dict::<&str>([])),
            ("desktop_only", Py::Bool(false)),
        ]),
    ));
    dict(fields)
}

/// `import_plan_stale` for a book that moved since the plan.
pub fn book_changed(expected: &Py, version: i64, tool: &str) -> AppError {
    AppError::new(
        "import_plan_stale",
        "The book changed since the import was planned",
    )
    .with_hint("Plan the import again against the current book.")
    .with_details(replan_details(
        vec![
            (
                "expected_input_version",
                get(expected, "expected_input_version")
                    .cloned()
                    .unwrap_or(Py::Null),
            ),
            ("current_input_version", Py::from(version)),
        ],
        tool,
    ))
}

/// `import_plan_blocked` and the digest refusal an apply raises.
pub fn apply_refusal(plan: &Py, plan_digest: &str, expected: &Py, tool: &str) -> Option<AppError> {
    if matches!(get(plan, "blockers"), Some(Py::List(blockers)) if !blockers.is_empty()) {
        return Some(
            AppError::new("import_plan_blocked", "This import is blocked").with_details(dict([(
                "blockers",
                get(plan, "blockers").cloned().unwrap_or(Py::Null),
            )])),
        );
    }
    if get(expected, "plan_digest") != Some(&s(plan_digest)) {
        return Some(
            AppError::new(
                "import_plan_stale",
                "The import no longer matches the approved plan",
            )
            .with_hint("Plan the import again and ask for consent again.")
            .with_details(replan_details(vec![], tool)),
        );
    }
    None
}

fn int(value: Option<&Py>) -> i64 {
    match value {
        Some(Py::Int(number)) => i64::try_from(number.clone()).unwrap_or(0),
        Some(Py::Bool(flag)) => i64::from(*flag),
        _ => 0,
    }
}

fn list_len(value: Option<&Py>) -> usize {
    match value {
        Some(Py::List(items)) => items.len(),
        _ => 0,
    }
}

/// `plan_or_apply(conn, profile, source_file=..., request=..., expected=...)`.
pub fn plan_or_apply(
    conn: &Connection,
    request: AttachmentImport<'_>,
    identities: &Identities<'_>,
) -> Result<Py, AppError> {
    let AttachmentImport {
        profile_id,
        source_file,
        request,
        expected,
    } = request;
    let target = normalize_target(request)?;
    let column_map = match get(request, "column_map") {
        None | Some(Py::Null) => None,
        Some(map @ Py::Dict(_)) => Some(map.clone()),
        Some(_) => return Err(validation("column_map must be an object")),
    };
    let transfers_only_confirmed =
        get(request, "transfers_only_confirmed") == Some(&Py::Bool(true));
    let version = input_version(conn, profile_id)?;
    let file = snapshot(source_file)?;
    if let Some(expected) = expected {
        if get(expected, "expected_content_sha256") != Some(&s(file.sha256.as_str())) {
            return Err(AppError::new(
                "attachment_changed",
                "The attached file changed since the import was planned",
            )
            .with_hint("Plan the import again from the current file and ask for consent again.")
            .with_details(replan_details(vec![], PLAN_TOOL)));
        }
        if !crate::file_import::attachment::same_int(
            get(expected, "expected_input_version"),
            version,
        ) {
            return Err(book_changed(expected, version, PLAN_TOOL));
        }
    }
    let mapping = column_map
        .as_ref()
        .map(ledger::ColumnMap::from_python)
        .transpose()?;
    let shown = file.path.to_string_lossy().into_owned();
    let analysis = analysis::analyze(&shown, mapping.as_ref(), 0)?;
    let (mut blockers, mut warnings) = analysis_blockers(&analysis, transfers_only_confirmed);
    let (described, target_blockers, existing) =
        resolve_target(conn, profile_id, &target, PLAN_TOOL, SOURCE_FORMAT)?;
    blockers.extend(target_blockers);
    if column_map
        .as_ref()
        .and_then(|map| get(map, "type_map"))
        .is_some_and(truthy)
    {
        warnings.push(s("type_map_decides_tax_kind"));
    }
    let mut rows = vec![
        ("in_file", Py::from(int(get(&analysis, "rows_read")))),
        ("to_import", Py::from(0)),
        ("already_present", Py::from(0)),
        ("would_update_existing", Py::from(0)),
    ];
    let mut effects = batch_effects(conn, profile_id, "", None)?;
    let mut wallet_after = Py::Null;
    let simulate = !blockers.iter().any(|blocker| {
        get(blocker, "code").is_some_and(|code| NO_SIMULATION.iter().any(|name| *code == s(*name)))
    });
    let workspace_id: String = conn
        .query_row(
            "SELECT workspace_id FROM profiles WHERE id = ?",
            [profile_id],
            |r| r.get(0),
        )
        .map_err(database_error)?;
    kassiber_store::begin_write_savepoint(conn, SAVEPOINT).map_err(database_error)?;
    let mut apply_now = false;
    let planned = (|| -> Result<(Py, String, Py, Option<WalletRef>), AppError> {
        let mut outcome = Py::Null;
        let mut wallet = existing.clone();
        if simulate {
            let run = (|| -> Result<Py, AppError> {
                if wallet.is_none() {
                    wallet = Some(create_target_wallet(
                        conn,
                        profile_id,
                        &target,
                        SOURCE_FORMAT,
                        identities.wallet,
                    )?);
                }
                let (wallet_id, _) = wallet.as_ref().expect("target wallet");
                super::execute_with_ledger_map(
                    conn,
                    Request {
                        profile_id: profile_id.to_owned(),
                        wallet: wallet_id.clone(),
                        file: shown.clone(),
                        format: Format::Ledger,
                        mode: None,
                    },
                    mapping.as_ref(),
                    identities.import,
                )
            })();
            match run {
                Ok(result) => outcome = result,
                // A non-`AppError` failure escapes Python's `except AppError`.
                Err(error) if error.code == "internal_error" => return Err(error),
                Err(error) => {
                    let code = if error.code.is_empty() {
                        "import_failed".to_owned()
                    } else {
                        error.code.clone()
                    };
                    blockers.push(blocker(&code, &error.message, vec![]));
                }
            }
        }
        if truthy(&outcome) {
            let updated = list_len(get(&outcome, "updated_records"));
            let to_import = int(get(&outcome, "imported"));
            let already_present = int(get(&outcome, "unchanged"));
            rows[1].1 = Py::from(to_import);
            rows[2].1 = Py::from(already_present);
            rows[3].1 = Py::from(updated as i64);
            let (wallet_id, _) = wallet.as_ref().expect("simulated wallet");
            let batch = get(&outcome, "import_batch_id").map(py_str);
            effects = batch_effects(conn, profile_id, wallet_id, batch.as_deref())?;
            let (klass, bounds) =
                describe_one(conn, Some(profile_id), wallet_id)?.expect("simulated wallet row");
            let scan_warnings = get(&bounds, "warnings").cloned();
            wallet_after = dict([("completeness_class", klass), ("scan_bounds", bounds)]);
            if updated > 0 {
                // The importer re-prices a row it already holds from the
                // file's own execution price. A rollback deletes only rows the
                // run created, so this part would stay; say so.
                warnings.push(s("updates_existing_rows_not_rolled_back"));
            }
            if to_import == 0 {
                // Without a new row there is no import run to roll back, so
                // an enrichment-only import stays a local decision.
                blockers.push(blocker(
                    "nothing_to_import",
                    "No new rows: every row is already in this wallet.",
                    vec![
                        (
                            "action",
                            s(if updated > 0 {
                                "import_locally"
                            } else {
                                "none"
                            }),
                        ),
                        ("desktop_only", Py::Bool(updated > 0)),
                    ],
                ));
            }
            if already_present > 0 {
                warnings.push(s("rows_already_present"));
            }
            let overlap = &effects[2].1;
            if int(get(overlap, "existing_rows_in_date_range")) != 0 {
                warnings.push(s("overlaps_existing_history"));
            }
            if int(get(overlap, "external_ids_in_other_wallets")) != 0 {
                warnings.push(s("external_ids_in_other_wallets"));
            }
            if matches!(scan_warnings, Some(Py::List(items)) if items.contains(&s("history_starts_after_first_funding")))
            {
                warnings.push(s("history_starts_after_first_funding"));
            }
        }
        let applied_plan = get(&analysis, "applied_plan").filter(|plan| truthy(plan));
        let mut plan = vec![
            ("schema_version", Py::from(PLAN_SCHEMA_VERSION)),
            ("workspace_id", s(workspace_id.as_str())),
            ("profile_id", s(profile_id)),
            ("input_version", Py::from(version)),
            (
                "attachment",
                dict([
                    ("content_sha256", s(file.sha256.as_str())),
                    ("size_bytes", Py::from(file.size as i64)),
                    (
                        "kind",
                        get(&analysis, "source")
                            .and_then(|source| get(source, "kind"))
                            .cloned()
                            .unwrap_or(Py::Null),
                    ),
                    ("filename", s(basename(source_file))),
                ]),
            ),
            ("source_format", s(SOURCE_FORMAT)),
            ("column_map", column_map.clone().unwrap_or(Py::Null)),
            (
                "effective_column_plan",
                applied_plan
                    .or_else(|| get(&analysis, "inferred_plan"))
                    .cloned()
                    .unwrap_or(Py::Null),
            ),
            (
                "transfers_only_confirmed",
                Py::Bool(transfers_only_confirmed),
            ),
            ("target", described.clone()),
            ("rows", dict(rows.clone())),
        ];
        plan.extend(effects.iter().cloned());
        let ready = blockers.is_empty();
        plan.extend([
            ("wallet_after", wallet_after.clone()),
            ("warnings", Py::List(warnings.clone())),
            ("blockers", Py::List(blockers.clone())),
            ("ready", Py::Bool(ready)),
        ]);
        let plan = dict(plan);
        let plan_digest = digest(&plan)?;
        if let Some(expected) = expected {
            if let Some(refusal) = apply_refusal(&plan, &plan_digest, expected, PLAN_TOOL) {
                return Err(refusal);
            }
            apply_now = true;
        }
        Ok((plan, plan_digest, outcome, wallet))
    })();
    if apply_now && planned.is_ok() {
        conn.execute_batch(&format!("RELEASE SAVEPOINT {SAVEPOINT}"))
            .map_err(database_error)?;
    } else {
        conn.execute_batch(&format!(
            "ROLLBACK TO SAVEPOINT {SAVEPOINT}; RELEASE SAVEPOINT {SAVEPOINT}"
        ))
        .map_err(database_error)?;
    }
    let (plan, plan_digest, outcome, wallet) = planned?;
    if !apply_now {
        let mut payload = match plan {
            Py::Dict(pairs) => pairs,
            _ => unreachable!("plan is a dict"),
        };
        payload.push((Key::from("plan_digest"), s(plan_digest.as_str())));
        payload.push((
            Key::from("apply_with"),
            dict([
                ("plan_digest", s(plan_digest.as_str())),
                ("expected_content_sha256", s(file.sha256.as_str())),
                ("expected_input_version", Py::from(version)),
            ]),
        ));
        return Ok(Py::Dict(payload));
    }
    let (wallet_id, wallet_label) = wallet.expect("applied wallet");
    let batch = get(&outcome, "import_batch_id")
        .cloned()
        .unwrap_or(Py::Null);
    let mut result = vec![
        ("applied", Py::Bool(true)),
        ("plan_digest", s(plan_digest)),
        ("import_batch_id", batch.clone()),
        (
            "wallet",
            dict([
                ("id", s(wallet_id)),
                ("label", s(wallet_label)),
                ("created", Py::Bool(matches!(target, Target::Create { .. }))),
            ]),
        ),
        ("rows", get(&plan, "rows").cloned().unwrap_or(Py::Null)),
    ];
    result.extend(effects);
    result.extend([
        (
            "warnings",
            get(&plan, "warnings").cloned().unwrap_or(Py::Null),
        ),
        (
            "journals_invalidated",
            Py::Bool(get(&outcome, "journal_invalidated").is_some_and(truthy)),
        ),
        (
            "rollback",
            dict([("batch", batch), ("tool", s("ui_imports_rollback_plan"))]),
        ),
    ]);
    Ok(dict(result))
}

/// `expected.get(key) != version` for an integer version: Python compares a
/// JSON `7`, `7.0` or `True` (for 1) equal to it.
pub fn same_int(value: Option<&Py>, version: i64) -> bool {
    match value {
        Some(Py::Int(number)) => i64::try_from(number.clone()).ok() == Some(version),
        Some(Py::Bool(flag)) => i64::from(*flag) == version,
        Some(Py::Float(number)) => *number == version as f64,
        _ => false,
    }
}
