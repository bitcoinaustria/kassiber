//! Quarantine rolled up by root cause (`quarantine_review.quarantine_groups`
//! and the membership, delta and resolution-effect helpers it shares with
//! `ui.journals.quarantine.resolve`).
//!
//! Group ids name a cause, not a page position: `root:<id>` is one root
//! transaction plus every row that waits on it, `unrooted:<reason>` the
//! downstream rows whose root cannot be named, and `reason:<reason>` every
//! row stored with that primary reason. Reads only stored journal state.
use std::collections::{BTreeMap, HashMap};

use bitcoin_hashes::{sha256, Hash};
use kassiber_envelope::{dict, py_str, s, AppError};
use kassiber_pyjson::{BigInt, Key, Value as Py};
use kassiber_store::rusqlite::{types::Value as Sql, Connection};

use super::{parse_detail, reason, reason_info, waiting_roots, Held};
use crate::views::{int_or_zero, py, query};

const GROUP_ROOT: &str = "root:";
const GROUP_UNROOTED: &str = "unrooted:";
const GROUP_REASON: &str = "reason:";
pub const DEFAULT_GROUP_LIMIT: i64 = 25;
pub const MAX_GROUP_LIMIT: i64 = 100;
pub const DEFAULT_MEMBER_LIMIT: i64 = 50;
pub const MAX_MEMBER_LIMIT: i64 = 200;
const MAX_DELTA_IDS: usize = 50;

/// `GROUP_DEFINITIONS`: one sentence per counter the groups payload reports.
const GROUP_DEFINITIONS: [(&str, &str); 6] = [
    ("count", "Every row in journal_quarantines for the active book: one per held transaction."),
    ("by_reason", "Rows per stored primary reason; each row counts once."),
    ("by_root", "Rows per root cause: a root transaction plus every downstream row that waits on it and clears when it is resolved."),
    ("dependent_count", "Rows in a root group other than the root itself."),
    ("unrooted", "Downstream rows whose root cannot be named; they need the user like a root does."),
    ("amounts", "Per asset, the stored transaction amounts of the group's rows in msat, split by direction; fee_msat is the rows' own recorded fee."),
];

/// One held row and the one group it belongs to.
#[derive(Clone)]
pub struct Member {
    transaction_id: String,
    reason: String,
    group_id: String,
    is_root: bool,
    occurred_at: Py,
    asset: Py,
    direction: Py,
    amount_msat: BigInt,
    fee_msat: BigInt,
    wallet: Py,
    external_id: Py,
}

impl Member {
    fn payload(&self, with_group: bool) -> Py {
        let mut fields = vec![
            ("transaction_id", s(self.transaction_id.as_str())),
            ("reason", s(self.reason.as_str())),
        ];
        if with_group {
            fields.push(("group_id", s(self.group_id.as_str())));
        }
        fields.extend([
            ("is_root", Py::Bool(self.is_root)),
            ("occurred_at", self.occurred_at.clone()),
            ("asset", self.asset.clone()),
            ("direction", self.direction.clone()),
            ("amount_msat", Py::Int(self.amount_msat.clone())),
            ("fee_msat", Py::Int(self.fee_msat.clone())),
            ("wallet", self.wallet.clone()),
            ("external_id", self.external_id.clone()),
        ]);
        dict(fields)
    }

    fn root_payload(&self) -> Py {
        dict([
            ("transaction_id", s(self.transaction_id.as_str())),
            ("reason", s(self.reason.as_str())),
            ("occurred_at", self.occurred_at.clone()),
            ("asset", self.asset.clone()),
            ("direction", self.direction.clone()),
            ("amount_msat", Py::Int(self.amount_msat.clone())),
            ("fee_msat", Py::Int(self.fee_msat.clone())),
            ("wallet", self.wallet.clone()),
            ("external_id", self.external_id.clone()),
        ])
    }

    /// `_member_sort_key`: the root first, then by time and id.
    fn sort_key(&self) -> (i64, String, String) {
        (
            if self.is_root { 0 } else { 1 },
            text_or_empty(&self.occurred_at),
            self.transaction_id.clone(),
        )
    }
}

/// `quarantine_membership`: members in stored row order (a later duplicate
/// row replaces an earlier one in place) and the root each waiting row names.
pub struct Membership {
    members: Vec<Member>,
    waiting: HashMap<String, String>,
}

impl Membership {
    fn member(&self, id: &str) -> Option<&Member> {
        self.members.iter().find(|m| m.transaction_id == id)
    }
}

fn text_or_empty(value: &Py) -> String {
    if crate::views::py_truthy(value) {
        py_str(value)
    } else {
        String::new()
    }
}

