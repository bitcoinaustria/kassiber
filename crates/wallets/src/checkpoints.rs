//! Balance checkpoints (docs/reference/balance-checkpoints.md): a balance a
//! source reports for one wallet at one time, compared with the balance the
//! book's journal quantity postings give that wallet at that time.
//!
//! Stored checkpoints come only from an explicit action: an exchange API sync
//! (`api`), an imported file's running balance (`file`) or the user's own entry
//! (`manual`). Chain and Lightning node checkpoints are derived on read from
//! what the last sync stored; computing them never opens a connection.
//!
//! Like `kassiber_custody::quantity::balances::compare_wallet_balances`, a
//! checkpoint is a diagnostic. It never changes custody, journals, basis or
//! tax results; a difference is a review item, never a correction.
use crate::database_error;
use kassiber_envelope::{dict, s, AppError};
use kassiber_pyjson::Value;
use kassiber_rates::time;
use kassiber_store::rusqlite::{params, Connection, OptionalExtension};

/// Every origin a checkpoint can have, in display order.
pub const ORIGINS: [&str; 5] = ["api", "file", "chain", "node", "manual"];
/// The origins stored in `balance_checkpoints`; `chain` and `node` are derived.
pub const STORED_ORIGINS: [&str; 3] = ["api", "file", "manual"];
pub const ASSETS: [&str; 2] = ["BTC", "LBTC"];
const MSAT_PER_BTC: i128 = 100_000_000_000;
/// BTC amounts are whole msat: at most 11 fractional digits.
const BTC_DECIMALS: usize = 11;
/// The CSV columns `export` writes and `import` reads.
pub const CSV_HEADER: [&str; 8] = [
    "date",
    "wallet_id",
    "wallet",
    "btc_amount",
    "asset",
    "origin",
    "note",
    "checkpoint_id",
];
/// One import is bounded so a hostile file cannot grow the book without end.
const MAX_IMPORT_ROWS: usize = 100_000;

type Result<T> = std::result::Result<T, AppError>;

/// Exact BTC decimal text (`"0.01"`, `"21"`, `".5"`) to msat. No sign,
/// exponent, separators or more than 11 fractional digits: anything else is
/// `None`, so callers fail closed instead of rounding.
pub fn parse_btc_msat(text: &str) -> Option<i64> {
    let (whole, fraction) = match text.split_once('.') {
        Some((whole, fraction)) => (whole, fraction),
        None => (text, ""),
    };
    if whole.is_empty() && fraction.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
        || fraction.len() > BTC_DECIMALS
        || (text.contains('.') && fraction.is_empty() && whole.is_empty())
    {
        return None;
    }
    let whole: i128 = if whole.is_empty() {
        0
    } else {
        // 21 million BTC fits easily; refuse absurd lengths before parsing.
        if whole.len() > 15 {
            return None;
        }
        whole.parse().ok()?
    };
    let mut padded = fraction.to_owned();
    padded.extend(std::iter::repeat_n('0', BTC_DECIMALS - fraction.len()));
    let fraction: i128 = padded.parse().ok()?;
    i64::try_from(whole * MSAT_PER_BTC + fraction).ok()
}

/// msat as BTC text: at least 8 decimals, up to 11 when msat need them.
pub fn format_btc(msat: i128) -> String {
    let sign = if msat < 0 { "-" } else { "" };
    let value = msat.unsigned_abs();
    let whole = value / MSAT_PER_BTC as u128;
    let fraction = format!("{:011}", value % MSAT_PER_BTC as u128);
    let trimmed = fraction.trim_end_matches('0');
    let shown = if trimmed.len() <= 8 {
        &fraction[..8]
    } else {
        trimmed
    };
    format!("{sign}{whole}.{shown}")
}

/// A user-entered time: a date (`2024-12-31`, the whole UTC day up to the
/// next midnight, stored as `23:59:59.999999Z`) or an ISO date-time with an explicit offset. A date-time
/// without an offset is ambiguous and refused. Returns UTC microseconds.
pub fn parse_as_of(raw: &str) -> Result<i64> {
    let text = kassiber_pytext::strip(raw);
    let invalid = || {
        AppError::new("validation", format!("Invalid checkpoint date '{text}'")).with_hint(
            "Use a date (2024-12-31, read as the end of that UTC day) or a date-time with an offset (2024-12-31T23:59:59Z).",
        )
    };
    let (instant, offset) = time::parse_with_offset(text).ok_or_else(invalid)?;
    if text.len() == 10 {
        // The whole UTC day: every instant before the next midnight. Stored
        // and compared times have microsecond precision, so the last
        // microsecond of the day is that exclusive bound, inclusively.
        return Ok(instant + 86_400_000_000 - 1);
    }
    if !offset {
        return Err(AppError::new(
            "validation",
            format!("Checkpoint time '{text}' has no UTC offset"),
        )
        .with_hint("Add Z or an offset such as +01:00, or give the date alone."));
    }
    Ok(instant)
}

/// The canonical stored spelling of an instant (`...Z`).
pub fn iso(us: i64) -> Result<String> {
    time::format(us).ok_or_else(|| AppError::new("validation", "Checkpoint date is out of range"))
}

