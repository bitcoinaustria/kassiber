//! Source coverage (Rust only; docs/reference/desktop.md#sources-live-sync-and-file-snapshots):
//! how far each wallet's history reaches, why, and the one action that brings
//! it current for a period end.
//!
//! A live source (descriptor, xpub, address list, Lightning node, BTCPay,
//! exchange API) is as current as its last successful sync. A file source is
//! a snapshot: it covers the time span of the rows its imported files held,
//! never the import time. A hybrid exchange keeps recent activity current
//! through its API and older history through a CSV. A manual ledger has no
//! notion of currency.
//!
//! Everything here reads the book only, plus the local metadata (never the
//! bytes) of the file a wallet remembers importing. It never opens a socket.
//! A coverage attestation is the user's statement that nothing happened after
//! a date. It closes a coverage gap only where it starts at or before the
//! source's covered end, and never creates transactions, balances, or tax
//! facts.
use crate::completeness;
use crate::{database_error, resolve};
use kassiber_envelope::{dict, s, AppError};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::{params, Connection};
use serde_json::{json, Value as Json};
use std::collections::BTreeMap;

/// The wallet config key of the user's coverage attestation.
pub const ATTESTATION_KEY: &str = "coverage_attestation";
/// The wallet config key of the last exchange API sync outcome.
pub const EXCHANGE_SYNC_KEY: &str = "exchange_sync";
/// Attestation kinds: nothing happened after the date, or the account was
/// closed on it (nothing happens after it, for any period).
pub const ATTESTATION_KINDS: [&str; 2] = ["no_activity_after", "closed_on"];
const UNKNOWN_OCCURRED_AT: &str = "1970-01-01T00:00:00Z";

/// What a coverage read covers.
#[derive(Default)]
pub struct Options<'a> {
    /// The period end the state is relative to (`YYYY-MM-DD` or an ISO
    /// timestamp); today when absent.
    pub period_end: Option<&'a str>,
    /// One wallet (id or label) instead of the whole profile.
    pub wallet: Option<&'a str>,
    /// Skip `scan_bounds` (its per-row balance walk) for hot reads such as the
    /// overview; `scan_warnings` is then empty and nothing else changes.
    pub skip_scan_bounds: bool,
    /// The user asked to compare the remembered file's contents with its
    /// last import: hash it (bounded, regular files only) for the one
    /// wallet named. Display reads never set it.
    pub check_file: bool,
}

/// One source's coverage.
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    pub wallet_id: String,
    pub label: String,
    pub kind: String,
    pub completeness_class: String,
    /// `live` | `file` | `hybrid` | `manual`.
    pub source_mode: &'static str,
    pub covered_from: Option<String>,
    pub covered_through: Option<String>,
    /// `current` | `behind` | `never_imported` | `sync_failed` | `unknown`.
    pub freshness_state: &'static str,
    /// Why: a stable code the desktop and assistant translate.
    pub reason: &'static str,
    /// The one action that fixes it: `sync`, `import_newer_file`,
    /// `sync_or_import`, `review_sync_error`, or `none`.
    pub action: &'static str,
    pub last_sync_at: Option<String>,
    pub last_sync_status: Option<String>,
    pub last_sync_error_code: Option<String>,
    pub last_import_at: Option<String>,
    pub import_count: i64,
    pub file_first_row_at: Option<String>,
    pub file_last_row_at: Option<String>,
    pub first_transaction_at: Option<String>,
    pub last_transaction_at: Option<String>,
    pub transaction_count: i64,
    pub source_format: Option<String>,
    /// The newest import run's rows read (NULL before it was recorded) and
    /// rows it added.
    pub last_run_rows_read: Option<i64>,
    pub last_run_rows_inserted: Option<i64>,
    /// The remembered file changed since its last import (`None` when there
    /// is no remembered file or it cannot be read).
    pub source_file_changed: Option<bool>,
    /// The result of an explicit content check: `same`, `different`, or
    /// `unknown` (no stored hash, or the file cannot be read as a regular
    /// file within the limit). `None` when no check was asked for.
    pub source_file_check: Option<&'static str>,
    /// Whether the wallet remembers the file it imports (never the path):
    /// the explicit check is offered for every such source.
    pub source_file_remembered: bool,
    /// For hybrids: how far back the exchange API returns history, when the
    /// exchange module declares a limit.
    pub api_history_limit: Option<String>,
    pub attestation: Option<Attestation>,
    /// The part of the gap a statement leaves open: (from, through], from the
    /// covered end to the statement's date. `None` when it closes the gap.
    pub unattested_from: Option<String>,
    pub unattested_through: Option<String>,
    pub scan_warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Attestation {
    pub kind: String,
    pub date: String,
    pub note: String,
    pub attested_at: String,
}