pub fn membership(conn: &Connection, profile_id: &str) -> Result<Membership, AppError> {
    let rows = query(
        conn,
        "SELECT q.transaction_id, q.reason, q.detail_json,
               t.occurred_at, t.asset, t.direction, t.amount, t.fee,
               t.external_id, w.label AS wallet
        FROM journal_quarantines q
        JOIN transactions t ON t.id = q.transaction_id
        JOIN wallets w ON w.id = t.wallet_id
        WHERE q.profile_id = ?",
        &[Sql::Text(profile_id.to_owned())],
    )?;
    let mut order: Vec<String> = Vec::new();
    let mut by_id: HashMap<String, &crate::views::Row> = HashMap::new();
    let mut details: HashMap<String, Py> = HashMap::new();
    for row in &rows {
        let id = py_str(&py(row.get("transaction_id")));
        if !by_id.contains_key(&id) {
            order.push(id.clone());
        }
        by_id.insert(id.clone(), row);
        details.insert(id, parse_detail(row.get("detail_json")));
    }
    let held: Vec<(String, Held)> = order
        .iter()
        .map(|id| {
            let row = by_id[id];
            (
                id.clone(),
                Held {
                    reason: py_str(&py(row.get("reason"))),
                    occurred_at: py(row.get("occurred_at")),
                    asset: py(row.get("asset")),
                },
            )
        })
        .collect();
    let waiting = waiting_roots(&held, &details);
    let mut members = Vec::with_capacity(order.len());
    for id in &order {
        let row = by_id[id];
        let reason_name = py_str(&py(row.get("reason")));
        let downstream = reason_info(&reason_name, &details[id]).downstream();
        let group_id = if let Some(root) = waiting.get(id) {
            format!("{GROUP_ROOT}{root}")
        } else if !downstream {
            format!("{GROUP_ROOT}{id}")
        } else {
            format!("{GROUP_UNROOTED}{reason_name}")
        };
        let external = py(row.get("external_id"));
        members.push(Member {
            transaction_id: id.clone(),
            is_root: group_id == format!("{GROUP_ROOT}{id}"),
            reason: reason_name,
            group_id,
            occurred_at: py(row.get("occurred_at")),
            asset: py(row.get("asset")),
            direction: py(row.get("direction")),
            amount_msat: int_or_zero(row.get("amount"))?,
            fee_msat: int_or_zero(row.get("fee"))?,
            wallet: py(row.get("wallet")),
            external_id: if crate::views::py_truthy(&external) {
                external
            } else {
                s("")
            },
        });
    }
    Ok(Membership { members, waiting })
}

/// `_membership_digest`: the first 16 hex digits of the sorted triples' JSON.
fn digest(members: &[Member]) -> Result<String, AppError> {
    let mut triples: Vec<(&str, &str, &str)> = members
        .iter()
        .map(|m| {
            (
                m.transaction_id.as_str(),
                m.reason.as_str(),
                m.group_id.as_str(),
            )
        })
        .collect();
    triples.sort();
    let encoded = compact_ascii(&Py::List(
        triples
            .into_iter()
            .map(|(a, b, c)| Py::List(vec![s(a), s(b), s(c)]))
            .collect(),
    ))?;
    Ok(sha256::Hash::hash(encoded.as_bytes()).to_string()[..16].to_owned())
}

fn compact_ascii(value: &Py) -> Result<String, AppError> {
    kassiber_pyjson::dumps(value, &kassiber_pyjson::Options::new().compact())
        .map_err(|error| AppError::new("internal_error", error.to_string()))
}

/// `_amounts`: per asset, sorted by asset.
fn amounts(members: &[&Member]) -> Py {
    let mut by_asset: BTreeMap<String, [BigInt; 4]> = BTreeMap::new();
    let zero = || BigInt::from(0);
    for member in members {
        let bucket = by_asset
            .entry(text_or_empty(&member.asset))
            .or_insert_with(|| [zero(), zero(), zero(), zero()]);
        bucket[0] = &bucket[0] + &BigInt::from(1);
        if kassiber_pytext::lower(&text_or_empty(&member.direction)) == "inbound" {
            bucket[1] = &bucket[1] + &member.amount_msat;
        } else {
            bucket[2] = &bucket[2] + &member.amount_msat;
        }
        bucket[3] = &bucket[3] + &member.fee_msat;
    }
    Py::List(
        by_asset
            .into_iter()
            .map(|(asset, [count, inbound, outbound, fee])| {
                dict([
                    ("asset", s(asset)),
                    ("count", Py::Int(count)),
                    ("inbound_msat", Py::Int(inbound)),
                    ("outbound_msat", Py::Int(outbound)),
                    ("fee_msat", Py::Int(fee)),
                ])
            })
            .collect(),
    )
}