/// A checkpoint, stored (`id` set) or derived from stored sync state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkpoint {
    pub id: Option<String>,
    pub wallet_id: String,
    pub asset: String,
    pub as_of: String,
    pub amount_msat: i64,
    pub origin: String,
    pub scope: Option<String>,
    pub origin_ref: Option<String>,
    pub attachment_id: Option<String>,
    pub note: Option<String>,
    pub created_at: Option<String>,
}

/// A checkpoint to store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct New {
    pub wallet_id: String,
    pub asset: String,
    /// Any ISO instant with an offset; stored in canonical UTC form.
    pub as_of: String,
    pub amount_msat: i64,
    pub origin: &'static str,
    pub scope: Option<String>,
    pub origin_ref: Option<String>,
    pub attachment_id: Option<String>,
    pub note: Option<String>,
}

/// Store a checkpoint unless an identical one (same wallet, asset, origin,
/// time, amount and scope) exists, so re-importing the same file or replaying
/// a sync adds nothing. Returns the id and whether a row was written.
pub fn record(conn: &Connection, profile_id: &str, new: &New) -> Result<(String, bool)> {
    if !STORED_ORIGINS.contains(&new.origin) {
        return Err(AppError::new(
            "internal_error",
            format!("Checkpoint origin '{}' is not stored", new.origin),
        ));
    }
    if !ASSETS.contains(&new.asset.as_str()) {
        return Err(AppError::new(
            "validation",
            format!("Checkpoint asset must be BTC or LBTC, not '{}'", new.asset),
        ));
    }
    if new.amount_msat < 0 {
        return Err(AppError::new(
            "validation",
            "A reported balance cannot be negative",
        ));
    }
    let (instant, offset) = time::parse_with_offset(&new.as_of)
        .filter(|(_, offset)| *offset)
        .ok_or_else(|| {
            AppError::new(
                "validation",
                format!(
                    "Checkpoint time '{}' is not an ISO instant with an offset",
                    new.as_of
                ),
            )
        })?;
    debug_assert!(offset);
    let as_of = iso(instant)?;
    let workspace: Option<String> = conn
        .query_row(
            "SELECT workspace_id FROM wallets WHERE id=? AND profile_id=?",
            [&new.wallet_id, profile_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(database_error)?;
    let Some(workspace) = workspace else {
        return Err(AppError::new(
            "not_found",
            format!("Wallet '{}' not found", new.wallet_id),
        ));
    };
    if let Some(attachment) = &new.attachment_id {
        let found: Option<String> = conn
            .query_row(
                "SELECT id FROM attachments WHERE id=? AND profile_id=?",
                [attachment, profile_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(database_error)?;
        if found.is_none() {
            return Err(AppError::new(
                "not_found",
                format!("Attachment '{attachment}' not found"),
            ));
        }
    }
    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM balance_checkpoints WHERE profile_id=? AND wallet_id=? AND asset=? \
             AND origin=? AND as_of=? AND amount_msat=? AND scope IS ? AND note IS ? ORDER BY created_at,id LIMIT 1",
            params![
                profile_id,
                new.wallet_id,
                new.asset,
                new.origin,
                as_of,
                new.amount_msat,
                new.scope,
                new.note
            ],
            |r| r.get(0),
        )
        .optional()
        .map_err(database_error)?;
    if let Some(id) = existing {
        return Ok((id, false));
    }
    let id = crate::lifecycle::uuid(conn)?;
    let created = crate::lifecycle::now(conn)?;
    conn.execute(
        "INSERT INTO balance_checkpoints(id,workspace_id,profile_id,wallet_id,asset,as_of,amount_msat,\
         origin,scope,origin_ref,attachment_id,note,created_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)",
        params![
            id,
            workspace,
            profile_id,
            new.wallet_id,
            new.asset,
            as_of,
            new.amount_msat,
            new.origin,
            new.scope,
            new.origin_ref,
            new.attachment_id,
            new.note,
            created
        ],
    )
    .map_err(database_error)?;
    Ok((id, true))
}

/// Delete one stored checkpoint of this profile. Derived chain and node
/// checkpoints have no id and cannot be removed.
pub fn remove(conn: &Connection, profile_id: &str, id: &str) -> Result<Value> {
    let found = stored(conn, profile_id, None)?
        .into_iter()
        .find(|c| c.id.as_deref() == Some(id))
        .ok_or_else(|| AppError::new("not_found", format!("Checkpoint '{id}' not found")))?;
    conn.execute(
        "DELETE FROM balance_checkpoints WHERE id=? AND profile_id=?",
        [id, profile_id],
    )
    .map_err(database_error)?;
    Ok(dict([
        ("removed", Value::Bool(true)),
        ("id", s(id)),
        ("wallet_id", s(found.wallet_id)),
        ("origin", s(found.origin)),
    ]))
}

/// The stored checkpoints of a profile (or one wallet), oldest first.
pub fn stored(
    conn: &Connection,
    profile_id: &str,
    wallet_id: Option<&str>,
) -> Result<Vec<Checkpoint>> {
    let mut statement = conn
        .prepare(
            "SELECT id,wallet_id,asset,as_of,amount_msat,origin,scope,origin_ref,attachment_id,note,created_at \
             FROM balance_checkpoints WHERE profile_id=?1 AND (?2 IS NULL OR wallet_id=?2) \
             ORDER BY as_of,created_at,rowid",
        )
        .map_err(database_error)?;
    let rows = statement
        .query_map(params![profile_id, wallet_id], |r| {
            Ok(Checkpoint {
                id: Some(r.get(0)?),
                wallet_id: r.get(1)?,
                asset: r.get(2)?,
                as_of: r.get(3)?,
                amount_msat: r.get(4)?,
                origin: r.get(5)?,
                scope: r.get(6)?,
                origin_ref: r.get(7)?,
                attachment_id: r.get(8)?,
                note: r.get(9)?,
                created_at: r.get(10)?,
            })
        })
        .map_err(database_error)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(database_error)
}

/// `views._journal_freshness`'s status: `current`, `no_transactions`,
/// `not_processed` or `stale`.
pub fn journal_status(conn: &Connection, profile_id: &str) -> Result<&'static str> {
    let row = conn
        .query_row(
            "SELECT last_processed_at,last_processed_tx_count,journal_input_version,last_processed_input_version,\
             required_journal_semantics_version,journal_semantics_version,\
             (SELECT COUNT(*) FROM transactions t WHERE t.profile_id=p.id AND t.excluded=0) \
             FROM profiles p WHERE p.id=?",
            [profile_id],
            |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<i64>>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                    r.get::<_, Option<i64>>(4)?,
                    r.get::<_, Option<i64>>(5)?,
                    r.get::<_, i64>(6)?,
                ))
            },
        )
        .optional()
        .map_err(database_error)?;
    let Some((processed_at, count, input, done, required, built, active)) = row else {
        return Err(AppError::new("not_found", "Profile not found"));
    };
    let required = required.unwrap_or(0);
    let built = built.unwrap_or(0);
    let semantics = required == 0
        || (built == kassiber_store::semantics::JOURNAL_CALCULATION_SEMANTICS_VERSION
            && built >= required);
    Ok(if active == 0 {
        "no_transactions"
    } else if processed_at.as_deref().is_none_or(str::is_empty) {
        "not_processed"
    } else if count.unwrap_or(0) != active || input.unwrap_or(0) != done.unwrap_or(0) || !semantics
    {
        "stale"
    } else {
        "current"
    })
}

