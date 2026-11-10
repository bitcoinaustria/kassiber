//! Compatibility for positional CSV identities and their duplicate review,
//! from `core/import_identity.py`.
//!
//! Import matching and detection are read-only; only an explicit owner review
//! writes, appending a confirmation that changes no transaction.
use crate::database_error;
use crate::file_import::synthetic_csv_identity;
use kassiber_envelope::AppError;
use kassiber_pyjson::{Key, Options, Value as Py};
use kassiber_store::rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub const DUPLICATE_REVIEW: &str = "synthetic_import_duplicate_review_required";
pub const CONFIRM_DISTINCT: &str = "confirm_distinct_import_rows";
pub const RESOLUTION: &str = "Review the source exports. If some rows are copies of one trade, use metadata exclude --transaction <id> for the duplicate rows. If every row is a genuine separate trade, apply the confirm_distinct_import_rows review with this group_fingerprint. Then rebuild journals. No survivor is selected automatically.";
pub const CROSS_IMPORT_EXCESS: &str = "cross_import_excess";
pub const PRE_BATCH_EXCESS: &str = "pre_batch_excess";
pub const REFILLED_EXCLUDED_COPY: &str = "refilled_excluded_copy";
const REFILLED_NOTE: &str = "A later import added a row in the place of a copy that was excluded earlier (refilled_excluded_transaction_ids). If that excluded row was the same trade, exclude the new row; confirming distinct trades counts the excluded trade again through the new row.";
const PRE_BATCH_NOTE: &str = "Every row predates import batches, so Kassiber counts only rows stored in the same second as one export. One import that crossed a second boundary is also held; confirm it as distinct trades.";

fn hold_note(reason: &str) -> Option<&'static str> {
    match reason {
        REFILLED_EXCLUDED_COPY => Some(REFILLED_NOTE),
        PRE_BATCH_EXCESS => Some(PRE_BATCH_NOTE),
        _ => None,
    }
}

/// One stored occurrence of an ID-less economic identity.
#[derive(Clone, Debug)]
pub struct Member {
    pub id: String,
    pub wallet_id: String,
    pub external_id: String,
    pub excluded: bool,
    pub workspace_id: String,
    pub profile_id: String,
    pub created_at: String,
    pub batch_id: Option<String>,
    pub import_sequence: Option<i64>,
    pub ordinal: i64,
    pub legacy: bool,
}

impl Member {
    /// `_import_order`: unbatched rows form one import before every batch.
    fn order(&self) -> (i64, Option<i64>) {
        if self.batch_id.is_none() {
            (0, Some(0))
        } else {
            (1, self.import_sequence)
        }
    }
}

/// Groups keyed by (wallet, economic hash), in first-seen order.
pub type Groups = Vec<((String, String), Vec<Member>)>;

/// `stored_synthetic_occurrences`: rows grouped by import order, then the old
/// numeric row index. Rows without a batch predate batch recording and sort
/// before every batched row by creation time; batches sort by their insertion
/// sequence. Excluded rows still consume an occurrence.
pub fn stored_synthetic_occurrences(
    conn: &Connection,
    profile_id: &str,
    wallet_id: Option<&str>,
) -> Result<Groups, AppError> {
    let mut statement = conn
        .prepare(
            "SELECT t.id, t.wallet_id, t.external_id, t.raw_json, t.excluded,
               t.workspace_id, t.profile_id, t.created_at,
               b.id AS batch_id, b.imported_at, b.import_sequence
        FROM transactions t
        LEFT JOIN import_batch_transactions bt ON bt.transaction_id = t.id
        LEFT JOIN import_batches b ON b.id = bt.batch_id
        WHERE t.profile_id = ?
          AND (? IS NULL OR t.wallet_id = ?)
          AND (t.external_id LIKE 'ledgerlive:%'
            OR t.external_id LIKE 'coinfinity:coinfinity:%'
            OR t.external_id LIKE 'pocketbitcoin:pocketbitcoin:%'
            OR t.external_id LIKE 'binance-supplemental:%'
            OR t.external_id LIKE 'binance:%')
        ORDER BY b.id IS NOT NULL, b.import_sequence, t.created_at, t.id",
        )
        .map_err(database_error)?;
    let rows = statement
        .query_map(params![profile_id, wallet_id, wallet_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<i64>>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, Option<String>>(8)?,
                r.get::<_, Option<i64>>(10)?,
            ))
        })
        .map_err(database_error)?;
    let mut groups: Groups = Vec::new();
    let mut index: HashMap<(String, String), usize> = HashMap::new();
    let mut seen = BTreeSet::new();
    for row in rows {
        let (id, wallet, external, raw, excluded, workspace, profile, created, batch, sequence) =
            row.map_err(database_error)?;
        if !seen.insert(id.clone()) {
            continue;
        }
        let external = external.unwrap_or_default();
        let Some((economic, ordinal, legacy)) =
            synthetic_csv_identity(&external, raw.as_deref().unwrap_or("null"))
        else {
            continue;
        };
        let key = (wallet.clone(), economic);
        let position = *index.entry(key.clone()).or_insert_with(|| {
            groups.push((key, Vec::new()));
            groups.len() - 1
        });
        groups[position].1.push(Member {
            id,
            wallet_id: wallet,
            external_id: external,
            excluded: excluded.unwrap_or(0) != 0,
            workspace_id: workspace,
            profile_id: profile,
            created_at: created.unwrap_or_default(),
            batch_id: batch,
            import_sequence: sequence,
            ordinal,
            legacy,
        });
    }
    for (_, members) in &mut groups {
        members.sort_by(|a, b| {
            (a.order(), &a.created_at, a.ordinal, &a.id).cmp(&(
                b.order(),
                &b.created_at,
                b.ordinal,
                &b.id,
            ))
        });
    }
    Ok(groups)
}