/// `_group_summary`.
fn group_summary(group_id: &str, members: &[&Member], all: &Membership) -> Py {
    let mut reasons: Vec<(String, i64)> = Vec::new();
    for member in members {
        match reasons.iter_mut().find(|(r, _)| *r == member.reason) {
            Some((_, count)) => *count += 1,
            None => reasons.push((member.reason.clone(), 1)),
        }
    }
    reasons.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let mut fields = vec![
        ("group_id", s(group_id)),
        ("count", Py::from(members.len() as i64)),
        (
            "reasons",
            Py::List(
                reasons
                    .into_iter()
                    .map(|(reason, count)| {
                        dict([("reason", s(reason)), ("count", Py::from(count))])
                    })
                    .collect(),
            ),
        ),
        ("amounts", amounts(members)),
    ];
    if let Some(root_id) = group_id.strip_prefix(GROUP_ROOT) {
        let root = all.member(root_id);
        // A root whose own reason is downstream sits in another group; its
        // waiting rows still name it, so count only rows that are really here.
        let in_group = root.is_some_and(|root| root.group_id == group_id);
        fields.push((
            "dependent_count",
            Py::from(members.len() as i64 - i64::from(in_group)),
        ));
        fields.push(("root", root.map_or(Py::Null, Member::root_payload)));
    }
    dict(fields)
}

fn remediation(action: &str, target: Py) -> Py {
    dict([
        ("action", s(action)),
        ("tool", s("ui.journals.quarantine.groups")),
        ("target", target),
        ("desktop_only", Py::Bool(false)),
    ])
}

fn cursor_error(message: &str, code: &str, group_id: &str) -> AppError {
    let target = if group_id.is_empty() {
        dict::<&str>([])
    } else {
        dict([("group_id", s(group_id))])
    };
    AppError::new(code, message)
        .with_details(dict([("remediation", remediation("reread_group", target))]))
}

/// The scope a profile row reads for the groups payload.
struct Profile<'a> {
    workspace_id: &'a str,
    profile_id: &'a str,
    /// `custody_journal.projection_freshness(conn, profile)`.
    freshness: &'a Py,
}

/// `build_journals_quarantine_groups_snapshot(conn, args)`: the assistant's
/// read of the active book's groups (no desktop kind serves it), with the
/// argument checks in Python's order.
pub fn snapshot(conn: &Connection, args: Option<&Py>) -> Result<Py, AppError> {
    const KIND: &str = "ui.journals.quarantine.groups";
    let args = crate::views::args_object(args)?;
    crate::views::unknown(KIND, &args, &["group_id", "cursor", "limit"])?;
    let field = |name: &str| {
        args.iter()
            .find(|(key, _)| *key == Key::from(name))
            .map(|(_, value)| value)
    };
    let mut stripped = Vec::new();
    for name in ["group_id", "cursor"] {
        stripped.push(match field(name) {
            None | Some(Py::Null) => None,
            Some(Py::Str(text))
                if kassiber_pytext::strip_py(text)
                    .code_points()
                    .next()
                    .is_some() =>
            {
                Some(kassiber_pytext::strip_py(text))
            }
            Some(_) => {
                return Err(AppError::new(
                    "validation",
                    format!("{KIND} {name} must be a non-empty string"),
                ))
            }
        });
    }
    let cursor = stripped.pop().flatten();
    let group_id = stripped.pop().flatten();
    let limit = match field("limit") {
        None => None,
        Some(_) if group_id.is_some() => Some(crate::views::limit(
            &args,
            DEFAULT_MEMBER_LIMIT,
            MAX_MEMBER_LIMIT,
        )?),
        Some(_) => Some(crate::views::limit(
            &args,
            DEFAULT_GROUP_LIMIT,
            MAX_GROUP_LIMIT,
        )?),
    };
    let scope = crate::views::active(conn)?;
    let Some(profile) = &scope.profile else {
        return Err(AppError::new("no_active_profile", "No active book is open"));
    };
    let profile_id = py_str(&py(profile.get("id")));
    let group_id = group_id.map(|id| py_str(&Py::Str(id)));
    quarantine_groups(
        conn,
        &profile_id,
        group_id.as_deref(),
        cursor.as_ref(),
        limit,
    )
}

/// `quarantine_groups(conn, profile, ...)` for the stored profile row.
pub fn quarantine_groups(
    conn: &Connection,
    profile_id: &str,
    group_id: Option<&str>,
    cursor: Option<&kassiber_pyjson::PyStr>,
    limit: Option<i64>,
) -> Result<Py, AppError> {
    let id = Sql::Text(profile_id.to_owned());
    let row = query(
        conn,
        "SELECT * FROM profiles WHERE id = ?",
        std::slice::from_ref(&id),
    )?
    .into_iter()
    .next()
    .ok_or_else(|| AppError::new("internal_error", "profile row disappeared"))?;
    let active: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transactions WHERE profile_id = ? AND excluded = 0",
            [&id],
            |r| r.get(0),
        )
        .map_err(crate::views::sql_error)?;
    let freshness = kassiber_workspace::readiness::evaluate_freshness(&row.to_py(), active)?;
    let workspace_id = py_str(&py(row.get("workspace_id")));
    groups(
        conn,
        &Profile {
            workspace_id: &workspace_id,
            profile_id,
            freshness: &freshness,
        },
        group_id,
        cursor,
        limit,
    )
}