/// One checkpoint against the book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comparison {
    /// `matches`, `differs` or `unknown`.
    pub status: &'static str,
    /// Why: `matches`, `differs`, `journals_stale`, `no_history`,
    /// `before_coverage` or `undated_postings`.
    pub reason: &'static str,
    pub book_msat: Option<i128>,
    /// Reported minus book: positive when the source holds more.
    pub difference_msat: Option<i128>,
    /// Known, benign or explanatory conditions, as codes.
    pub notes: Vec<&'static str>,
    pub message: String,
}

/// Compare one checkpoint with the book: the wallet's journal quantity
/// postings in `asset` dated at or before `as_of`, exactly, in msat.
pub fn compare(
    conn: &Connection,
    profile_id: &str,
    checkpoint: &Checkpoint,
    journals: &str,
) -> Result<Comparison> {
    let unknown = |reason, message: String| Comparison {
        status: "unknown",
        reason,
        book_msat: None,
        difference_msat: None,
        notes: vec![],
        message,
    };
    let as_of = time::parse(&checkpoint.as_of).ok_or_else(|| {
        AppError::new(
            "internal_error",
            format!("Stored checkpoint time '{}' is invalid", checkpoint.as_of),
        )
    })?;
    if !matches!(journals, "current" | "no_transactions") {
        return Ok(unknown(
            "journals_stale",
            "Journals need processing before this balance can be compared.".into(),
        ));
    }
    // The wallet's covered range starts at its first active transaction.
    let mut first: Option<i64> = None;
    {
        let mut statement = conn
            .prepare(
                "SELECT occurred_at FROM transactions WHERE profile_id=? AND wallet_id=? AND excluded=0 AND asset=?",
            )
            .map_err(database_error)?;
        let mut rows = statement
            .query(params![profile_id, checkpoint.wallet_id, checkpoint.asset])
            .map_err(database_error)?;
        while let Some(row) = rows.next().map_err(database_error)? {
            let at: Option<String> = row.get(0).map_err(database_error)?;
            if let Some(at) = at.as_deref().and_then(time::parse) {
                first = Some(first.map_or(at, |f| f.min(at)));
            }
        }
    }
    let Some(first) = first else {
        return Ok(unknown(
            "no_history",
            format!(
                "The book has no {} history for this wallet to compare with; import or sync it first.",
                checkpoint.asset
            ),
        ));
    };
    if as_of < first {
        return Ok(unknown(
            "before_coverage",
            "This balance is dated before the wallet's first imported transaction; import the earlier history to compare it.".into(),
        ));
    }
    let mut book: i128 = 0;
    let mut later = false;
    let mut later_sum: i128 = 0;
    {
        let mut statement = conn
            .prepare(
                "SELECT occurred_at,amount_msat FROM journal_quantity_postings \
                 WHERE profile_id=? AND location_kind='wallet' AND location_id=? AND asset=?",
            )
            .map_err(database_error)?;
        let mut rows = statement
            .query(params![profile_id, checkpoint.wallet_id, checkpoint.asset])
            .map_err(database_error)?;
        while let Some(row) = rows.next().map_err(database_error)? {
            let at: Option<String> = row.get(0).map_err(database_error)?;
            let amount: i64 = row.get(1).map_err(database_error)?;
            let Some(at) = at.as_deref().and_then(time::parse) else {
                return Ok(unknown(
                    "undated_postings",
                    "Some of the wallet's journal postings have no date, so its balance at this time is unknown.".into(),
                ));
            };
            if at <= as_of {
                book += i128::from(amount);
            } else {
                later = true;
                later_sum += i128::from(amount);
            }
        }
    }
    let difference = i128::from(checkpoint.amount_msat) - book;
    // The chain inventory is as of the last refresh, but a transaction of the
    // same sync can carry a block time after it (a block timestamp ahead of
    // the local clock). Such later postings defer the comparison only while
    // they could explain the gap: same sign, no larger than their sum.
    let incomplete = checkpoint.origin == "chain" && difference != 0 && later;
    if incomplete
        && later_sum.signum() == difference.signum()
        && difference.abs() <= later_sum.abs()
    {
        return Ok(Comparison {
            status: "unknown",
            reason: "after_last_sync",
            book_msat: Some(book),
            difference_msat: Some(difference),
            notes: vec!["after_last_sync"],
            message: "The book has transactions dated after the last sync's coin snapshot that can explain the difference; the comparison is incomplete until the next sync.".into(),
        });
    }
    let mut notes = vec![];
    if incomplete {
        notes.push("after_last_sync");
    }
    let quarantined: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM journal_quarantines q JOIN transactions t ON t.id=q.transaction_id \
             WHERE q.profile_id=? AND t.wallet_id=? AND t.asset=?",
            params![profile_id, checkpoint.wallet_id, checkpoint.asset],
            |r| r.get(0),
        )
        .map_err(database_error)?;
    if quarantined > 0 {
        notes.push("quarantined_transactions");
    }
    let excluded: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transactions WHERE profile_id=? AND wallet_id=? AND asset=? AND excluded=1",
            params![profile_id, checkpoint.wallet_id, checkpoint.asset],
            |r| r.get(0),
        )
        .map_err(database_error)?;
    if excluded > 0 {
        notes.push("excluded_transactions");
    }
    if difference == 0 {
        return Ok(Comparison {
            status: "matches",
            reason: "matches",
            book_msat: Some(book),
            difference_msat: Some(0),
            notes,
            message: format!(
                "The book holds the reported {} {}.",
                format_btc(book),
                checkpoint.asset
            ),
        });
    }
    let mut message = format!(
        "The source reported {} {} but the book holds {} {} at that time (difference {}{}). The source may be incomplete: import or sync the missing history, then process journals.",
        format_btc(i128::from(checkpoint.amount_msat)),
        checkpoint.asset,
        format_btc(book),
        checkpoint.asset,
        if difference > 0 { "+" } else { "" },
        format_btc(difference),
    );
    if quarantined > 0 {
        message.push_str(&format!(
            " {quarantined} quarantined transaction(s) of this wallet are not in the book balance."
        ));
    }
    if excluded > 0 {
        message.push_str(&format!(
            " {excluded} excluded transaction(s) of this wallet are not in the book balance."
        ));
    }
    if incomplete {
        message.push_str(" The book also holds transactions dated after this sync's coin snapshot that cannot explain this difference; the comparison is incomplete until the next sync.");
    }
    if let Some(explained) = checkpoint.scope.as_deref().and_then(scope_note) {
        message.push(' ');
        message.push_str(explained);
    }
    Ok(Comparison {
        status: "differs",
        reason: "differs",
        book_msat: Some(book),
        difference_msat: Some(difference),
        notes,
        message,
    })
}