/// `largest_export_count`: the most occurrences any one import proves,
/// excluded rows included. Unbatched rows count together as one import unless
/// `split_unbatched` counts them per `created_at` second. A current occurrence
/// suffix proves that many rows appeared in its export.
pub fn largest_export_count(members: &[Member], split_unbatched: bool) -> i64 {
    let source = |row: &Member| -> (Option<String>, String) {
        match &row.batch_id {
            None if split_unbatched => (None, row.created_at.chars().take(19).collect()),
            batch => (batch.clone(), String::new()),
        }
    };
    let mut counts: BTreeMap<(Option<String>, String), i64> = BTreeMap::new();
    for row in members {
        *counts.entry(source(row)).or_default() += 1;
    }
    for row in members.iter().filter(|r| !r.legacy) {
        let count = counts.entry(source(row)).or_default();
        *count = (*count).max(row.ordinal);
    }
    counts.into_values().max().unwrap_or(0)
}

/// `_excluded_excess`: excluded rows beyond the largest single-export count,
/// latest import first. Such a row is a reviewed cross-batch copy, not an
/// occurrence of its own.
fn excluded_excess(members: &[Member]) -> BTreeSet<String> {
    let excess = members.len() as i64 - largest_export_count(members, false);
    members
        .iter()
        .rev()
        .filter(|r| r.excluded)
        .take(excess.max(0) as usize)
        .map(|r| r.id.clone())
        .collect()
}

/// `_assign_slots`: current IDs hold their own number; old positional rows
/// fill the lowest numbers no current row holds, in stored order.
fn assign_slots<'a>(
    members: &[&'a Member],
    skipped: &BTreeSet<String>,
) -> BTreeMap<i64, &'a Member> {
    let mut slots = BTreeMap::new();
    for row in members
        .iter()
        .filter(|r| !r.legacy && !skipped.contains(&r.id))
    {
        slots.entry(row.ordinal).or_insert(*row);
    }
    let mut free = 0;
    for row in members
        .iter()
        .filter(|r| r.legacy && !skipped.contains(&r.id))
    {
        free += 1;
        while slots.contains_key(&free) {
            free += 1;
        }
        slots.insert(free, *row);
    }
    slots
}

/// `occurrence_slots`: each occurrence number and the stored row holding it.
/// An excluded cross-batch excess copy holds none.
pub fn occurrence_slots(members: &[Member]) -> BTreeMap<i64, Member> {
    let refs: Vec<&Member> = members.iter().collect();
    assign_slots(&refs, &excluded_excess(members))
        .into_iter()
        .map(|(n, row)| (n, row.clone()))
        .collect()
}

/// `_historical_numbers`: the number each row held when its import was stored.
fn historical_numbers(members: &[Member]) -> HashMap<String, i64> {
    let orders: BTreeSet<_> = members.iter().map(Member::order).collect();
    let mut numbers = HashMap::new();
    for order in orders {
        let stored: Vec<&Member> = members.iter().filter(|r| r.order() <= order).collect();
        for (number, row) in assign_slots(&stored, &BTreeSet::new()) {
            numbers.entry(row.id.clone()).or_insert(number);
        }
    }
    numbers
}