/// `quarantine_groups`: whole-book counts and msat totals by reason and root
/// cause, or one group's members paged by a journal-state-bound cursor.
fn groups(
    conn: &Connection,
    profile: &Profile<'_>,
    group_id: Option<&str>,
    cursor: Option<&kassiber_pyjson::PyStr>,
    limit: Option<i64>,
) -> Result<Py, AppError> {
    let membership = membership(conn, profile.profile_id)?;
    let members = &membership.members;
    let fresh = |key: &str| {
        kassiber_envelope::get(profile.freshness, key)
            .cloned()
            .unwrap_or(Py::Null)
    };
    let input_version = match fresh("journal_input_version") {
        Py::Int(n) => n,
        _ => BigInt::from(0),
    };
    let last_processed_at = fresh("last_processed_at");
    let digest = digest(members)?;
    let mut base = vec![
        ("workspace_id", s(profile.workspace_id)),
        ("profile_id", s(profile.profile_id)),
        ("input_version", Py::Int(input_version.clone())),
        ("last_processed_at", last_processed_at.clone()),
        (
            "needs_processing",
            Py::Bool(crate::views::py_truthy(&fresh("needs_processing"))),
        ),
        ("membership_digest", s(digest.as_str())),
        ("count", Py::from(members.len() as i64)),
        (
            "definitions",
            dict(GROUP_DEFINITIONS.iter().map(|(k, v)| (*k, s(*v)))),
        ),
    ];
    let mut by_group: BTreeMap<&str, Vec<&Member>> = BTreeMap::new();
    for member in members {
        by_group.entry(&member.group_id).or_default().push(member);
    }

    let Some(group_id) = group_id else {
        if cursor.is_some() {
            return Err(AppError::new(
                "validation",
                "A quarantine group cursor needs its group_id",
            ));
        }
        let bounded = limit.unwrap_or(DEFAULT_GROUP_LIMIT);
        let mut by_reason: Vec<(&str, Vec<&Member>)> = Vec::new();
        for member in members {
            match by_reason.iter_mut().find(|(r, _)| *r == member.reason) {
                Some((_, rows)) => rows.push(member),
                None => by_reason.push((&member.reason, vec![member])),
            }
        }
        by_reason.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(b.0)));
        let reason_groups = by_reason
            .iter()
            .map(|(reason_name, rows)| {
                dict([
                    ("group_id", s(format!("{GROUP_REASON}{reason_name}"))),
                    ("reason", s(*reason_name)),
                    ("category", s(reason(reason_name).category)),
                    ("count", Py::from(rows.len() as i64)),
                    ("amounts", amounts(rows)),
                ])
            })
            .collect();
        let mut root_ids: Vec<&str> = by_group
            .keys()
            .copied()
            .filter(|key| key.starts_with(GROUP_ROOT))
            .collect();
        root_ids.sort_by(|a, b| {
            by_group[b]
                .len()
                .cmp(&by_group[a].len())
                .then_with(|| a.cmp(b))
        });
        let unrooted: Vec<&str> = by_group
            .keys()
            .copied()
            .filter(|key| key.starts_with(GROUP_UNROOTED))
            .collect();
        let shown = python_slice_end(root_ids.len(), bounded);
        base.extend([
            ("waiting_count", Py::from(membership.waiting.len() as i64)),
            ("by_reason", Py::List(reason_groups)),
            (
                "by_root",
                Py::List(
                    root_ids[..shown]
                        .iter()
                        .map(|key| group_summary(key, &by_group[key], &membership))
                        .collect(),
                ),
            ),
            ("root_group_count", Py::from(root_ids.len() as i64)),
            (
                "root_groups_truncated",
                Py::Bool(root_ids.len() as i64 > bounded),
            ),
            (
                "unrooted",
                Py::List(
                    unrooted
                        .iter()
                        .map(|key| {
                            let mut summary = group_summary(key, &by_group[key], &membership);
                            kassiber_envelope::set(
                                &mut summary,
                                "reason",
                                s(&key[GROUP_UNROOTED.len()..]),
                            );
                            summary
                        })
                        .collect(),
                ),
            ),
        ]);
        return Ok(dict(base));
    };

    let group_members: Vec<&Member> = if let Some(reason_name) = group_id.strip_prefix(GROUP_REASON)
    {
        members.iter().filter(|m| m.reason == reason_name).collect()
    } else if group_id.starts_with(GROUP_ROOT) || group_id.starts_with(GROUP_UNROOTED) {
        by_group.get(group_id).cloned().unwrap_or_default()
    } else {
        return Err(AppError::new(
            "validation",
            "group_id must start with root:, unrooted: or reason:",
        ));
    };
    if group_members.is_empty() {
        return Err(
            AppError::new("not_found", "No quarantined rows are in this group now").with_details(
                dict([
                    ("group_id", s(group_id)),
                    (
                        "remediation",
                        remediation("reread_groups", dict::<&str>([])),
                    ),
                ]),
            ),
        );
    }
    let scope = Py::List(vec![
        s(profile.profile_id),
        Py::Int(input_version),
        last_processed_at,
        s(group_id),
        s(digest.as_str()),
    ]);
    let after = match cursor {
        None => None,
        Some(cursor) => {
            let (stored_scope, after) = decode_cursor(cursor).ok_or_else(|| {
                cursor_error(
                    "Quarantine group cursor is malformed",
                    "validation",
                    group_id,
                )
            })?;
            if !python_equal(&stored_scope, &scope) {
                return Err(cursor_error(
                    "The quarantine changed since this cursor was issued; read the group again",
                    "quarantine_cursor_stale",
                    group_id,
                ));
            }
            Some(after)
        }
    };
    let page_limit = limit.unwrap_or(DEFAULT_MEMBER_LIMIT);
    let mut ordered = group_members.clone();
    ordered.sort_by_key(|m| m.sort_key());
    if let Some((rank, at, id)) = &after {
        ordered.retain(|m| {
            let (r, a, i) = m.sort_key();
            (BigInt::from(r), a, i) > (rank.clone(), at.clone(), id.clone())
        });
    }
    let page = &ordered[..python_slice_end(ordered.len(), page_limit)];
    let next_cursor = if ordered.len() as i64 > page_limit {
        let last = page
            .last()
            .ok_or_else(|| AppError::new("internal_error", "list index out of range"))?;
        let (rank, at, id) = last.sort_key();
        let payload = compact_ascii(&dict([
            ("scope", scope),
            ("after", Py::List(vec![Py::from(rank), s(at), s(id)])),
        ]))?;
        s(urlsafe_b64encode(payload.as_bytes()))
    } else {
        Py::Null
    };
    base.extend([
        (
            "group",
            group_summary(group_id, &group_members, &membership),
        ),
        (
            "members",
            Py::List(page.iter().map(|m| m.payload(false)).collect()),
        ),
        ("next_cursor", next_cursor),
    ]);
    Ok(dict(base))
}