/// What a reported balance covers, for scopes whose number can legitimately
/// differ from the imported history. Exchange modules name their scope.
pub fn scope_note(scope: &str) -> Option<&'static str> {
    Some(match scope {
        "kraken_spot" => "Kraken's figure is the spot balance (XXBT/XBT); staked and Earn balances (XBT.F, XBT.B, XBT.S, XBT.M, XBT.P) and futures are excluded, but funds still in transit are included.",
        "coinbase_btc_accounts" => "Coinbase's figure is the sum of every BTC account's balance, including vaults; a pending send or receive may already be included.",
        "binance_spot_free_plus_locked" => "Binance's figure is the spot wallet (free plus locked in open orders); Earn, Funding, margin and futures wallets are excluded. Binance spot trades are not imported from the API, so they appear as a difference.",
        "bitpanda_wallet_balance_after" => "Bitpanda's figure is each BTC wallet's balance after its latest BTC transaction; staking and Earn balances held outside those wallets are not included.",
        "kraken_ledger_balance" => "Kraken's ledger export balance is the spot balance after the file's last BTC row.",
        "file_running_balance" => "The file's running balance is the balance after its last row.",
        "chain_utxo_set" => "The chain figure is the unspent outputs the last sync observed; a transaction seen after that sync is not included.",
        "cln_bookkeeper_accounts" => "The node figure is the sum of Core Lightning's bookkeeper accounts (on-chain wallet and channels) at its last sync.",
        _ => return None,
    })
}