impl Attestation {
    /// Whether this statement closes the gap after `through_day`, the last
    /// covered day. Both kinds say nothing happened after their date, so they
    /// cover (through_day, period end] only when the date is not after
    /// through_day; a later date leaves (through_day, date] unattested.
    fn closes_gap_after(&self, through_day: &str) -> bool {
        self.date.as_str() <= through_day
    }

    fn to_value(&self) -> Value {
        dict([
            ("kind", s(self.kind.as_str())),
            ("date", s(self.date.as_str())),
            ("note", s(self.note.as_str())),
            ("attested_at", s(self.attested_at.as_str())),
        ])
    }

    fn parse(config: &Json) -> Option<Self> {
        let raw = config.get(ATTESTATION_KEY)?;
        let text = |key: &str| raw.get(key).and_then(Json::as_str).map(str::to_owned);
        let kind = text("kind").filter(|k| ATTESTATION_KINDS.contains(&k.as_str()))?;
        let date = text("date").filter(|d| valid_day(d))?;
        Some(Self {
            kind,
            date,
            note: text("note").unwrap_or_default(),
            attested_at: text("attested_at").unwrap_or_default(),
        })
    }
}

fn opt(value: &Option<String>) -> Value {
    value.as_deref().map_or(Value::Null, s)
}

impl Source {
    pub fn needs_action(&self) -> bool {
        matches!(
            self.freshness_state,
            "behind" | "sync_failed" | "never_imported"
        )
    }

    pub fn to_value(&self) -> Value {
        dict([
            ("wallet_id", s(self.wallet_id.as_str())),
            ("wallet", s(self.label.as_str())),
            ("kind", s(self.kind.as_str())),
            ("completeness_class", s(self.completeness_class.as_str())),
            ("source_mode", s(self.source_mode)),
            ("covered_from", opt(&self.covered_from)),
            ("covered_through", opt(&self.covered_through)),
            ("freshness_state", s(self.freshness_state)),
            ("reason", s(self.reason)),
            ("action", s(self.action)),
            ("needs_action", Value::Bool(self.needs_action())),
            (
                "live",
                dict([
                    ("last_sync_at", opt(&self.last_sync_at)),
                    ("last_sync_status", opt(&self.last_sync_status)),
                    ("last_sync_error_code", opt(&self.last_sync_error_code)),
                    ("api_history_limit", opt(&self.api_history_limit)),
                ]),
            ),
            ("file", {
                let mut file = dict([
                    ("source_format", opt(&self.source_format)),
                    ("import_count", Value::from(self.import_count)),
                    ("last_import_at", opt(&self.last_import_at)),
                    ("first_row_at", opt(&self.file_first_row_at)),
                    ("last_row_at", opt(&self.file_last_row_at)),
                    (
                        "last_run_rows_read",
                        self.last_run_rows_read.map_or(Value::Null, Value::from),
                    ),
                    (
                        "last_run_rows_inserted",
                        self.last_run_rows_inserted.map_or(Value::Null, Value::from),
                    ),
                    (
                        "source_file_changed",
                        self.source_file_changed.map_or(Value::Null, Value::Bool),
                    ),
                ]);
                // Only an explicit check (`check_file`) answers it.
                if let (Some(check), Value::Dict(fields)) = (self.source_file_check, &mut file) {
                    fields.push(("source_file_check".into(), s(check)));
                }
                // Present only when true, so other sources keep their bytes.
                if let (true, Value::Dict(fields)) = (self.source_file_remembered, &mut file) {
                    fields.push(("source_file_remembered".into(), Value::Bool(true)));
                }
                file
            }),
            (
                "transactions",
                dict([
                    ("count", Value::from(self.transaction_count)),
                    ("first_at", opt(&self.first_transaction_at)),
                    ("last_at", opt(&self.last_transaction_at)),
                ]),
            ),
            (
                "attestation",
                self.attestation
                    .as_ref()
                    .map_or(Value::Null, Attestation::to_value),
            ),
            (
                "attestation_gap",
                match (&self.unattested_from, &self.unattested_through) {
                    (Some(from), Some(through)) => {
                        dict([("from", s(from.as_str())), ("through", s(through.as_str()))])
                    }
                    _ => Value::Null,
                },
            ),
            (
                "scan_warnings",
                Value::List(self.scan_warnings.iter().map(|w| s(w.as_str())).collect()),
            ),
        ])
    }
}

