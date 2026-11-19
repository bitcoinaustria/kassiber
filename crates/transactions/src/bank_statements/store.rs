//! Statement imports: the file becomes a `statement` document with its
//! attachment, its lines become evidence rows, and each import can be rolled
//! back. Nothing here touches transactions, journals or balances.
use super::{
    account_short, invalid, line_preview, mask_account, mask_ibans, parse, Money, ParseOptions,
    ParsedStatement, Result,
};
use crate::sql::{atomic, now, rows, uuid, Scope};
use kassiber_envelope::{dict, get, s, AppError};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, path::Path};

pub struct ImportRequest {
    pub file: String,
    pub options: ParseOptions,
    pub label: Option<String>,
    pub dry_run: bool,
}

pub struct LineFilter {
    /// `out`, `in` or `all`.
    pub direction: String,
    pub document: Option<String>,
    pub limit: i64,
}

pub(crate) fn store_error(e: kassiber_store::Error) -> AppError {
    AppError::new(e.code, e.message).retryable(e.retryable)
}

/// Whether this book has bank-statement tables yet (created on first import).
pub fn tables_present(conn: &Connection) -> Result<bool> {
    kassiber_store::optional::bank_statements_present(conn).map_err(store_error)
}

fn text(row: &Value, key: &str) -> Option<String> {
    match get(row, key) {
        Some(Value::Str(t)) => t.as_str().map(str::to_owned),
        _ => None,
    }
}

fn opt(value: Option<&str>) -> Value {
    value.map_or(Value::Null, s)
}

fn int(n: usize) -> Value {
    Value::Int(n.into())
}