/// A checkpoint and its comparison as the surfaces show it.
pub fn entry(checkpoint: &Checkpoint, wallet_label: &str, comparison: &Comparison) -> Value {
    let opt = |v: &Option<String>| v.as_deref().map_or(Value::Null, s);
    let msat = |v: Option<i128>| v.map_or(Value::Null, |v| Value::Int(v.into()));
    let btc = |v: Option<i128>| v.map_or(Value::Null, |v| s(format_btc(v)));
    dict([
        ("id", opt(&checkpoint.id)),
        ("stored", Value::Bool(checkpoint.id.is_some())),
        ("wallet_id", s(checkpoint.wallet_id.as_str())),
        ("wallet", s(wallet_label)),
        ("asset", s(checkpoint.asset.as_str())),
        ("as_of", s(checkpoint.as_of.as_str())),
        ("origin", s(checkpoint.origin.as_str())),
        ("scope", opt(&checkpoint.scope)),
        ("origin_ref", opt(&checkpoint.origin_ref)),
        ("attachment_id", opt(&checkpoint.attachment_id)),
        ("note", opt(&checkpoint.note)),
        ("created_at", opt(&checkpoint.created_at)),
        ("reported_msat", Value::Int(checkpoint.amount_msat.into())),
        ("reported_btc", s(format_btc(checkpoint.amount_msat.into()))),
        ("status", s(comparison.status)),
        ("reason", s(comparison.reason)),
        ("book_msat", msat(comparison.book_msat)),
        ("book_btc", btc(comparison.book_msat)),
        ("difference_msat", msat(comparison.difference_msat)),
        ("difference_btc", btc(comparison.difference_msat)),
        (
            "notes",
            Value::List(comparison.notes.iter().map(|n| s(*n)).collect()),
        ),
        ("message", s(comparison.message.as_str())),
    ])
}

