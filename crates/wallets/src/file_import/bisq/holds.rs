//! Bisq trades the importer cannot shape are held in the book, not in a
//! wallet: `provider_trade_holds` (an opt-in table the first hold creates)
//! keeps each held trade with the on-chain txids its Funds > Transactions rows
//! name. Journal processing quarantines every live transaction a hold names
//! until a custody review covers it or the owner excludes it, so a held
//! trade's deposit never books as an ordinary disposal. Txids only accumulate;
//! a hold clears only when a later export completes the trade with escrow
//! transactions covering every txid it named, and stays cleared only while
//! that trade's legs are in the book: rolling the completing import back
//! opens it again ([`OPEN`]). See `docs/exchanges/bisq.md` and
//! `docs/reference/database-compatibility.md#opt-in-schema-groups`.
use super::super::*;
use super::Parsed;
use kassiber_store::optional::{self, PROVIDER_TRADE_HOLDS};
use std::collections::BTreeSet;

/// The quarantine reason journals store.
pub const REASON: &str = "bisq_trade_review_required";
const PROVIDER: &str = "bisq";
/// SQL predicate on a hold row aliased `h`: held, or cleared while none of
/// the completed trade's legs (`bisq:<trade>:<leg>`) are live in the book.
/// The clearance rests on those legs; an imports rollback deletes them and
/// so re-opens the hold. `kassiber_setup::network_partition` repeats it.
pub const OPEN: &str = "(h.state='held' OR NOT EXISTS(SELECT 1 FROM transactions l WHERE l.profile_id=h.profile_id AND l.excluded=0 AND substr(l.external_id,1,length(h.provider)+length(h.trade_id)+2)=h.provider||':'||h.trade_id||':'))";

/// One held trade and the on-chain txids Funds > Transactions names for it.
pub(in super::super) struct Hold {
    pub trade: String,
    pub status: String,
    pub reason: &'static str,
    pub txids: Vec<String>,
}

fn present(conn: &Connection) -> Result<bool, AppError> {
    optional::present(conn, PROVIDER_TRADE_HOLDS).map_err(|e| AppError::new(e.code, e.message))
}
fn strings(text: &str) -> BTreeSet<String> {
    serde_json::from_str(text).unwrap_or_default()
}

/// The txids an open hold on `trade` names.
pub(super) fn anchors(
    conn: &Connection,
    profile: &str,
    trade: &str,
) -> Result<BTreeSet<String>, AppError> {
    if !present(conn)? {
        return Ok(BTreeSet::new());
    }
    let sql = format!("SELECT txids_json FROM provider_trade_holds h WHERE profile_id=? AND provider=? AND trade_id=? AND {OPEN}");
    let txids: Option<String> = conn
        .query_row(&sql, params![profile, PROVIDER, trade], |r| r.get(0))
        .optional()
        .map_err(database_error)?;
    Ok(txids.as_deref().map(strings).unwrap_or_default())
}