/// `YYYY-MM-DD` of an ISO date or timestamp, when it has one.
pub fn day_of(value: &str) -> Option<String> {
    let day = value.trim().get(..10)?;
    valid_day(day).then(|| day.to_owned())
}

fn valid_day(day: &str) -> bool {
    let bytes = day.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    let number = |range: std::ops::Range<usize>| day.get(range)?.parse::<u32>().ok();
    let (Some(year), Some(month), Some(date)) = (number(0..4), number(5..7), number(8..10)) else {
        return false;
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let length = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    year >= 1 && (1..=length).contains(&date)
}

fn later(a: Option<String>, b: Option<String>) -> Option<String> {
    match (a, b) {
        (Some(a), Some(b)) => Some(if a >= b { a } else { b }),
        (a, b) => a.or(b),
    }
}

fn earlier(a: Option<String>, b: Option<String>) -> Option<String> {
    match (a, b) {
        (Some(a), Some(b)) => Some(if a <= b { a } else { b }),
        (a, b) => a.or(b),
    }
}

fn text(config: &Json, key: &str) -> Option<String> {
    config
        .get(key)
        .and_then(Json::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
}

/// The exchange kinds whose API a wallet can sync, plus file-only exchanges.
fn exchange_api_kind(kind: &str) -> bool {
    crate::file_import::exchange_api::KINDS.contains(&kind)
}

/// How far back an exchange's API returns history. `None` means the module
/// declares no limit; exchanges fill this in as they learn one.
pub fn api_history_limit(_kind: &str) -> Option<&'static str> {
    None
}

#[derive(Default)]
struct Imports {
    count: i64,
    last_import_at: Option<String>,
    /// The span of the rows the runs' files held, from runs that recorded it.
    first_row_at: Option<String>,
    last_row_at: Option<String>,
    /// For runs from before that was recorded: the span of the rows they
    /// created. Never other rows of the wallet.
    legacy_first: Option<String>,
    legacy_last: Option<String>,
    /// The newest run: rows its file held and rows it added.
    last_rows_read: Option<i64>,
    last_rows_inserted: Option<i64>,
    /// The newest recorded file size and when that run was imported.
    last_file_size: Option<i64>,
    /// The newest recorded file hash.
    last_sha256: Option<String>,
}

#[derive(Default)]
struct Rows {
    /// Active (not excluded) rows and their dated span.
    count: i64,
    first: Option<String>,
    last: Option<String>,
    /// Every stored row, excluded ones too.
    total: i64,
}

#[derive(Default)]
struct Failure {
    failed: bool,
    code: Option<String>,
    at: Option<String>,
}

fn imports(conn: &Connection, profile: &str) -> Result<BTreeMap<String, Imports>, AppError> {
    let mut by_wallet: BTreeMap<String, Imports> = BTreeMap::new();
    // What a run's range may rest on: rows an import run of the same wallet
    // and importer (`source_format`, its identity scheme) created and the
    // book still holds, excluded ones too. A run that only matched rows has
    // no members of its own and rests on the rows of the run that created
    // them; rows entered by hand, synced from an API or created by another
    // importer never support it.
    let mut support: BTreeMap<(String, String), (Option<String>, Option<String>)> = BTreeMap::new();
    let mut members = conn
        .prepare(
            "SELECT b.wallet_id, COALESCE(b.source_format, ''), \
             MIN(NULLIF(t.occurred_at, ?2)), MAX(NULLIF(t.occurred_at, ?2)) \
             FROM import_batches b \
             JOIN import_batch_transactions bt ON bt.batch_id = b.id \
             JOIN transactions t ON t.id = bt.transaction_id AND t.wallet_id = b.wallet_id \
             WHERE b.profile_id = ?1 AND b.wallet_id IS NOT NULL \
             GROUP BY b.wallet_id, COALESCE(b.source_format, '')",
        )
        .map_err(database_error)?;
    let spans = members
        .query_map(params![profile, UNKNOWN_OCCURRED_AT], |row| {
            Ok((
                (row.get::<_, String>(0)?, row.get::<_, String>(1)?),
                (
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ),
            ))
        })
        .map_err(database_error)?;
    for span in spans {
        let (key, bounds) = span.map_err(database_error)?;
        support.insert(key, bounds);
    }
    let mut statement = conn
        .prepare(
            "SELECT wallet_id, imported_at, first_row_at, last_row_at, rows_read, rows_inserted, file_size, file_sha256, \
             COALESCE(source_format, '') \
             FROM import_batches WHERE profile_id = ? AND wallet_id IS NOT NULL \
             ORDER BY import_sequence ASC, imported_at ASC, id ASC",
        )
        .map_err(database_error)?;
    let found = statement
        .query_map([profile], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, Option<i64>>(5)?,
                row.get::<_, Option<i64>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, String>(8)?,
            ))
        })
        .map_err(database_error)?;
    for item in found {
        let (wallet, at, first, last, read, inserted, size, sha, format) =
            item.map_err(database_error)?;
        // A range counts only as far as the rows it rests on still reach: a
        // rollback or a book reset that removed them takes it with them.
        let (first, last) = match support.get(&(wallet.clone(), format)) {
            Some((support_first, support_last)) => {
                supported(first, last, support_first.clone(), support_last.clone())
            }
            None => (None, None),
        };
        let entry = by_wallet.entry(wallet).or_default();
        entry.count += 1;
        entry.last_import_at = later(entry.last_import_at.take(), at);
        entry.first_row_at = earlier(entry.first_row_at.take(), first);
        entry.last_row_at = later(entry.last_row_at.take(), last);
        entry.last_rows_read = read;
        entry.last_rows_inserted = inserted;
        if size.is_some() {
            entry.last_file_size = size;
        }
        // The newest run's own hash: an older one says nothing about the
        // file last imported (a newest run over the limit has none).
        entry.last_sha256 = sha;
    }
    // Legacy runs (no recorded file range): the rows each created.
    let mut legacy = conn
        .prepare(
            "SELECT b.wallet_id, MIN(NULLIF(t.occurred_at, ?2)), MAX(NULLIF(t.occurred_at, ?2)) \
             FROM import_batches b \
             JOIN import_batch_transactions bt ON bt.batch_id = b.id \
             JOIN transactions t ON t.id = bt.transaction_id \
             WHERE b.profile_id = ?1 AND b.wallet_id IS NOT NULL AND b.rows_read IS NULL AND t.excluded = 0 \
             GROUP BY b.wallet_id",
        )
        .map_err(database_error)?;
    let spans = legacy
        .query_map(params![profile, UNKNOWN_OCCURRED_AT], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(database_error)?;
    for span in spans {
        let (wallet, first, last) = span.map_err(database_error)?;
        if let Some(entry) = by_wallet.get_mut(&wallet) {
            entry.legacy_first = first;
            entry.legacy_last = last;
        }
    }
    Ok(by_wallet)
}