fn hex(bytes: impl AsRef<[u8]>) -> String {
    bytes.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// SHA-256 over separated parts.
pub fn content_hash(parts: &[&str]) -> String {
    let mut digest = Sha256::new();
    digest.update(b"kassiber-bank-statement-v3");
    for part in parts {
        digest.update(b"\x1f");
        digest.update(part.as_bytes());
    }
    hex(digest.finalize())
}

/// The identity group of each line: own account, booking date, exact amount
/// and currency, direction and the bank's reference when there is one.
pub fn dedupe_groups(statement: &ParsedStatement) -> Vec<String> {
    statement
        .lines
        .iter()
        .map(|l| {
            let direction = if l.amount.is_negative() { "out" } else { "in" };
            let amount = l.amount.canonical();
            let parts = [
                l.account.as_deref().unwrap_or(""),
                &l.booking_date,
                &amount,
                &l.currency,
                direction,
            ];
            content_hash(&[&parts[..], &[l.bank_reference.as_deref().unwrap_or("")]].concat())
        })
        .collect()
}

const COMPARED: [&str; 5] = [
    "value date",
    "counterparty",
    "counterparty IBAN",
    "remittance text",
    "end-to-end reference",
];

/// What an import does with each line: `Some(key)` inserts it, `None` means
/// the book holds the same booking (equal on every field and carrying the
/// bank's own reference). A reference-free line equal to a stored one, or a
/// line whose group holds only different lines, is refused by name: Kassiber
/// never skips or inserts a line it cannot place.
fn plan_lines(
    conn: &Connection,
    scope: &Scope<'_>,
    statement: &ParsedStatement,
    tables: bool,
) -> Result<(Vec<String>, Vec<Option<String>>)> {
    let groups = dedupe_groups(statement);
    let mut stored: BTreeMap<&String, Vec<[String; 5]>> = BTreeMap::new();
    for group in &groups {
        if !tables || stored.contains_key(group) {
            continue;
        }
        let found = rows(
            conn,
            "SELECT value_date,counterparty_name,counterparty_iban_masked,remittance,end_to_end_id FROM bank_statement_lines WHERE profile_id=? AND dedupe_group=? ORDER BY dedupe_key",
            params![scope.profile_id, group],
        )?;
        let fields = |row: &Value| {
            [
                "value_date",
                "counterparty_name",
                "counterparty_iban_masked",
                "remittance",
                "end_to_end_id",
            ]
            .map(|k| text(row, k).unwrap_or_default())
        };
        stored.insert(group, found.iter().map(fields).collect());
    }
    let mut next: BTreeMap<&String, usize> = stored.iter().map(|(g, l)| (*g, l.len())).collect();
    // Lines of this file carrying the bank's reference, by group: the same
    // booking twice in one file is one line, as it is across files, and two
    // versions of it are refused.
    let mut referenced: BTreeMap<&String, [String; 5]> = BTreeMap::new();
    let mut keys = Vec::with_capacity(groups.len());
    for (line, group) in statement.lines.iter().zip(&groups) {
        let fields = [
            line.value_date.as_deref(),
            line.counterparty_name.as_deref(),
            None,
            line.remittance.as_deref(),
            line.end_to_end_id.as_deref(),
        ]
        .map(|v| v.unwrap_or("").to_owned());
        let mut fields = fields;
        fields[2] = line
            .counterparty_iban
            .as_deref()
            .map(mask_account)
            .unwrap_or_default();
        let at = (
            line.position,
            &line.booking_date,
            line.amount.canonical(),
            &line.currency,
        );
        if line.bank_reference.is_some() {
            match referenced.get(group) {
                Some(first) if *first == fields => {
                    keys.push(None);
                    continue;
                }
                Some(first) => {
                    let field = (0..5)
                        .find(|i| first[*i] != fields[*i])
                        .map_or("a field", |i| COMPARED[i]);
                    return Err(invalid(format!(
                        "Line {}: the file has another line with the same account, date, amount, currency and reference, but its {field} differs",
                        at.0
                    ))
                    .with_hint("Check the statement; Kassiber does not guess which line is right.")
                    .with_details(dict([("reason", s("bank_statement_line_conflict")), ("position", int(at.0)), ("field", s(field))])));
                }
                None => {
                    referenced.insert(group, fields.clone());
                }
            }
        }
        let candidates = stored.get(group).map_or(&[][..], Vec::as_slice);
        if candidates.is_empty() {
            let n = next.entry(group).or_insert(0);
            *n += 1;
            keys.push(Some(content_hash(&[group, &n.to_string()])));
            continue;
        }
        if candidates.contains(&fields) {
            if line.bank_reference.is_some() || line.end_to_end_id.is_some() {
                keys.push(None);
                continue;
            }
            let in_file = groups.iter().filter(|g| *g == group).count();
            return Err(invalid(format!(
                "Line {}: two transfers on {} for {} {} with identical details and no bank reference; Kassiber can't tell whether this is the same transfer (the book holds {}, this file {in_file})",
                at.0, at.1, at.2, at.3, candidates.len(),
            ))
            .with_hint("Import the statement that already covers this day again unchanged, or a statement whose lines carry the bank's references.")
            .with_details(dict([("reason", s("bank_statement_line_ambiguous")), ("position", int(at.0))])));
        }
        let field = (0..5)
            .find(|i| candidates[0][*i] != fields[*i])
            .map_or("a field", |i| COMPARED[i]);
        return Err(invalid(format!(
            "Line {}: the book already has a line with the same account, date, amount, currency and reference, but its {field} differs",
            at.0
        ))
        .with_hint("Check the two statements; Kassiber does not guess which one is right.")
        .with_details(dict([("reason", s("bank_statement_line_conflict")), ("position", int(at.0)), ("field", s(field))])));
    }
    Ok((groups, keys))
}

/// A recorded import as a read returns it.
fn import_payload(row: &Value) -> Value {
    dict(
        [
            "id",
            "document_id",
            "attachment_id",
            "source_format",
            "account_masked",
            "bank_name",
            "bank_bic",
            "statement_ref",
            "period_start",
            "period_end",
            "lines_total",
            "lines_new",
            "lines_existing",
            "lines_skipped",
            "imported_at",
        ]
        .map(|key| {
            let value = match key {
                "bank_name" | "bank_bic" | "statement_ref" => {
                    opt(text(row, key).map(|t| mask_ibans(&t)).as_deref())
                }
                _ => get(row, key).cloned().unwrap_or(Value::Null),
            };
            (key, value)
        }),
    )
}

fn sorted_json(value: &Value) -> Result<String> {
    let options = kassiber_pyjson::Options {
        sort_keys: true,
        ..Default::default()
    };
    kassiber_pyjson::dumps(value, &options).map_err(|e| invalid(e.to_string()))
}

/// Import (or preview) one statement file.
pub fn import_statement(
    conn: &Connection,
    scope: &Scope<'_>,
    attachments_root: &Path,
    request: &ImportRequest,
) -> Result<Value> {
    let path = crate::documents::expanded(&request.file)?;
    // The path often carries the account number: never echoed.
    let missing = || {
        AppError::new("not_found", "The statement file was not found")
            .with_hint("Check the path and pick the file again.")
    };
    if !path.is_file() {
        return Err(missing());
    }
    if fs::metadata(&path).map_err(|_| missing())?.len() > super::MAX_FILE_BYTES as u64 {
        return Err(invalid(format!(
            "Statement file is larger than {} MiB",
            super::MAX_FILE_BYTES / 1024 / 1024
        )));
    }
    let bytes = fs::read(&path).map_err(|_| missing())?;
    let file_sha256 = hex(Sha256::digest(&bytes));
    let statement = parse(&bytes, &request.options)?;
    let mut accounts: Vec<&str> = statement
        .lines
        .iter()
        .filter_map(|l| l.account.as_deref())
        .collect();
    accounts.sort_unstable();
    accounts.dedup();
    // The same bytes for an own account, whatever configuration reads them;
    // and the file key: bytes, accounts and how they were read.
    let account_keys: Vec<String> = accounts
        .iter()
        .map(|a| content_hash(&["account", &file_sha256, a]))
        .collect();
    let column_map = request
        .options
        .column_map
        .as_ref()
        .map(sorted_json)
        .transpose()?;
    let format = statement.format.expect("parsed format").name();
    let file_key = content_hash(&[
        "file",
        &file_sha256,
        &accounts.join("\x1e"),
        format,
        column_map.as_deref().unwrap_or(""),
        request.options.currency.as_deref().unwrap_or(""),
    ]);
    let lines = &statement.lines;
    let period_start = lines.iter().map(|l| l.booking_date.clone()).min();
    let period_end = lines.iter().map(|l| l.booking_date.clone()).max();
    let account_masked = statement.account.as_deref().map(mask_account);
    let mut copied = Vec::new();
    let result = atomic(conn, || {
        // A book gets the bank tables on its first real import.
        if !request.dry_run {
            kassiber_store::optional::ensure_bank_statement_schema(conn).map_err(store_error)?;
        }
        let tables = tables_present(conn)?;
        let existing = match tables {
            true => rows(conn, "SELECT * FROM bank_statement_imports WHERE profile_id=? AND file_key=? ORDER BY imported_at, id LIMIT 1", params![scope.profile_id, file_key])?,
            false => Vec::new(),
        };
        // The same statement read another way would place its lines anew:
        // refused, never reinterpreted.
        if tables && existing.is_empty() {
            for key in &account_keys {
                let earlier = rows(
                    conn,
                    "SELECT id FROM bank_statement_imports WHERE profile_id=? AND instr(' ' || account_keys || ' ', ' ' || ? || ' ') > 0 ORDER BY imported_at, id LIMIT 1",
                    params![scope.profile_id, key],
                )?;
                if let Some(id) = earlier.first().and_then(|r| text(r, "id")) {
                    return Err(invalid(format!("This statement was already imported for this account with a different parsing configuration (format, column mapping or currency) as import {id}; roll back that import first"))
                        .with_hint(format!("Run `kassiber documents rollback-statement --import {id} --confirm`, then import the statement again with the new mapping."))
                        .with_details(dict([("reason", s("bank_statement_reparsed")), ("import_id", s(id))])));
                }
            }
        }
        let (groups, keys) = match existing.is_empty() {
            true => plan_lines(conn, scope, &statement, tables)?,
            false => (Vec::new(), vec![None; lines.len()]),
        };
        let new_count = keys.iter().filter(|k| k.is_some()).count();
        let already = !existing.is_empty();
        let summary = |import: Value, document: Value| {
            let skipped = statement
                .skipped
                .iter()
                .take(50)
                .map(|(at, why)| dict([("position", int(*at)), ("reason", s(why.as_str()))]));
            dict([
                ("dry_run", Value::Bool(request.dry_run)),
                ("already_imported", Value::Bool(already)),
                ("format", s(format)),
                ("account_masked", opt(account_masked.as_deref())),
                (
                    "bank_name",
                    opt(statement.bank_name.as_deref().map(mask_ibans).as_deref()),
                ),
                ("currency", opt(statement.currency.as_deref())),
                ("period_start", opt(period_start.as_deref())),
                ("period_end", opt(period_end.as_deref())),
                ("lines_total", int(keys.len())),
                ("lines_new", int(if already { 0 } else { new_count })),
                (
                    "lines_existing",
                    int(if already {
                        keys.len()
                    } else {
                        keys.len() - new_count
                    }),
                ),
                ("lines_skipped", int(statement.skipped.len())),
                ("skipped", Value::List(skipped.collect())),
                (
                    "preview",
                    Value::List(lines.iter().take(20).map(line_preview).collect()),
                ),
                ("import", import),
                ("document", document),
            ])
        };
        if let Some(found) = existing.first() {
            let document = crate::documents::get_document(
                conn,
                scope.profile_id,
                &text(found, "document_id").unwrap_or_default(),
            )?;
            return Ok(summary(import_payload(found), document));
        }
        if request.dry_run {
            return Ok(summary(Value::Null, Value::Null));
        }
        let import_id = uuid(conn)?;
        let at = now(conn)?;
        // Generated text carries no bank text: the template, the account's
        // short form and the period. The original file name is never kept.
        let safe_label = statement_label(
            statement.account.as_deref().map(account_short).as_deref(),
            period_start.as_deref(),
            period_end.as_deref(),
        );
        let label = request
            .label
            .clone()
            .filter(|l| !l.trim().is_empty())
            .unwrap_or_else(|| safe_label.clone());
        let document = crate::documents::create(
            conn,
            scope,
            &dict([
                ("document_type", s("statement")),
                ("label", s(label.as_str())),
                ("issuer", Value::Null),
                ("issued_at", opt(period_end.as_deref())),
                ("fiat_currency", opt(statement.currency.as_deref())),
                ("notes", s("Imported bank statement. Evidence only: its lines support source-of-funds review and never become transactions, balances or journal entries.")),
            ]),
            &dict([("bank_statement_import", s(import_id.as_str())), ("source_format", s(format))]),
        )?;
        let document_id = text(&document, "id").unwrap_or_default();
        let name = if format == "camt053" {
            "bank-statement.xml"
        } else {
            "bank-statement.csv"
        };
        let attachment = crate::documents::attach(
            conn,
            scope,
            attachments_root,
            &dict([
                ("document", s(document_id.as_str())),
                ("file_path", s(path.to_string_lossy().as_ref())),
                ("stored_name", s(name)),
                ("label", s(safe_label.as_str())),
            ]),
            &mut copied,
        )?;
        let attachment_id = text(&attachment, "attachment_id").unwrap_or_default();
        let stored_sha: Option<String> = conn
            .query_row(
                "SELECT sha256 FROM attachments WHERE id=?",
                [&attachment_id],
                |r| r.get(0),
            )
            .map_err(crate::sql::error)?;
        if stored_sha.as_deref() != Some(file_sha256.as_str()) {
            return Err(AppError::new(
                "conflict",
                "The statement file changed while it was being imported",
            )
            .with_hint("Import it again."));
        }
        conn.execute(
            "INSERT INTO bank_statement_imports(id,workspace_id,profile_id,document_id,attachment_id,source_format,file_sha256,file_key,account_keys,account_id,account_masked,bank_name,bank_bic,statement_ref,column_map_json,period_start,period_end,lines_total,lines_new,lines_existing,lines_skipped,imported_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
            params![
                import_id, scope.workspace_id, scope.profile_id, document_id, attachment_id, format, file_sha256, file_key,
                account_keys.join(" "), statement.account, account_masked, statement.bank_name, statement.bank_bic, statement.statement_id,
                column_map, period_start, period_end, keys.len() as i64, new_count as i64, (keys.len() - new_count) as i64,
                statement.skipped.len() as i64, at,
            ],
        )
        .map_err(crate::sql::error)?;
        let mut insert = conn
            .prepare("INSERT INTO bank_statement_lines(id,workspace_id,profile_id,import_id,document_id,line_index,booking_date,value_date,amount,currency,counterparty_name,counterparty_iban_masked,remittance,bank_reference,end_to_end_id,account_id,account_masked,dedupe_group,dedupe_key,created_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)")
            .map_err(crate::sql::error)?;
        for ((line, key), group) in lines.iter().zip(&keys).zip(&groups) {
            let Some(key) = key else { continue };
            insert
                .execute(params![
                    uuid(conn)?,
                    scope.workspace_id,
                    scope.profile_id,
                    import_id,
                    document_id,
                    line.position as i64,
                    line.booking_date,
                    line.value_date,
                    line.amount.canonical(),
                    line.currency,
                    line.counterparty_name,
                    line.counterparty_iban.as_deref().map(mask_account),
                    line.remittance,
                    line.bank_reference,
                    line.end_to_end_id,
                    line.account,
                    line.account.as_deref().map(mask_account),
                    group,
                    key,
                    at,
                ])
                .map_err(crate::sql::error)?;
        }
        let import = rows(
            conn,
            "SELECT * FROM bank_statement_imports WHERE id=?",
            [&import_id],
        )?;
        Ok(summary(import_payload(&import[0]), document))
    });
    if result.is_err() {
        for file in copied {
            let _ = fs::remove_file(file);
        }
    }
    result
}

/// Recorded imports, newest first.
pub fn list_imports(conn: &Connection, scope: &Scope<'_>, limit: i64) -> Result<Value> {
    if !tables_present(conn)? {
        return Ok(Value::List(Vec::new()));
    }
    let limit = if limit <= 0 { 100 } else { limit.min(1000) };
    let found = rows(conn, "SELECT * FROM bank_statement_imports WHERE profile_id=? ORDER BY imported_at DESC, id DESC LIMIT ?", params![scope.profile_id, limit])?;
    Ok(Value::List(found.iter().map(import_payload).collect()))
}

/// Plan, or with `confirm` perform, the rollback of one import.
pub fn rollback(
    conn: &Connection,
    scope: &Scope<'_>,
    attachments_root: &Path,
    import_id: &str,
    confirm: bool,
) -> Result<Value> {
    let not_found = || {
        AppError::new(
            "not_found",
            format!("Statement import '{import_id}' not found"),
        )
    };
    let mut remove = Vec::new();
    let result = atomic(conn, || {
        if !tables_present(conn)? {
            return Err(not_found());
        }
        let import = rows(
            conn,
            "SELECT * FROM bank_statement_imports WHERE profile_id=? AND id=?",
            params![scope.profile_id, import_id],
        )?
        .into_iter()
        .next()
        .ok_or_else(not_found)?;
        let count = |sql: &str| -> Result<i64> {
            conn.query_row(sql, [import_id], |r| r.get(0))
                .map_err(crate::sql::error)
        };
        let lines = count("SELECT COUNT(*) FROM bank_statement_lines WHERE import_id=?")?;
        let accepted = count("SELECT COUNT(DISTINCT d.line_id) FROM bank_statement_line_decisions d JOIN bank_statement_lines l ON l.id=d.line_id WHERE l.import_id=? AND d.state='accepted'")?;
        let plan = dict([
            ("import", import_payload(&import)),
            ("lines_to_delete", Value::Int(lines.into())),
            ("accepted_lines", Value::Int(accepted.into())),
            ("confirmed", Value::Bool(confirm)),
        ]);
        if !confirm {
            return Ok(plan);
        }
        if accepted > 0 {
            return Err(AppError::new(
                "conflict",
                format!(
                    "{accepted} line(s) of this statement are accepted source-of-funds evidence"
                ),
            )
            .with_hint("Reject those lines first (source-funds bank-lines reject), then roll back.")
            .with_details(dict([("accepted_lines", Value::Int(accepted.into()))])));
        }
        let document_id = text(&import, "document_id").unwrap_or_default();
        for row in rows(conn, "SELECT a.id,a.stored_relpath FROM external_document_attachments j JOIN attachments a ON a.id=j.attachment_id WHERE j.document_id=?", [&document_id])? {
            if let (Some(path), true) = crate::attachments::stored_path(attachments_root, &text(&row, "stored_relpath").unwrap_or_default()) {
                remove.push(path);
            }
            conn.execute("DELETE FROM attachments WHERE id=?", [text(&row, "id")]).map_err(crate::sql::error)?;
        }
        for sql in [
            "DELETE FROM bank_statement_lines WHERE import_id=?",
            "DELETE FROM bank_statement_imports WHERE id=?",
        ] {
            conn.execute(sql, [import_id]).map_err(crate::sql::error)?;
        }
        conn.execute(
            "DELETE FROM external_documents WHERE id=? AND profile_id=?",
            params![document_id, scope.profile_id],
        )
        .map_err(crate::sql::error)?;
        Ok(plan)
    });
    if result.is_ok() {
        for file in remove {
            let _ = fs::remove_file(file);
        }
    }
    result
}

/// One stored line: amounts, dates and ids exact; the IBANs stored masked.
#[derive(Clone, Debug)]
pub struct StoredLine {
    pub id: String,
    pub booking_date: String,
    pub value_date: Option<String>,
    /// Exact signed decimal text.
    pub amount: String,
    pub currency: String,
    pub counterparty_name: Option<String>,
    pub counterparty_iban_masked: Option<String>,
    pub remittance: Option<String>,
    pub bank_reference: Option<String>,
    pub account_masked: Option<String>,
    pub bank_name: Option<String>,
}

pub const LINE_COLUMNS: &str = "l.*, i.bank_name";

impl StoredLine {
    /// From a row of [`LINE_COLUMNS`].
    pub fn from_row(row: &Value) -> Self {
        StoredLine {
            id: text(row, "id").unwrap_or_default(),
            booking_date: text(row, "booking_date").unwrap_or_default(),
            value_date: text(row, "value_date"),
            amount: text(row, "amount").unwrap_or_default(),
            currency: text(row, "currency").unwrap_or_default(),
            counterparty_name: text(row, "counterparty_name"),
            counterparty_iban_masked: text(row, "counterparty_iban_masked"),
            remittance: text(row, "remittance"),
            bank_reference: text(row, "bank_reference"),
            account_masked: text(row, "account_masked"),
            bank_name: text(row, "bank_name"),
        }
    }

    /// The line as a read returns it: free text with IBANs masked.
    pub fn payload(&self, row: &Value) -> Value {
        let outgoing = Money::parse(&self.amount).is_some_and(|m| m.is_negative());
        let shown = |v: &Option<String>| opt(v.as_deref().map(mask_ibans).as_deref());
        dict([
            ("id", s(self.id.as_str())),
            (
                "document_id",
                get(row, "document_id").cloned().unwrap_or(Value::Null),
            ),
            (
                "import_id",
                get(row, "import_id").cloned().unwrap_or(Value::Null),
            ),
            (
                "line_index",
                get(row, "line_index").cloned().unwrap_or(Value::Null),
            ),
            ("booking_date", s(self.booking_date.as_str())),
            ("value_date", opt(self.value_date.as_deref())),
            ("amount", s(self.amount.as_str())),
            ("currency", s(self.currency.as_str())),
            ("direction", s(if outgoing { "out" } else { "in" })),
            ("counterparty_name", shown(&self.counterparty_name)),
            (
                "counterparty_iban_masked",
                opt(self.counterparty_iban_masked.as_deref()),
            ),
            ("remittance", shown(&self.remittance)),
            ("bank_reference", shown(&self.bank_reference)),
            ("account_masked", opt(self.account_masked.as_deref())),
            ("bank_name", shown(&self.bank_name)),
        ])
    }
}

/// The accepted lines supporting one source-of-funds source.
pub fn accepted_lines(conn: &Connection, source_id: &str) -> Result<Vec<StoredLine>> {
    if !tables_present(conn)? {
        return Ok(Vec::new());
    }
    let found = rows(
        conn,
        "SELECT DISTINCT l.*, i.bank_name FROM bank_statement_line_decisions d JOIN bank_statement_lines l ON l.id=d.line_id JOIN bank_statement_imports i ON i.id=l.import_id JOIN transactions t ON t.id=d.transaction_id AND t.profile_id=d.profile_id WHERE d.source_id=? AND d.state='accepted' ORDER BY l.booking_date, l.id",
        [source_id],
    )?;
    Ok(found.iter().map(StoredLine::from_row).collect())
}

/// Stored lines with their review decisions.
pub fn list_lines(conn: &Connection, scope: &Scope<'_>, filter: &LineFilter) -> Result<Value> {
    if !tables_present(conn)? {
        return Ok(Value::List(Vec::new()));
    }
    let direction = match filter.direction.as_str() {
        "" | "all" => "",
        "out" => " AND l.amount LIKE '-%'",
        "in" => " AND l.amount NOT LIKE '-%'",
        other => {
            return Err(invalid(format!("Unsupported direction '{other}'"))
                .with_hint("Choose one of: out, in, all."))
        }
    };
    let limit = if filter.limit <= 0 {
        200
    } else {
        filter.limit.min(5000)
    };
    let found = rows(
        conn,
        &format!("SELECT {LINE_COLUMNS} FROM bank_statement_lines l JOIN bank_statement_imports i ON i.id=l.import_id WHERE l.profile_id=?1 AND (?2='' OR l.document_id=?2){direction} ORDER BY l.booking_date DESC, l.line_index, l.id LIMIT ?3"),
        params![scope.profile_id, filter.document.clone().unwrap_or_default(), limit],
    )?;
    let mut decisions: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for row in rows(conn, "SELECT line_id,transaction_id,state,source_id,group_key,updated_at FROM bank_statement_line_decisions WHERE profile_id=? ORDER BY updated_at, transaction_id", [scope.profile_id])? {
        let item = dict(["transaction_id", "state", "source_id", "group_key", "updated_at"].map(|k| (k, get(&row, k).cloned().unwrap_or(Value::Null))));
        decisions.entry(text(&row, "line_id").unwrap_or_default()).or_default().push(item);
    }
    Ok(Value::List(
        found
            .iter()
            .map(|row| {
                let line = StoredLine::from_row(row);
                let mut payload = line.payload(row);
                kassiber_envelope::set(
                    &mut payload,
                    "decisions",
                    Value::List(decisions.remove(&line.id).unwrap_or_default()),
                );
                payload
            })
            .collect(),
    ))
}

/// "Bank statement <••last four> <period>": Kassiber's template with the
/// account's short form, never bank text or a file name.
pub fn statement_label(
    account_short: Option<&str>,
    start: Option<&str>,
    end: Option<&str>,
) -> String {
    let mut label = String::from("Bank statement");
    if let Some(short) = account_short {
        label.push(' ');
        label.push_str(short);
    }
    match (start, end) {
        (Some(a), Some(b)) if a != b => label.push_str(&format!(" {a} to {b}")),
        (Some(a), _) => label.push_str(&format!(" {a}")),
        _ => {}
    }
    label
}