/// `sequence[:limit]` for a list of `len` items.
fn python_slice_end(len: usize, limit: i64) -> usize {
    if limit >= 0 {
        (limit as usize).min(len)
    } else {
        len.saturating_sub(limit.unsigned_abs() as usize)
    }
}

/// Python `==` over decoded JSON: numbers compare by value across int,
/// float and bool; containers compare element by element.
fn python_equal(left: &Py, right: &Py) -> bool {
    fn number(value: &Py) -> Option<f64> {
        match value {
            Py::Bool(b) => Some(f64::from(u8::from(*b))),
            Py::Int(n) => n.to_string().parse().ok(),
            Py::Float(f) => Some(*f),
            _ => None,
        }
    }
    match (left, right) {
        (Py::Int(a), Py::Int(b)) => a == b,
        (Py::Int(_) | Py::Bool(_) | Py::Float(_), Py::Int(_) | Py::Bool(_) | Py::Float(_)) => {
            match (left, right) {
                (Py::Bool(a), Py::Bool(b)) => a == b,
                (Py::Int(n), other) | (other, Py::Int(n)) => {
                    // An integer equals a float only when the float is that
                    // exact integer.
                    let f = number(other).unwrap();
                    f.fract() == 0.0 && f.is_finite() && {
                        let text = format!("{f:.0}");
                        text.parse::<BigInt>().is_ok_and(|v| &v == n)
                    }
                }
                _ => number(left) == number(right),
            }
        }
        (Py::Null, Py::Null) => true,
        (Py::Str(a), Py::Str(b)) => a == b,
        (Py::List(a), Py::List(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| python_equal(x, y))
        }
        _ => false,
    }
}