fn transaction_rows(conn: &Connection, profile: &str) -> Result<BTreeMap<String, Rows>, AppError> {
    let mut statement = conn
        .prepare(
            "SELECT wallet_id, SUM(excluded = 0), \
             MIN(CASE WHEN excluded = 0 THEN NULLIF(occurred_at, ?2) END), \
             MAX(CASE WHEN excluded = 0 THEN NULLIF(occurred_at, ?2) END), \
             COUNT(*) \
             FROM transactions WHERE profile_id = ?1 GROUP BY wallet_id",
        )
        .map_err(database_error)?;
    let found = statement
        .query_map(params![profile, UNKNOWN_OCCURRED_AT], |row| {
            Ok((
                row.get::<_, String>(0)?,
                Rows {
                    count: row.get::<_, Option<i64>>(1)?.unwrap_or(0),
                    first: row.get(2)?,
                    last: row.get(3)?,
                    total: row.get(4)?,
                },
            ))
        })
        .map_err(database_error)?;
    found
        .collect::<Result<BTreeMap<_, _>, _>>()
        .map_err(database_error)
}

/// The newest freshness failure per wallet that no later success cleared.
fn failures(conn: &Connection, profile: &str) -> Result<BTreeMap<String, Failure>, AppError> {
    let mut statement = conn
        .prepare(
            "SELECT source_key, status, last_success_at, last_error_at, last_error_code FROM freshness_source_states \
             WHERE profile_id = ? AND (source_key LIKE 'onchain_wallet:%' OR source_key LIKE 'btcpay_wallet:%' \
             OR source_key LIKE 'btcpay_provenance:%')",
        )
        .map_err(database_error)?;
    let found = statement
        .query_map([profile], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })
        .map_err(database_error)?;
    let mut by_wallet: BTreeMap<String, Failure> = BTreeMap::new();
    for item in found {
        let (key, status, success, error_at, code) = item.map_err(database_error)?;
        let Some((_, wallet)) = key.split_once(':') else {
            continue;
        };
        // `btcpay_provenance:account:<id>` routes belong to no wallet.
        if wallet.starts_with("account:") {
            continue;
        }
        let errored = matches!(status.as_deref(), Some("failed" | "error"))
            || error_at
                .as_ref()
                .is_some_and(|at| success.as_ref().is_none_or(|ok| at > ok));
        if errored {
            let entry = by_wallet.entry(wallet.to_owned()).or_default();
            entry.failed = true;
            if entry.code.is_none() {
                entry.code = code;
            }
            entry.at = later(entry.at.take(), error_at);
        }
    }
    Ok(by_wallet)
}