/// Record this import's holds and clear the ones its completed trades prove
/// resolved. Returns whether anything changed.
pub(super) fn persist(
    conn: &Connection,
    workspace: &str,
    profile: &str,
    parsed: &Parsed,
    now: &str,
) -> Result<bool, AppError> {
    if !present(conn)? {
        if parsed.held.is_empty() {
            return Ok(false);
        }
        optional::ensure(conn, PROVIDER_TRADE_HOLDS)
            .map_err(|e| AppError::new(e.code, e.message))?;
    }
    let mut changed = false;
    for hold in &parsed.held {
        let existing: Option<(String, String, String, String)> = conn
            .query_row(
                "SELECT txids_json,reason,trade_status,state FROM provider_trade_holds WHERE profile_id=? AND provider=? AND trade_id=?",
                params![profile, PROVIDER, hold.trade],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()
            .map_err(database_error)?;
        // Anchors only accumulate; the latest reason and status show.
        let mut txids = existing.as_ref().map(|e| strings(&e.0)).unwrap_or_default();
        txids.extend(hold.txids.iter().cloned());
        let txids = config::stored_json(&json!(txids));
        let fresh = (
            txids.clone(),
            hold.reason.into(),
            hold.status.clone(),
            "held".into(),
        );
        if existing.as_ref() == Some(&fresh) {
            continue;
        }
        if existing.is_some() {
            conn.execute(
                "UPDATE provider_trade_holds SET trade_status=?,reason=?,state='held',txids_json=?,cleared_reason=NULL,updated_at=? WHERE profile_id=? AND provider=? AND trade_id=?",
                params![hold.status, hold.reason, txids, now, profile, PROVIDER, hold.trade],
            )
        } else {
            conn.execute(
                "INSERT INTO provider_trade_holds(id,workspace_id,profile_id,provider,trade_id,trade_status,reason,state,txids_json,created_at,updated_at) VALUES(?,?,?,?,?,?,?,'held',?,?,?)",
                params![lifecycle::uuid(conn)?, workspace, profile, PROVIDER, hold.trade, hold.status, hold.reason, txids, now, now],
            )
        }
        .map_err(database_error)?;
        changed = true;
    }
    // A completed trade's escrow transactions cover every txid its hold
    // named: the parser holds it again otherwise.
    for trade in &parsed.completed {
        changed |= conn
            .execute(
                "UPDATE provider_trade_holds SET state='cleared',cleared_reason='trade_completed',updated_at=? WHERE profile_id=? AND provider=? AND trade_id=? AND state='held'",
                params![now, profile, PROVIDER, trade],
            )
            .map_err(database_error)?
            > 0;
    }
    Ok(changed)
}

/// Journal quarantine records for every live transaction an open hold names
/// that no active custody review covers, in the shape `journals process`
/// stores.
pub fn quarantines(conn: &Connection, profile: &str) -> Result<Vec<Py>, AppError> {
    if !present(conn)? {
        return Ok(Vec::new());
    }
    let sql = format!("SELECT t.id,t.workspace_id,h.trade_id,h.trade_status,h.reason,lower(t.external_id) FROM provider_trade_holds h JOIN json_each(h.txids_json) j JOIN transactions t ON t.profile_id=h.profile_id AND t.excluded=0 AND lower(t.external_id)=j.value WHERE h.profile_id=? AND {OPEN} AND NOT EXISTS(SELECT 1 FROM custody_component_transaction_memberships m JOIN custody_components c ON c.id=m.component_id WHERE m.profile_id=h.profile_id AND m.transaction_id=t.id AND c.state='active') ORDER BY h.provider,h.trade_id,j.value,t.id");
    let mut statement = conn.prepare(&sql).map_err(database_error)?;
    let rows = statement
        .query_map([profile], |r| {
            (0..6)
                .map(|i| r.get::<_, String>(i))
                .collect::<Result<Vec<_>, _>>()
        })
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)?;
    let mut named = BTreeSet::new();
    let text = |v: &str| Py::Str(v.into());
    Ok(rows
        .into_iter()
        .filter(|row| named.insert(row[0].clone()))
        .map(|row| {
            let detail = dict(vec![
                ("provider", text("Bisq")),
                ("trade_id", text(&row[2])),
                ("trade_status", text(&row[3])),
                ("hold_reason", text(&row[4])),
                ("txid", text(&row[5])),
                ("resolution", text("Record what the trade actually moved: review the named transactions with a custody component or pair (or exclude one that is not yours). The hold clears for each transaction you review.")),
            ]);
            let detail =
                kassiber_pyjson::dumps(&detail, &kassiber_pyjson::Options::new().sort_keys(true))
                    .expect("plain JSON");
            dict(vec![
                ("transaction_id", text(&row[0])),
                ("workspace_id", text(&row[1])),
                ("profile_id", text(profile)),
                ("reason", text(REASON)),
                ("detail_json", text(&detail)),
            ])
        })
        .collect())
}