/// The cursor's scope and `after` key: `json.loads` of the URL-safe base64
/// text, a mapping with `scope` and a three-item `after` of an int and two
/// strings. `None` for anything Python rejects as malformed.
fn decode_cursor(cursor: &kassiber_pyjson::PyStr) -> Option<(Py, (BigInt, String, String))> {
    let text = cursor.as_str()?;
    let bytes = urlsafe_b64decode(text.as_bytes())?;
    let decoded = String::from_utf8(bytes).ok()?;
    let payload = kassiber_pytext::loads(&decoded).ok()?;
    let Py::Dict(pairs) = &payload else {
        return None;
    };
    let lookup = |name: &str| {
        pairs.iter().rev().find_map(|(key, value)| match key {
            Key::Str(k) if k.as_str() == Some(name) => Some(value),
            _ => None,
        })
    };
    let scope = lookup("scope")?.clone();
    let Py::List(after) = lookup("after")? else {
        return None;
    };
    match after.as_slice() {
        [Py::Int(rank), Py::Str(at), Py::Str(id)] => Some((
            scope,
            (
                rank.clone(),
                py_str(&Py::Str(at.clone())),
                py_str(&Py::Str(id.clone())),
            ),
        )),
        _ => None,
    }
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// `base64.urlsafe_b64encode`, with padding.
fn urlsafe_b64encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for index in 0..4 {
            if index <= chunk.len() {
                out.push(ALPHABET[((n >> (18 - 6 * index)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// `base64.urlsafe_b64decode`: CPython's non-strict `a2b_base64` after
/// mapping `-_` to `+/`. Characters outside the alphabet are skipped; a
/// complete pad sequence ends the input; a partial final quad fails.
fn urlsafe_b64decode(input: &[u8]) -> Option<Vec<u8>> {
    let value = |byte: u8| -> Option<u32> {
        Some(match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        } as u32)
    };
    let mut out = Vec::new();
    let (mut quad, mut left, mut pads) = (0u8, 0u32, 0u8);
    for &byte in input {
        if byte == b'=' {
            if quad >= 2 {
                pads += 1;
                if quad + pads >= 4 {
                    return Some(out);
                }
            }
            continue;
        }
        let Some(bits) = value(byte) else {
            continue;
        };
        pads = 0;
        match quad {
            0 => {
                quad = 1;
                left = bits;
            }
            1 => {
                quad = 2;
                out.push(((left << 2) | (bits >> 4)) as u8);
                left = bits & 0x0f;
            }
            2 => {
                quad = 3;
                out.push(((left << 4) | (bits >> 2)) as u8);
                left = bits & 0x03;
            }
            _ => {
                quad = 0;
                out.push(((left << 6) | bits) as u8);
                left = 0;
            }
        }
    }
    (quad == 0).then_some(out)
}

/// `root_transaction_for_group`: the root a current `root:` group acts on.
pub fn root_transaction_for_group(
    conn: &Connection,
    profile_id: &str,
    group_id: &str,
) -> Result<String, AppError> {
    let Some(transaction_id) = group_id.strip_prefix(GROUP_ROOT) else {
        return Err(AppError::new(
            "validation",
            "Only a root: group can be resolved; it acts on its root transaction",
        )
        .with_details(dict([("group_id", s(group_id))])));
    };
    let current = membership(conn, profile_id)?;
    if !current.member(transaction_id).is_some_and(|m| m.is_root) {
        return Err(AppError::new(
            "quarantine_group_stale",
            "This root group is not current; read the quarantine groups again",
        )
        .with_details(dict([
            ("group_id", s(group_id)),
            (
                "remediation",
                remediation("reread_groups", dict::<&str>([])),
            ),
        ])));
    }
    Ok(transaction_id.to_owned())
}

/// `membership_delta`: what a resolution and its reprocess changed.
fn membership_delta(
    before: &Membership,
    after: &Membership,
    target_id: &str,
) -> Vec<(&'static str, Py)> {
    let held_after = |id: &str| after.member(id).is_some();
    let mut dependents: Vec<&str> = before
        .waiting
        .iter()
        .filter(|(_, root)| root.as_str() == target_id)
        .map(|(id, _)| id.as_str())
        .collect();
    dependents.sort();
    let cleared: Vec<&str> = dependents
        .iter()
        .copied()
        .filter(|id| !held_after(id))
        .collect();
    let still: Vec<&str> = dependents
        .iter()
        .copied()
        .filter(|id| held_after(id))
        .collect();
    let mut other: Vec<&str> = before
        .members
        .iter()
        .map(|m| m.transaction_id.as_str())
        .filter(|id| !held_after(id) && *id != target_id && !dependents.contains(id))
        .collect();
    other.sort();
    let mut new: Vec<&str> = after
        .members
        .iter()
        .map(|m| m.transaction_id.as_str())
        .filter(|id| before.member(id).is_none())
        .collect();
    new.sort();
    let bounded = |ids: &[&str]| {
        dict([
            ("count", Py::from(ids.len() as i64)),
            (
                "transaction_ids",
                Py::List(ids.iter().take(MAX_DELTA_IDS).map(|id| s(*id)).collect()),
            ),
            ("truncated", Py::Bool(ids.len() > MAX_DELTA_IDS)),
        ])
    };
    vec![
        ("dependents_before", Py::from(dependents.len() as i64)),
        ("dependents_cleared", bounded(&cleared)),
        ("dependents_still_quarantined", bounded(&still)),
        ("other_rows_cleared", bounded(&other)),
        ("newly_quarantined", bounded(&new)),
        ("count_before", Py::from(before.members.len() as i64)),
        ("count_after", Py::from(after.members.len() as i64)),
    ]
}

/// `resolution_effect`: which rows that waited on a resolved root cleared
/// with it, measured only after a reprocess.
pub fn resolution_effect(
    conn: &Connection,
    profile_id: &str,
    transaction_id: &str,
    before: &Membership,
    reprocessed: bool,
) -> Result<Vec<(&'static str, Py)>, AppError> {
    if !reprocessed {
        let waiting = before
            .waiting
            .values()
            .filter(|root| root.as_str() == transaction_id)
            .count();
        return Ok(vec![
            ("dependents_cleared", Py::Null),
            (
                "quarantine_delta",
                dict([
                    ("status", s("not_measured")),
                    ("reason", s("journals_not_reprocessed")),
                    ("dependents_before", Py::from(waiting as i64)),
                    (
                        "remediation",
                        dict([
                            ("action", s("process_journals")),
                            ("tool", s("ui.journals.process")),
                            ("target", dict::<&str>([])),
                            ("desktop_only", Py::Bool(false)),
                        ]),
                    ),
                ]),
            ),
        ]);
    }
    let after = membership(conn, profile_id)?;
    let delta = membership_delta(before, &after, transaction_id);
    let cleared = delta
        .iter()
        .find(|(key, _)| *key == "dependents_cleared")
        .map(|(_, value)| value.clone())
        .unwrap_or(Py::Null);
    let mut measured = vec![("status", s("measured"))];
    measured.extend(delta);
    Ok(vec![
        ("dependents_cleared", cleared),
        ("quarantine_delta", dict(measured)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `gen_quarantine_group_vectors.py`: Python's `json.dumps` of each output.
    fn expected(key: &str) -> String {
        let vectors: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/data/quarantine_groups.json")).unwrap();
        vectors[key].as_str().unwrap().to_owned()
    }

    fn fixture() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("groups_fixture.sql"))
            .unwrap();
        conn
    }

    fn text(value: &Py) -> String {
        kassiber_pyjson::dumps(value, &kassiber_pyjson::Options::new()).unwrap()
    }

    fn read(conn: &Connection, group: Option<&str>, cursor: Option<&Py>, limit: Option<i64>) -> Py {
        let freshness = dict([
            ("journal_input_version", Py::from(3)),
            ("last_processed_at", s("2024-05-01T00:00:00Z")),
            ("needs_processing", Py::Bool(false)),
        ]);
        let profile = Profile {
            workspace_id: "w",
            profile_id: "p",
            freshness: &freshness,
        };
        let cursor = match cursor {
            Some(Py::Str(text)) => Some(text.clone()),
            _ => None,
        };
        groups(conn, &profile, group, cursor.as_ref(), limit).unwrap()
    }

    #[test]
    fn groups_roll_up_by_reason_and_root_like_python() {
        let conn = fixture();
        assert_eq!(text(&read(&conn, None, None, None)), expected("overview"));
        assert_eq!(
            text(&read(&conn, None, None, Some(1))),
            expected("overview_limit_1")
        );
        assert_eq!(
            text(&read(
                &conn,
                Some("reason:custody_basis_barrier"),
                None,
                None
            )),
            expected("reason_group")
        );
        assert_eq!(
            text(&read(
                &conn,
                Some("unrooted:custody_basis_barrier"),
                None,
                None
            )),
            expected("unrooted_group")
        );
    }

    #[test]
    fn group_members_page_by_python_cursors() {
        let conn = fixture();
        let mut cursor = Py::Null;
        let mut actual = Vec::new();
        loop {
            let page = read(&conn, Some("root:a"), Some(&cursor), Some(1));
            cursor = kassiber_envelope::get(&page, "next_cursor")
                .cloned()
                .unwrap();
            actual.push(page);
            // Three members, one per page: a repeated page must not loop.
            if cursor == Py::Null || actual.len() > 3 {
                break;
            }
        }
        assert_eq!(text(&Py::List(actual)), expected("root_pages"));
    }

    #[test]
    fn resolution_effect_reports_cleared_waiting_and_new_rows_like_python() {
        let conn = fixture();
        let before = membership(&conn, "p").unwrap();
        conn.execute_batch(
            "DELETE FROM journal_quarantines WHERE transaction_id IN ('a','b');
             INSERT INTO journal_quarantines VALUES('n','p','missing_spot_price','{}');",
        )
        .unwrap();
        let effect =
            |reprocessed| dict(resolution_effect(&conn, "p", "a", &before, reprocessed).unwrap());
        assert_eq!(text(&effect(true)), expected("effect"));
        assert_eq!(text(&effect(false)), expected("unmeasured"));
    }

    #[test]
    fn only_a_current_root_group_resolves_and_cursors_bind_to_the_quarantine() {
        let conn = fixture();
        assert_eq!(
            root_transaction_for_group(&conn, "p", "root:a").unwrap(),
            "a"
        );
        for (group, code) in [
            ("reason:missing_spot_price", "validation"),
            ("root:b", "quarantine_group_stale"),
            ("root:zzz", "quarantine_group_stale"),
        ] {
            assert_eq!(
                root_transaction_for_group(&conn, "p", group)
                    .unwrap_err()
                    .code,
                code,
                "{group}"
            );
        }
        let first = read(&conn, Some("root:a"), None, Some(1));
        let cursor = kassiber_envelope::get(&first, "next_cursor")
            .cloned()
            .unwrap();
        conn.execute(
            "INSERT INTO journal_quarantines VALUES('n','p','missing_spot_price','{}')",
            [],
        )
        .unwrap();
        let freshness = dict([
            ("journal_input_version", Py::from(3)),
            ("last_processed_at", s("2024-05-01T00:00:00Z")),
        ]);
        let profile = Profile {
            workspace_id: "w",
            profile_id: "p",
            freshness: &freshness,
        };
        let Py::Str(cursor) = cursor else { panic!() };
        let error = groups(&conn, &profile, Some("root:a"), Some(&cursor), None).unwrap_err();
        assert_eq!(error.code, "quarantine_cursor_stale");
        for (group, cursor, code) in [
            (Some("bogus"), None, "validation"),
            (Some("root:t"), Some("e30="), "validation"),
            (Some("root:none"), None, "not_found"),
            (None, Some("e30="), "validation"),
        ] {
            let cursor = cursor.map(kassiber_pyjson::PyStr::from);
            assert_eq!(
                groups(&conn, &profile, group, cursor.as_ref(), None)
                    .unwrap_err()
                    .code,
                code,
                "{group:?}"
            );
        }
    }

    #[test]
    fn cursor_base64_follows_cpython_non_strict_decoding() {
        for (text, expected) in [
            ("eyJhIjoxfQ==", Some(&b"{\"a\":1}"[..])),
            ("eyJhIjoxfQ", None),
            ("ey Jh\nIjoxfQ==", Some(&b"{\"a\":1}"[..])),
            ("eyJhIjoxfQ==trailing", Some(&b"{\"a\":1}"[..])),
            ("YQ=", None),
            ("YQ==", Some(&b"a"[..])),
            ("Y", None),
            ("-_-_", Some(&[0xfb, 0xff, 0xbf][..])),
            ("+/+/", Some(&[0xfb, 0xff, 0xbf][..])),
            ("", Some(&b""[..])),
        ] {
            assert_eq!(
                urlsafe_b64decode(text.as_bytes()).as_deref(),
                expected,
                "{text}"
            );
        }
        for bytes in [&b""[..], b"a", b"ab", b"abc", b"\xfb\xff\xbf"] {
            assert_eq!(
                urlsafe_b64decode(urlsafe_b64encode(bytes).as_bytes()).as_deref(),
                Some(bytes)
            );
        }
        assert_eq!(urlsafe_b64encode(b"\xfb\xff\xbf"), "-_-_");
        assert_eq!(urlsafe_b64encode(b"ab"), "YWI=");
    }

    #[test]
    fn cursor_scope_compares_with_python_equality() {
        let one = Py::from(1);
        assert!(python_equal(&one, &Py::Float(1.0)));
        assert!(python_equal(&one, &Py::Bool(true)));
        assert!(!python_equal(&one, &Py::Float(1.5)));
        assert!(!python_equal(&one, &s("1")));
        assert!(python_equal(
            &Py::List(vec![s("p"), Py::Null]),
            &Py::List(vec![s("p"), Py::Null])
        ));
        assert!(!python_equal(
            &Py::List(vec![s("p")]),
            &Py::List(vec![s("p"), Py::Null])
        ));
    }

    #[test]
    fn slices_follow_python_bounds() {
        assert_eq!(python_slice_end(5, 3), 3);
        assert_eq!(python_slice_end(5, 9), 5);
        assert_eq!(python_slice_end(5, 0), 0);
        assert_eq!(python_slice_end(5, -2), 3);
        assert_eq!(python_slice_end(1, -2), 0);
    }
}