/// Whether the remembered file changed since its last import, from local
/// metadata only: it must be a regular file (a symlink, FIFO or device is not
/// followed and yields `None`), and a different size than the import
/// recorded, or a modification after that import, means a newer export is
/// waiting. The file is never opened or read here.
fn file_changed(config: &Json, imports: Option<&Imports>) -> Option<bool> {
    let path = text(config, "source_file")?;
    let imports = imports?;
    let last = imports.last_import_at.as_deref()?;
    let meta = std::fs::symlink_metadata(&path).ok()?;
    if !meta.is_file() {
        return None;
    }
    if let (Some(recorded), Ok(now)) = (imports.last_file_size, i64::try_from(meta.len())) {
        if recorded != now {
            return Some(true);
        }
    }
    let seconds = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    let modified = crate::time::from_unix(&kassiber_pyjson::BigInt::from(seconds))?;
    Some(modified.as_str() > last)
}

/// An explicit content check of the remembered file against its newest
/// recorded hash: the same bounded, regular-file-only snapshot an import
/// takes, run only when the user asks.
fn check_file(config: &Json, imports: Option<&Imports>) -> &'static str {
    let (Some(path), Some(known)) = (
        text(config, "source_file"),
        imports.and_then(|i| i.last_sha256.as_deref()),
    ) else {
        return "unknown";
    };
    match crate::file_import::file_snapshot(&path).sha256 {
        Some(now) if now == known => "same",
        Some(_) => "different",
        None => "unknown",
    }
}