/// The Core Lightning bookkeeper balances of the wallet's last successful
/// node sync, as a checkpoint at that sync's completion. Read from stored
/// `balance_snapshot` records only.
pub fn node_checkpoint(
    conn: &Connection,
    profile_id: &str,
    wallet_id: &str,
) -> Result<Option<Checkpoint>> {
    let sync: Option<(String, String)> = conn
        .query_row(
            "SELECT id,completed_at FROM lightning_node_syncs WHERE profile_id=? AND wallet_id=? \
             AND status='synced' AND completed_at IS NOT NULL ORDER BY completed_at DESC,id DESC LIMIT 1",
            [profile_id, wallet_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(database_error)?;
    let Some((sync_id, completed_at)) = sync else {
        return Ok(None);
    };
    let (count, total): (i64, Option<i64>) = conn
        .query_row(
            "SELECT COUNT(*),SUM(amount_msat) FROM lightning_node_records WHERE profile_id=? AND wallet_id=? \
             AND record_type='balance_snapshot' AND sync_id=?",
            [profile_id, wallet_id, sync_id.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(database_error)?;
    let Some(at) = time::parse(&completed_at) else {
        return Ok(None);
    };
    if count == 0 {
        return Ok(None);
    }
    let Some(total) = total.filter(|t| *t >= 0) else {
        return Ok(None);
    };
    Ok(Some(Checkpoint {
        id: None,
        wallet_id: wallet_id.into(),
        asset: "BTC".into(),
        as_of: iso(at)?,
        amount_msat: total,
        origin: "node".into(),
        scope: Some("cln_bookkeeper_accounts".into()),
        origin_ref: Some(format!("lightning_sync:{sync_id}")),
        attachment_id: None,
        note: None,
        created_at: None,
    }))
}

/// Severity order for a wallet's overall state.
fn rank(status: &str) -> u8 {
    match status {
        "differs" => 3,
        "unknown" => 2,
        "matches" => 1,
        _ => 0,
    }
}

/// Every checkpoint of a profile (or one wallet), stored and `derived`,
/// compared with the book. `history` lists every checkpoint; otherwise only
/// the latest per wallet, origin and asset. Local reads only.
pub fn report(
    conn: &Connection,
    profile_id: &str,
    wallet_id: Option<&str>,
    derived: Vec<Checkpoint>,
    history: bool,
) -> Result<Value> {
    let journals = journal_status(conn, profile_id)?;
    let mut wallets: Vec<(String, String, String)> = vec![];
    {
        let mut statement = conn
            .prepare(
                "SELECT id,label,kind FROM wallets WHERE profile_id=?1 AND (?2 IS NULL OR id=?2) ORDER BY label,id",
            )
            .map_err(database_error)?;
        let rows = statement
            .query_map(params![profile_id, wallet_id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .map_err(database_error)?;
        for row in rows {
            wallets.push(row.map_err(database_error)?);
        }
    }
    let mut all = stored(conn, profile_id, wallet_id)?;
    // Insertion order of the stored rows. `created_at` has second precision
    // and ids are random, so a tie falls back to the rowid: each insert gets
    // one above every row the table holds.
    let mut inserted: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    {
        let mut statement = conn
            .prepare("SELECT id,rowid FROM balance_checkpoints WHERE profile_id=?1")
            .map_err(database_error)?;
        let rows = statement
            .query_map([profile_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(database_error)?;
        for row in rows {
            let (id, rowid) = row.map_err(database_error)?;
            inserted.insert(id, rowid);
        }
    }
    let order = |c: &Checkpoint| c.id.as_ref().and_then(|id| inserted.get(id)).copied();
    for wallet in &wallets {
        if let Some(node) = node_checkpoint(conn, profile_id, &wallet.0)? {
            all.push(node);
        }
    }
    all.extend(
        derived
            .into_iter()
            .filter(|c| wallet_id.is_none_or(|w| w == c.wallet_id)),
    );
    let mut wallet_rows = vec![];
    let mut entries = vec![];
    let (mut matches, mut differs, mut unknown) = (0i64, 0i64, 0i64);
    for (id, label, kind) in &wallets {
        let mut mine: Vec<(i64, &Checkpoint)> = all
            .iter()
            .filter(|c| c.wallet_id == *id)
            .map(|c| (time::parse(&c.as_of).unwrap_or(i64::MIN), c))
            .collect();
        // Newest first; a later-created row wins a tie, then a later-inserted one.
        mine.sort_by(|(a, x), (b, y)| {
            b.cmp(a)
                .then_with(|| y.created_at.cmp(&x.created_at))
                .then_with(|| order(y).cmp(&order(x)))
                .then_with(|| y.id.cmp(&x.id))
        });
        let mut seen: Vec<(&str, &str)> = vec![];
        let mut latest = vec![];
        let mut worst = "none";
        for (_, checkpoint) in &mine {
            let key = (checkpoint.origin.as_str(), checkpoint.asset.as_str());
            let is_latest = !seen.contains(&key);
            if !is_latest && !history {
                continue;
            }
            let comparison = compare(conn, profile_id, checkpoint, journals)?;
            let mut value = entry(checkpoint, label, &comparison);
            kassiber_envelope::set(&mut value, "latest", Value::Bool(is_latest));
            if is_latest {
                seen.push(key);
                match comparison.status {
                    "matches" => matches += 1,
                    "differs" => differs += 1,
                    _ => unknown += 1,
                }
                if rank(comparison.status) > rank(worst) {
                    worst = comparison.status;
                }
                latest.push(value.clone());
            }
            entries.push(value);
        }
        if wallet_id.is_some() || !latest.is_empty() {
            wallet_rows.push(dict([
                ("wallet_id", s(id.as_str())),
                ("wallet", s(label.as_str())),
                ("kind", s(kind.as_str())),
                ("status", s(worst)),
                ("latest", Value::List(latest)),
            ]));
        }
    }
    if let Some(wallet) = wallet_id {
        if wallets.is_empty() {
            return Err(AppError::new(
                "not_found",
                format!("Wallet '{wallet}' not found"),
            ));
        }
    }
    Ok(dict([
        ("journals", dict([("status", s(journals))])),
        (
            "counts",
            dict([
                ("matches", Value::Int(matches.into())),
                ("differs", Value::Int(differs.into())),
                ("unknown", Value::Int(unknown.into())),
            ]),
        ),
        ("wallets", Value::List(wallet_rows)),
        ("checkpoints", Value::List(entries)),
    ]))
}

/// A manual checkpoint from user-entered text.
pub struct Manual<'a> {
    pub wallet: &'a str,
    pub btc: &'a str,
    pub as_of: &'a str,
    pub asset: Option<&'a str>,
    pub note: Option<&'a str>,
    pub attachment: Option<&'a str>,
}

fn amount(text: &str) -> Result<i64> {
    parse_btc_msat(kassiber_pytext::strip(text)).ok_or_else(|| {
        AppError::new("validation", format!("Invalid BTC amount '{text}'")).with_hint(
            "Give the balance in BTC as a plain decimal with at most 11 decimals, e.g. 0.12345678.",
        )
    })
}

fn asset(raw: Option<&str>) -> Result<String> {
    let code = kassiber_pytext::upper(kassiber_pytext::strip(raw.unwrap_or("BTC")));
    let code = if code.is_empty() {
        "BTC".to_owned()
    } else if code == "L-BTC" {
        "LBTC".to_owned()
    } else {
        code
    };
    if !ASSETS.contains(&code.as_str()) {
        return Err(AppError::new(
            "validation",
            format!("Balance checkpoints track BTC or LBTC, not '{code}'"),
        )
        .with_hint("Kassiber keeps Bitcoin only; altcoin and stablecoin balances are refused."));
    }
    Ok(code)
}

fn note(raw: Option<&str>) -> Option<String> {
    raw.map(|n| kassiber_pytext::strip(n).to_owned())
        .filter(|n| !n.is_empty())
}

/// `checkpoints add`: the user's own "this wallet held N BTC on <date>".
pub fn add_manual(conn: &Connection, profile_id: &str, manual: &Manual<'_>) -> Result<Value> {
    let wallet = crate::resolve(conn, profile_id, manual.wallet)?;
    let amount_msat = amount(manual.btc)?;
    let as_of = iso(parse_as_of(manual.as_of)?)?;
    let new = New {
        wallet_id: wallet.id.clone(),
        asset: asset(manual.asset)?,
        as_of,
        amount_msat,
        origin: "manual",
        scope: None,
        origin_ref: None,
        attachment_id: manual
            .attachment
            .map(|a| kassiber_pytext::strip(a).to_owned())
            .filter(|a| !a.is_empty()),
        note: note(manual.note),
    };
    let (id, created) = record(conn, profile_id, &new)?;
    let checkpoint = stored(conn, profile_id, Some(&wallet.id))?
        .into_iter()
        .find(|c| c.id.as_deref() == Some(id.as_str()))
        .expect("recorded checkpoint");
    let journals = journal_status(conn, profile_id)?;
    let comparison = compare(conn, profile_id, &checkpoint, journals)?;
    let mut value = entry(&checkpoint, &wallet.label, &comparison);
    kassiber_envelope::set(&mut value, "created", Value::Bool(created));
    Ok(value)
}

/// One CSV field, quoted when it holds a separator, quote or line break.
fn csv_field(text: &str) -> String {
    if text.contains([',', '"', '\n', '\r']) || text.starts_with([' ', '\t']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_owned()
    }
}

/// Stored checkpoints as CSV (`date,wallet,btc_amount,asset,origin,note`),
/// oldest first. Wallets are named by label for a readable handoff.
pub fn export_csv(conn: &Connection, profile_id: &str, wallet_id: Option<&str>) -> Result<String> {
    let labels: std::collections::BTreeMap<String, String> = {
        let mut statement = conn
            .prepare("SELECT id,label FROM wallets WHERE profile_id=?")
            .map_err(database_error)?;
        let rows = statement
            .query_map([profile_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(database_error)?;
        rows.collect::<std::result::Result<_, _>>()
            .map_err(database_error)?
    };
    let mut out = CSV_HEADER.join(",");
    out.push('\n');
    for checkpoint in stored(conn, profile_id, wallet_id)? {
        let label = labels
            .get(&checkpoint.wallet_id)
            .cloned()
            .unwrap_or_default();
        // A leading formula character would run in a spreadsheet.
        let safe = |text: &str| {
            if text.starts_with(['=', '+', '-', '@']) {
                format!("'{text}")
            } else {
                text.to_owned()
            }
        };
        let fields = [
            checkpoint.as_of.clone(),
            checkpoint.wallet_id.clone(),
            safe(&label),
            format_btc(checkpoint.amount_msat.into()),
            checkpoint.asset.clone(),
            checkpoint.origin.clone(),
            safe(checkpoint.note.as_deref().unwrap_or("")),
            checkpoint.id.clone().unwrap_or_default(),
        ];
        out.push_str(
            &fields
                .iter()
                .map(|f| csv_field(f))
                .collect::<Vec<_>>()
                .join(","),
        );
        out.push('\n');
    }
    Ok(out)
}

/// RFC 4180 records: quoted fields may hold separators, quotes and breaks.
fn csv_records(text: &str) -> Result<Vec<Vec<String>>> {
    let mut records = vec![];
    let mut record = vec![];
    let mut field = String::new();
    let mut chars = text.trim_start_matches('\u{feff}').chars().peekable();
    let mut quoted = false;
    let mut started = false;
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => quoted = false,
                other => field.push(other),
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => {
                quoted = true;
                started = true;
            }
            ',' => {
                record.push(std::mem::take(&mut field));
                started = true;
            }
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' | '\r' => {
                record.push(std::mem::take(&mut field));
                if !(record.len() == 1 && record[0].is_empty() && !started) {
                    records.push(std::mem::take(&mut record));
                } else {
                    record.clear();
                }
                started = false;
            }
            other => {
                field.push(other);
                started = true;
            }
        }
        if records.len() > MAX_IMPORT_ROWS {
            return Err(AppError::new(
                "validation",
                format!("Checkpoint CSV has more than {MAX_IMPORT_ROWS} rows"),
            ));
        }
    }
    if quoted {
        return Err(AppError::new(
            "validation",
            "Checkpoint CSV ends inside a quoted field",
        ));
    }
    if started || !field.is_empty() {
        record.push(field);
        records.push(record);
    }
    Ok(records)
}

/// `checkpoints import`: CSV rows (`date,wallet,btc_amount[,asset][,origin][,note]`)
/// become manual checkpoints. Every row is checked before any is stored;
/// one bad row refuses the file. An `origin` column is kept only as a note:
/// a balance read from a file is the user's entry, not an API or chain fact.
pub fn import_csv(conn: &Connection, profile_id: &str, text: &str, source: &str) -> Result<Value> {
    let records = csv_records(text)?;
    let Some((header, rows)) = records.split_first() else {
        return Err(AppError::new("validation", "Checkpoint CSV is empty"));
    };
    let header: Vec<String> = header
        .iter()
        .map(|h| kassiber_pytext::lower(kassiber_pytext::strip(h)))
        .collect();
    let column = |name: &str| header.iter().position(|h| h == name);
    let (wallet, wallet_id_at) = (column("wallet"), column("wallet_id"));
    let (Some(date), Some(btc), true) = (
        column("date"),
        column("btc_amount"),
        wallet.is_some() || wallet_id_at.is_some(),
    ) else {
        return Err(AppError::new(
            "validation",
            "Checkpoint CSV needs date, wallet (or wallet_id) and btc_amount columns",
        )
        .with_hint("Use the header date,wallet_id,wallet,btc_amount,asset,origin,note (wallet_id, asset, origin and note are optional)."));
    };
    let (asset_at, origin_at, note_at) = (column("asset"), column("origin"), column("note"));
    let id_at = column("checkpoint_id");
    let mut prepared = vec![];
    for (index, row) in rows.iter().enumerate() {
        let line = index + 2;
        if row.iter().all(|f| kassiber_pytext::strip(f).is_empty()) {
            continue;
        }
        let at = |i: Option<usize>| i.and_then(|i| row.get(i)).map(String::as_str);
        let fail = |error: AppError| {
            let message = format!("Row {line}: {}", error.message);
            let mut refused = AppError::new(error.code.clone(), message);
            if let Some(hint) = error.hint.as_deref() {
                refused = refused.with_hint(hint.to_owned());
            }
            refused
        };
        let wallet = import_wallet(conn, profile_id, at(wallet_id_at), at(wallet)).map_err(fail)?;
        let amount_msat = amount(at(Some(btc)).unwrap_or("")).map_err(fail)?;
        let as_of = parse_as_of(at(Some(date)).unwrap_or(""))
            .and_then(iso)
            .map_err(fail)?;
        let asset = asset(at(asset_at)).map_err(fail)?;
        // A row this book exported names its checkpoint: when that row is
        // still stored exactly as exported, the import leaves it alone.
        let origin = note(at(origin_at)).unwrap_or_else(|| "manual".into());
        let known = match note(at(id_at)) {
            Some(id) => exported_known(
                conn,
                profile_id,
                &id,
                &wallet.id,
                &asset,
                &as_of,
                amount_msat,
                &origin,
                note(at(note_at)).as_deref(),
            )?,
            None => false,
        };
        let mut note_text = note(at(note_at)).unwrap_or_default();
        if let Some(origin) = Some(origin.clone()).filter(|o| o != "manual") {
            if !ORIGINS.contains(&origin.as_str()) {
                return Err(fail(AppError::new(
                    "validation",
                    format!("Unknown origin '{origin}'"),
                )));
            }
            let prefix = format!("imported {origin} checkpoint");
            note_text = if note_text.is_empty() {
                prefix
            } else {
                format!("{prefix}: {note_text}")
            };
        }
        prepared.push((
            known,
            New {
                wallet_id: wallet.id,
                asset,
                as_of,
                amount_msat,
                origin: "manual",
                scope: None,
                origin_ref: Some(format!("csv:{source}")),
                attachment_id: None,
                note: (!note_text.is_empty()).then_some(note_text),
            },
        ));
    }
    let (mut created, mut unchanged) = (0i64, 0i64);
    kassiber_store::begin_write_savepoint(conn, "checkpoint_import").map_err(database_error)?;
    let result = prepared.iter().try_for_each(|(known, new)| {
        // An exported row coming back adds nothing; any other row is the
        // user's own statement, never gaining another origin's authority.
        let written = !known && record(conn, profile_id, new)?.1;
        if written {
            created += 1;
        } else {
            unchanged += 1;
        }
        Ok::<(), AppError>(())
    });
    match &result {
        Ok(()) => conn.execute_batch("RELEASE checkpoint_import"),
        Err(_) => conn.execute_batch("ROLLBACK TO checkpoint_import;RELEASE checkpoint_import"),
    }
    .map_err(database_error)?;
    result?;
    Ok(dict([
        ("rows", Value::Int((prepared.len() as i64).into())),
        ("created", Value::Int(created.into())),
        ("unchanged", Value::Int(unchanged.into())),
    ]))
}

/// The wallet an import row names: its `wallet_id` in this book first, then
/// the label, then the label without the `'` export added in front of a
/// formula character.
fn import_wallet(
    conn: &Connection,
    profile_id: &str,
    wallet_id: Option<&str>,
    label: Option<&str>,
) -> Result<crate::Wallet> {
    if let Some(id) = wallet_id
        .map(kassiber_pytext::strip)
        .filter(|id| !id.is_empty())
    {
        let found: Option<String> = conn
            .query_row(
                "SELECT id FROM wallets WHERE profile_id=? AND id=?",
                [profile_id, id],
                |r| r.get(0),
            )
            .optional()
            .map_err(database_error)?;
        if let Some(found) = found {
            return crate::resolve(conn, profile_id, &found);
        }
    }
    let label = label.unwrap_or("");
    match crate::resolve(conn, profile_id, label) {
        Err(error) if error.code == "not_found" => match label.strip_prefix('\'') {
            Some(rest) if rest.starts_with(['=', '+', '-', '@']) => {
                crate::resolve(conn, profile_id, rest)
            }
            _ => Err(error),
        },
        other => other,
    }
}

/// Whether an import row is a checkpoint this book exported and still holds
/// exactly: the same id, wallet, asset, time, amount, origin and note. The
/// note is compared with its exported form too (a formula-like note is
/// exported behind a `'`), so an edited note is new evidence, never a replay.
#[allow(clippy::too_many_arguments)]
fn exported_known(
    conn: &Connection,
    profile_id: &str,
    id: &str,
    wallet_id: &str,
    asset: &str,
    as_of: &str,
    amount_msat: i64,
    origin: &str,
    imported_note: Option<&str>,
) -> Result<bool> {
    let stored: Option<Option<String>> = conn
        .query_row(
            "SELECT note FROM balance_checkpoints WHERE profile_id=? AND id=? AND wallet_id=? \
             AND asset=? AND as_of=? AND amount_msat=? AND origin=?",
            params![profile_id, id, wallet_id, asset, as_of, amount_msat, origin],
            |r| r.get(0),
        )
        .optional()
        .map_err(database_error)?;
    let Some(stored) = stored else {
        return Ok(false);
    };
    let stored = note(stored.as_deref());
    Ok(match (stored.as_deref(), imported_note) {
        (None, None) => true,
        (Some(kept), Some(imported)) => {
            imported == kept
                || (kept.starts_with(['=', '+', '-', '@'])
                    && imported.strip_prefix('\'') == Some(kept))
        }
        _ => false,
    })
}

#[cfg(test)]
mod tests;