/// `_refilled_copies`: whether an active current row took a number that,
/// among earlier imports, only an excluded copy held; and those copies.
fn refilled_copies(members: &[Member]) -> (bool, BTreeSet<String>) {
    let numbers = historical_numbers(members);
    let mut refilled = false;
    let mut copies = BTreeSet::new();
    for row in members.iter().filter(|r| !r.legacy && !r.excluded) {
        let prior: Vec<Member> = members
            .iter()
            .filter(|other| other.order() < row.order())
            .cloned()
            .collect();
        let freed = excluded_excess(&prior);
        let refs: Vec<&Member> = prior.iter().collect();
        if !assign_slots(&refs, &freed).contains_key(&row.ordinal)
            && prior
                .iter()
                .any(|other| numbers.get(&other.id) == Some(&row.ordinal))
        {
            refilled = true;
            copies.extend(freed);
        }
    }
    (refilled, copies)
}

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn dict(entries: Vec<(&str, Py)>) -> Py {
    Py::Dict(
        entries
            .into_iter()
            .map(|(k, v)| (Key::Str(k.into()), v))
            .collect(),
    )
}

fn strings<'a>(values: impl IntoIterator<Item = &'a String>) -> Py {
    Py::List(
        values
            .into_iter()
            .map(|v| Py::Str(v.as_str().into()))
            .collect(),
    )
}

/// `group_fingerprint`: binds a review to the exact members and exclusions.
pub fn group_fingerprint(wallet_id: &str, economic: &str, members: &[Member]) -> String {
    let mut pairs: Vec<(String, bool)> =
        members.iter().map(|r| (r.id.clone(), r.excluded)).collect();
    pairs.sort();
    let payload = dict(vec![
        ("wallet_id", Py::Str(wallet_id.into())),
        ("economic_hash", Py::Str(economic.into())),
        (
            "members",
            Py::List(
                pairs
                    .into_iter()
                    .map(|(id, excluded)| Py::List(vec![Py::Str(id.into()), Py::Bool(excluded)]))
                    .collect(),
            ),
        ),
    ]);
    let options = Options {
        sort_keys: true,
        separators: Some((",".into(), ":".into())),
        ..Default::default()
    };
    hex(kassiber_pyjson::dumps(&payload, &options)
        .expect("plain JSON")
        .as_bytes())
}

/// One held group, as `held_duplicate_groups` returns it.
#[derive(Clone, Debug)]
pub struct Group {
    pub wallet_id: String,
    pub economic_hash: String,
    pub group_fingerprint: String,
    pub active: Vec<Member>,
    pub largest: i64,
    pub import_count: usize,
    pub hold_reasons: Vec<&'static str>,
    pub refilled_excluded_ids: Vec<String>,
}