/// The coverage mode from what the source is configured to read: an
/// exchange key makes it live (hybrid with file imports), a remembered or
/// imported file makes it a file snapshot whatever its completeness class,
/// the live classes stay live, and only a source with neither is manual.
fn mode(klass: &str, kind: &str, config: &Json, imported: bool) -> &'static str {
    let api = exchange_api_kind(kind) && text(config, "backend").is_some();
    let file = imported || text(config, "source_file").is_some();
    if api {
        return if file { "hybrid" } else { "live" };
    }
    match klass {
        "descriptor-derived" | "address-list" | "lightning-node" | "provider-api" => "live",
        "manual" | "untracked" if !file => "manual",
        _ => "file",
    }
}

/// Every source of the profile (or the one `options.wallet` names), in label
/// order, relative to `options.period_end` or `now`.
pub fn sources(
    conn: &Connection,
    profile: &str,
    options: &Options<'_>,
    now: &str,
) -> Result<Vec<Source>, AppError> {
    let only = options
        .wallet
        .map(|reference| resolve(conn, profile, reference))
        .transpose()?;
    let period_day = match options.period_end {
        Some(end) => day_of(end)
            .ok_or_else(|| AppError::new("validation", "period_end must be a date (YYYY-MM-DD)"))?,
        None => day_of(now).unwrap_or_default(),
    };
    let mut statement = conn
        .prepare(
            "SELECT id, label, kind, config_json FROM wallets WHERE profile_id = ? \
             ORDER BY lower(label) ASC, id ASC",
        )
        .map_err(database_error)?;
    let wallets = statement
        .query_map([profile], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                row.get::<_, Option<String>>(3)?,
            ))
        })
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)?;
    let wallets: Vec<_> = wallets
        .into_iter()
        .filter(|(id, ..)| only.as_ref().is_none_or(|w| &w.id == id))
        .collect();
    let described = if options.skip_scan_bounds {
        let mut described = BTreeMap::new();
        for (id, _, kind, config) in &wallets {
            let (klass, declared) = completeness::classify(
                &s(kind.as_str()),
                &config.as_deref().map_or(Value::Null, s),
            )?;
            described.insert(
                id.clone(),
                (s(klass), dict([("declared_scan_start_at", declared)])),
            );
        }
        described
    } else {
        completeness::describe(
            conn,
            profile,
            &wallets
                .iter()
                .map(|(id, _, kind, config)| completeness::Wallet {
                    id: id.clone(),
                    kind: s(kind.as_str()),
                    config_json: config.as_deref().map_or(Value::Null, s),
                })
                .collect::<Vec<_>>(),
        )?
    };
    let imports = imports(conn, profile)?;
    let rows = transaction_rows(conn, profile)?;
    let failures = failures(conn, profile)?;
    let mut output = Vec::with_capacity(wallets.len());
    for (id, label, kind, raw) in wallets {
        let config: Json = raw
            .as_deref()
            .and_then(|raw| serde_json::from_str(raw).ok())
            .filter(Json::is_object)
            .unwrap_or_else(|| json!({}));
        let kind = kind.trim().to_lowercase();
        let (klass, bounds) = described
            .get(&id)
            .map(|(klass, bounds)| (kassiber_envelope::py_str(klass), bounds.clone()))
            .unwrap_or_else(|| ("file-import".into(), Value::Null));
        let scan_warnings = match kassiber_envelope::get(&bounds, "warnings") {
            Some(Value::List(items)) => items.iter().map(kassiber_envelope::py_str).collect(),
            _ => vec![],
        };
        let declared_start = match kassiber_envelope::get(&bounds, "declared_scan_start_at") {
            Some(value @ Value::Str(_)) => Some(kassiber_envelope::py_str(value)),
            _ => None,
        };
        let import = imports.get(&id);
        let row = rows.get(&id);
        let source_mode = mode(&klass, &kind, &config, import.is_some_and(|i| i.count > 0));
        let exchange = config.get(EXCHANGE_SYNC_KEY);
        let exchange_text = |key: &str| {
            exchange
                .and_then(|e| e.get(key))
                .and_then(Json::as_str)
                .map(str::to_owned)
        };
        let last_synced_at = text(&config, "last_synced_at");
        // The exchange outcome is newer than the stored sync time only when
        // the newest attempt failed.
        let (last_sync_status, last_sync_error_code, exchange_failed) =
            match exchange_text("status").as_deref() {
                Some("error") => {
                    let at = exchange_text("at");
                    let newer = at
                        .as_ref()
                        .is_some_and(|at| last_synced_at.as_ref().is_none_or(|ok| at >= ok));
                    (
                        Some("error".to_owned()),
                        Some(stored_error_class(exchange).to_owned()),
                        newer,
                    )
                }
                Some(other) => (Some(other.to_owned()), None, false),
                None => (
                    last_synced_at.as_ref().map(|_| "ok".to_owned()),
                    None,
                    false,
                ),
            };
        let failure = failures.get(&id);
        let failed = exchange_failed || failure.is_some_and(|f| f.failed);
        let error_code = last_sync_error_code.or_else(|| failure.and_then(|f| f.code.clone()));
        let tx_first = row.and_then(|r| r.first.clone());
        let tx_last = row.and_then(|r| r.last.clone());
        // Import runs say what files covered; other rows of the wallet
        // (manual entries, API rows) never extend a file's reach.
        let file_first = earlier(
            import.and_then(|i| i.first_row_at.clone()),
            import.and_then(|i| i.legacy_first.clone()),
        );
        let file_last = later(
            import.and_then(|i| i.last_row_at.clone()),
            import.and_then(|i| i.legacy_last.clone()),
        );
        let (covered_from, covered_through) = match source_mode {
            "live" => (
                earlier(declared_start.clone(), tx_first.clone()),
                last_synced_at.clone(),
            ),
            "hybrid" => (
                earlier(declared_start.clone(), file_first.clone()),
                later(last_synced_at.clone(), file_last.clone()),
            ),
            "file" => (file_first.clone(), file_last.clone()),
            _ => (tx_first.clone(), tx_last.clone()),
        };
        let attestation = Attestation::parse(&config);
        // Runs alone are no data: their rows may have been rolled back.
        let has_data = row.is_some_and(|r| r.total > 0);
        let through_day = covered_through.as_deref().and_then(day_of);
        let gap = through_day
            .as_deref()
            .is_none_or(|through| through < period_day.as_str());
        // A statement closes the gap only from the covered end onward.
        let attested = gap
            && attestation
                .as_ref()
                .zip(through_day.as_deref())
                .is_some_and(|(a, through)| a.closes_gap_after(through));
        let (unattested_from, unattested_through) = match (&attestation, &through_day) {
            // Shown within the requested period only.
            (Some(a), Some(_)) if gap && !attested => (
                covered_through.clone(),
                Some(if a.date.as_str() <= period_day.as_str() {
                    a.date.clone()
                } else {
                    period_day.clone()
                }),
            ),
            _ => (None, None),
        };
        let (freshness_state, reason, action) = if failed {
            ("sync_failed", "last_sync_failed", "review_sync_error")
        } else if source_mode == "manual" {
            if attested {
                ("current", "attested", "none")
            } else {
                ("unknown", "manual_entries", "none")
            }
        } else if source_mode == "live" && last_synced_at.is_none() {
            ("never_imported", "never_synced", "sync")
        } else if matches!(source_mode, "file" | "hybrid") && !has_data && last_synced_at.is_none()
        {
            ("never_imported", "never_imported", file_action(source_mode))
        } else if !gap {
            ("current", "covered", "none")
        } else if attested {
            ("current", "attested", "none")
        } else if through_day.is_none() {
            ("unknown", "no_dated_rows", file_action(source_mode))
        } else {
            (
                "behind",
                if unattested_through.is_some() {
                    "attestation_leaves_gap"
                } else if source_mode == "file" {
                    "file_ends_before_period_end"
                } else {
                    "synced_before_period_end"
                },
                match source_mode {
                    "live" => "sync",
                    other => file_action(other),
                },
            )
        };
        output.push(Source {
            wallet_id: id.clone(),
            label,
            completeness_class: klass,
            source_mode,
            covered_from,
            covered_through,
            freshness_state,
            reason,
            action,
            last_sync_at: last_synced_at,
            last_sync_status,
            last_sync_error_code: error_code,
            last_import_at: import.and_then(|i| i.last_import_at.clone()),
            import_count: import.map_or(0, |i| i.count),
            file_first_row_at: file_first,
            file_last_row_at: file_last,
            first_transaction_at: tx_first,
            last_transaction_at: tx_last,
            transaction_count: row.map_or(0, |r| r.count),
            source_format: text(&config, "source_format"),
            last_run_rows_read: import.and_then(|i| i.last_rows_read),
            last_run_rows_inserted: import.and_then(|i| i.last_rows_inserted),
            source_file_changed: file_changed(&config, import),
            source_file_check: (options.check_file && only.is_some())
                .then(|| check_file(&config, import)),
            source_file_remembered: text(&config, "source_file").is_some(),
            api_history_limit: (source_mode == "hybrid")
                .then(|| api_history_limit(&kind).map(str::to_owned))
                .flatten(),
            attestation,
            unattested_from,
            unattested_through,
            scan_warnings,
            kind,
        });
    }
    Ok(output)
}

