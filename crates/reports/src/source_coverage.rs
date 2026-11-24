//! Sources that do not reach a report period's end (Rust only;
//! docs/reference/desktop.md#sources-live-sync-and-file-snapshots).
//!
//! Exports carry them so the exported figures say they are provisional, as
//! the Reports page does. The export still proceeds and no number changes:
//! this reads the coverage model only (book and file metadata, never a
//! network or file contents).
use kassiber_envelope::{dict, s, AppError};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::Connection;
use kassiber_wallets::coverage::{self, day_of};

/// `{period_end, sources}`: every source of `wallets` (all when `None`) that
/// is behind, failed, or never imported for the day of `period_end`.
pub fn model(
    conn: &Connection,
    profile: &str,
    wallets: Option<&[String]>,
    period_end: &str,
    now: &str,
) -> Result<Value, AppError> {
    let Some(day) = day_of(period_end).or_else(|| day_of(now)) else {
        return Ok(dict([
            ("period_end", Value::Null),
            ("sources", Value::List(vec![])),
        ]));
    };
    let sources = coverage::sources(
        conn,
        profile,
        &coverage::Options {
            period_end: Some(&day),
            skip_scan_bounds: true,
            ..coverage::Options::default()
        },
        now,
    )?;
    let rows = sources
        .into_iter()
        .filter(|source| source.needs_action())
        .filter(|source| wallets.is_none_or(|ids| ids.contains(&source.wallet_id)))
        .map(|source| {
            dict([
                ("wallet", s(source.label.as_str())),
                ("source_mode", s(source.source_mode)),
                ("freshness_state", s(source.freshness_state)),
                (
                    "covered_through",
                    source
                        .covered_through
                        .as_deref()
                        .and_then(day_of)
                        .map_or(s(""), s),
                ),
                ("action", s(source.action)),
            ])
        })
        .collect();
    Ok(dict([
        ("period_end", s(day)),
        ("sources", Value::List(rows)),
    ]))
}

/// Adds `source_coverage` to an export model when a source needs action.
/// A model whose sources all reach the period end stays unchanged, so the
/// export of a current book is byte for byte what it was.
pub fn attach(
    conn: &Connection,
    model: &mut Value,
    profile: &str,
    wallets: Option<&[String]>,
    period_end: &str,
    now: &str,
) -> Result<(), AppError> {
    let coverage = self::model(conn, profile, wallets, period_end, now)?;
    let lagging = matches!(&coverage, Value::Dict(pairs)
        if pairs.iter().any(|(k, v)| *k == "sources".into()
            && matches!(v, Value::List(rows) if !rows.is_empty())));
    if let (true, Value::Dict(fields)) = (lagging, model) {
        fields.push(("source_coverage".into(), coverage));
    }
    Ok(())
}

/// A report period's end: the tax year's last day, or today for the year
/// still running.
pub fn year_end(year: i64, now: &str) -> String {
    let end = format!("{year:04}-12-31");
    match day_of(now) {
        Some(today) if today < end => today,
        _ => end,
    }
}