/// `held_duplicate_groups`: every held excess or refill no owner confirmed.
pub fn held_duplicate_groups(conn: &Connection, profile_id: &str) -> Result<Vec<Group>, AppError> {
    let mut confirmed = BTreeSet::new();
    {
        let mut statement = conn
            .prepare("SELECT wallet_id, economic_hash, group_fingerprint FROM synthetic_import_duplicate_reviews WHERE profile_id = ?")
            .map_err(database_error)?;
        let rows = statement
            .query_map([profile_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(database_error)?;
        for row in rows {
            confirmed.insert(row.map_err(database_error)?);
        }
    }
    let mut groups = Vec::new();
    for ((wallet_id, economic), members) in stored_synthetic_occurrences(conn, profile_id, None)? {
        let active: Vec<Member> = members.iter().filter(|r| !r.excluded).cloned().collect();
        // Unbatched rows count together as one more import beside batches.
        let batches: BTreeSet<Option<&str>> =
            active.iter().map(|r| r.batch_id.as_deref()).collect();
        let mut largest = largest_export_count(&members, false);
        let mut reasons = Vec::new();
        if batches.len() >= 2 && active.len() as i64 > largest {
            reasons.push(CROSS_IMPORT_EXCESS);
        }
        if batches.len() == 1 && batches.contains(&None) {
            // Nothing proves how many imports wrote rows that all predate
            // batches; only rows stored in one second count as one export.
            largest = largest_export_count(&members, true);
            if active.len() as i64 > largest {
                reasons.push(PRE_BATCH_EXCESS);
            }
        }
        let (refilled, copies) = refilled_copies(&members);
        if refilled {
            reasons.push(REFILLED_EXCLUDED_COPY);
        }
        if reasons.is_empty() {
            continue;
        }
        let fingerprint = group_fingerprint(&wallet_id, &economic, &members);
        if confirmed.contains(&(wallet_id.clone(), economic.clone(), fingerprint.clone())) {
            continue;
        }
        groups.push(Group {
            wallet_id,
            economic_hash: economic,
            group_fingerprint: fingerprint,
            import_count: batches.len(),
            active,
            largest,
            hold_reasons: reasons,
            refilled_excluded_ids: copies.into_iter().collect(),
        });
    }
    Ok(groups)
}

/// `duplicate_review_quarantines`: every active member of a held group, as
/// `(transaction_id, workspace_id, profile_id, reason, detail_json)` dicts.
pub fn duplicate_review_quarantines(
    conn: &Connection,
    profile_id: &str,
) -> Result<Vec<Py>, AppError> {
    let mut quarantines = Vec::new();
    for group in held_duplicate_groups(conn, profile_id)? {
        let resolution = std::iter::once(RESOLUTION)
            .chain(group.hold_reasons.iter().filter_map(|r| hold_note(r)))
            .collect::<Vec<_>>()
            .join(" ");
        let mut ids: Vec<String> = group.active.iter().map(|r| r.id.clone()).collect();
        ids.sort();
        let detail = dict(vec![
            (
                "economic_hash",
                Py::Str(group.economic_hash.as_str().into()),
            ),
            (
                "group_fingerprint",
                Py::Str(group.group_fingerprint.as_str().into()),
            ),
            ("transaction_ids", strings(&ids)),
            ("active_count", Py::Int(group.active.len().into())),
            ("largest_single_batch_count", Py::Int(group.largest.into())),
            ("import_batch_count", Py::Int(group.import_count.into())),
            (
                "hold_reasons",
                Py::List(
                    group
                        .hold_reasons
                        .iter()
                        .map(|r| Py::Str((*r).into()))
                        .collect(),
                ),
            ),
            (
                "refilled_excluded_transaction_ids",
                strings(&group.refilled_excluded_ids),
            ),
            ("resolution", Py::Str(resolution.into())),
        ]);
        let detail =
            kassiber_pyjson::dumps(&detail, &Options::new().sort_keys(true)).expect("plain JSON");
        for row in &group.active {
            quarantines.push(dict(vec![
                ("transaction_id", Py::Str(row.id.as_str().into())),
                ("workspace_id", Py::Str(row.workspace_id.as_str().into())),
                ("profile_id", Py::Str(row.profile_id.as_str().into())),
                ("reason", Py::Str(DUPLICATE_REVIEW.into())),
                ("detail_json", Py::Str(detail.as_str().into())),
            ]));
        }
    }
    Ok(quarantines)
}

/// The confirmation `confirm_distinct` recorded.
#[derive(Debug)]
pub struct Confirmation {
    pub review_id: String,
    pub group_fingerprint: String,
    pub transaction_ids: Vec<String>,
}

/// `confirm_distinct`: record the owner's review that a held group holds only
/// distinct trades, bound to the exact group. Nothing is excluded or
/// rewritten; journals are invalidated. `review_id` and `now` come from the
/// caller's id and clock sources.
#[allow(clippy::too_many_arguments)]
pub fn confirm_distinct(
    conn: &Connection,
    workspace_id: &str,
    profile_id: &str,
    transaction_id: &str,
    expected_fingerprint: &str,
    reason: &str,
    authored_source: &str,
    review_id: &str,
    now: &str,
) -> Result<Confirmation, AppError> {
    let group = held_duplicate_groups(conn, profile_id)?
        .into_iter()
        .find(|g| g.active.iter().any(|r| r.id == transaction_id));
    let Some(group) = group.filter(|g| g.group_fingerprint == expected_fingerprint) else {
        return Err(AppError::new(
            "review_case_changed",
            "Import duplicate review changed; inspect it again",
        ));
    };
    let mut ids: Vec<String> = group.active.iter().map(|r| r.id.clone()).collect();
    ids.sort();
    let ids_json = kassiber_pyjson::dumps(&strings(&ids), &Options::default()).expect("plain JSON");
    conn.execute(
        "INSERT INTO synthetic_import_duplicate_reviews(id, workspace_id, profile_id, wallet_id,
               economic_hash, group_fingerprint, transaction_ids_json, reason, authored_source, created_at)
           VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        params![
            review_id,
            workspace_id,
            profile_id,
            group.wallet_id,
            group.economic_hash,
            expected_fingerprint,
            ids_json,
            reason,
            authored_source,
            now
        ],
    )
    .map_err(database_error)?;
    conn.execute(
        "UPDATE profiles SET last_processed_at = NULL, last_processed_tx_count = 0,
               journal_input_version = journal_input_version + 1, ownership_review_counts_json = NULL
           WHERE id = ?",
        [profile_id],
    )
    .map_err(database_error)?;
    Ok(Confirmation {
        review_id: review_id.to_owned(),
        group_fingerprint: expected_fingerprint.to_owned(),
        transaction_ids: ids,
    })
}