/// The part of a file range that existing observations still support:
/// clamped to their span, and nothing when none remain or none overlap.
fn supported(
    first: Option<String>,
    last: Option<String>,
    support_first: Option<String>,
    support_last: Option<String>,
) -> (Option<String>, Option<String>) {
    let (Some(support_first), Some(support_last)) = (support_first, support_last) else {
        return (None, None);
    };
    let first = first.map(|first| {
        if first < support_first {
            support_first.clone()
        } else {
            first
        }
    });
    let last = last.map(|last| {
        if last > support_last {
            support_last.clone()
        } else {
            last
        }
    });
    match (&first, &last) {
        (Some(f), Some(l)) if f > l => (None, None),
        _ => (first, last),
    }
}

fn file_action(mode: &str) -> &'static str {
    if mode == "hybrid" {
        "sync_or_import"
    } else {
        "import_newer_file"
    }
}

/// `ui.sources.coverage` / `wallets coverage`: every source and a summary.
pub fn report(
    conn: &Connection,
    profile: &str,
    options: &Options<'_>,
    now: &str,
) -> Result<Value, AppError> {
    let sources = sources(conn, profile, options, now)?;
    let count = |state: &str| {
        Value::from(
            sources
                .iter()
                .filter(|s| s.freshness_state == state)
                .count() as i64,
        )
    };
    let period_end = match options.period_end {
        Some(end) => day_of(end),
        None => day_of(now),
    };
    Ok(dict([
        ("period_end", period_end.map_or(Value::Null, s)),
        (
            "period_end_default",
            Value::Bool(options.period_end.is_none()),
        ),
        (
            "summary",
            dict([
                ("total", Value::from(sources.len() as i64)),
                ("current", count("current")),
                ("behind", count("behind")),
                ("sync_failed", count("sync_failed")),
                ("never_imported", count("never_imported")),
                ("unknown", count("unknown")),
                (
                    "needs_action",
                    Value::from(sources.iter().filter(|s| s.needs_action()).count() as i64),
                ),
            ]),
        ),
        (
            "sources",
            Value::List(sources.iter().map(Source::to_value).collect()),
        ),
    ]))
}

/// The error classes a stored exchange sync may carry: the only failure
/// detail the book keeps. Provider text (messages, hints, response bodies)
/// is never stored; the class is derived from the envelope's error code.
/// The store's open rewrites records older builds left with text.
pub use kassiber_store::exchange_sync_errors::{exchange_error_class, EXCHANGE_ERROR_CLASSES};

/// The class a stored record carries; older records stored the code itself.
fn stored_error_class(record: Option<&Json>) -> &'static str {
    let error = record.and_then(|r| r.get("error"));
    let class = error
        .and_then(|e| e.get("class"))
        .and_then(Json::as_str)
        .and_then(|class| {
            EXCHANGE_ERROR_CLASSES
                .iter()
                .find(|c| **c == class)
                .copied()
        });
    class.unwrap_or_else(|| {
        error
            .and_then(|e| e.get("code"))
            .and_then(Json::as_str)
            .map_or("other", exchange_error_class)
    })
}

#[cfg(test)]
#[path = "coverage/tests.rs"]
mod tests;
