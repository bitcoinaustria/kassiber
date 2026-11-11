//! `custody_authored_migration.py`: legacy reviewed pairs and direct payouts
//! become authored components. The book-wide migration runs in the store's
//! ordered compatibility pass (`legacy_migration`); the per-profile drift
//! repair runs in the journal pre-build.
pub use super::sql::CapturedError as Error;
use super::{component_rows as components, component_store as store, pair_spec as spec, *};
use kassiber_envelope::{s, set, truthy};
use kassiber_pyjson::BigInt;
use kassiber_store::rusqlite::{params, TransactionState};
use std::collections::{BTreeMap, BTreeSet};

pub(super) type RefreshResult<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConsolidationResult {
    pub activated: usize,
    pub unchanged: usize,
    pub skipped: usize,
}

impl ConsolidationResult {
    pub fn changed(&self) -> bool {
        self.activated != 0 || self.skipped != 0
    }
}

struct Savepoint<'a>(&'a Connection, &'static str, bool);

impl Drop for Savepoint<'_> {
    fn drop(&mut self) {
        if self.2 {
            let _ = self
                .0
                .execute_batch(&format!("ROLLBACK TO {}; RELEASE {}", self.1, self.1));
        }
    }
}

/// Python's `_savepoint`: roll the step back on any error, release otherwise.
pub(super) fn atomic<T>(
    c: &Connection,
    name: &'static str,
    f: impl FnOnce() -> RefreshResult<T>,
) -> RefreshResult<T> {
    kassiber_store::begin_write_savepoint(c, name).map_err(sql::error)?;
    let mut guard = Savepoint(c, name, true);
    let result = f()?;
    c.execute_batch(&format!("RELEASE {name}"))
        .map_err(sql::error)?;
    guard.2 = false;
    Ok(result)
}

pub(super) struct FixedTime(pub(super) String);

impl Sources for FixedTime {
    fn now(&self, _: &Connection) -> Result<String> {
        Ok(self.0.clone())
    }
    fn uuid(&self, _: &Connection) -> Result<String> {
        Err(AppError::new(
            "internal_error",
            "legacy refresh requires deterministic identities",
        ))
    }
}

/// An `AppError` Python's migration catches and records as an issue; any
/// other failure (SQLite, Python `ValueError`/`TypeError` analogues)
/// propagates and rolls the step back.
pub(super) fn domain(error: &Error) -> bool {
    let Error::App(error) = error else {
        return false;
    };
    error.code.starts_with("custody_")
        || matches!(
            error.code.as_str(),
            "not_found" | "validation" | "app_error"
        )
}

/// `_pair_rows`: reviewed pairs joined to both observed boundaries.
pub(super) fn pair_rows(c: &Connection, include_deleted: bool) -> Result<Vec<Value>> {
    rows(
        c,
        &format!(
            "SELECT p.*, 'transaction_pairs' AS legacy_table,
        o.asset AS out_asset,o.amount AS out_tx_amount,o.fee AS out_fee,
        o.amount_includes_fee AS out_amount_includes_fee,o.direction AS out_direction,
        o.occurred_at AS out_occurred_at,o.raw_json AS out_raw_json,o.wallet_id AS out_wallet_id,
        ow.kind AS out_wallet_kind,ow.config_json AS out_wallet_config_json,
        i.asset AS in_asset,i.amount AS in_tx_amount,i.fee AS in_fee,
        i.amount_includes_fee AS in_amount_includes_fee,i.direction AS in_direction,
        i.occurred_at AS in_occurred_at,i.raw_json AS in_raw_json,i.wallet_id AS in_wallet_id,
        iw.kind AS in_wallet_kind,iw.config_json AS in_wallet_config_json
        FROM transaction_pairs p JOIN transactions o ON o.id=p.out_transaction_id
        JOIN wallets ow ON ow.id=o.wallet_id JOIN transactions i ON i.id=p.in_transaction_id
        JOIN wallets iw ON iw.id=i.wallet_id {}
        ORDER BY p.profile_id,p.created_at,p.id",
            if include_deleted {
                ""
            } else {
                "WHERE p.deleted_at IS NULL"
            }
        ),
        [],
    )
}

/// `_connected_pair_groups`: pairs sharing a transaction migrate together.
pub(super) fn connected(rows: &[Value]) -> Vec<Vec<Value>> {
    let mut by_tx = BTreeMap::<String, Vec<usize>>::new();
    for (i, row) in rows.iter().enumerate() {
        for key in ["out_transaction_id", "in_transaction_id"] {
            by_tx.entry(text(row, key)).or_default().push(i);
        }
    }
    let mut remaining: BTreeSet<_> = (0..rows.len()).collect();
    let mut groups = Vec::new();
    while let Some(i) = remaining.first().copied() {
        let mut pending = vec![i];
        let mut group = BTreeSet::new();
        while let Some(i) = pending.pop() {
            if !group.insert(i) {
                continue;
            }
            remaining.remove(&i);
            for key in ["out_transaction_id", "in_transaction_id"] {
                pending.extend(by_tx[&text(&rows[i], key)].iter().copied());
            }
        }
        groups.push(group.into_iter().map(|i| rows[i].clone()).collect());
    }
    groups
}

/// `_link_legacy_row` for each row of a group.
pub(super) fn link(c: &Connection, group: &[Value], id: &str) -> Result<()> {
    for row in group {
        let table = match text(row, "legacy_table").as_str() {
            "transaction_pairs" => "transaction_pairs",
            "direct_swap_payouts" => "direct_swap_payouts",
            other => {
                return Err(AppError::new(
                    "internal_error",
                    format!("unsupported legacy custody table: {other}"),
                ))
            }
        };
        c.execute(
            &format!("UPDATE {table} SET component_id=? WHERE id=?"),
            params![id, sql::bind(&value(row, "id"), 2)?],
        )
        .map_err(sql::error)?;
    }
    Ok(())
}

/// `int(value)` with Python's text, float and bool rules; a text value that
/// is not an integer is Python's `ValueError`, which no migration catches.
pub(super) fn python_int(raw: &Value) -> RefreshResult<BigInt> {
    if let Value::Str(text) = raw {
        return kassiber_pytext::parse_int(text.as_str().unwrap_or_default()).map_err(|error| Error::Python {
            family: "ValueError",
            message: match error {
                kassiber_pytext::NumberError::Invalid => format!(
                    "invalid literal for int() with base 10: {}", kassiber_envelope::py_repr(raw)),
                kassiber_pytext::NumberError::TooManyDigits(n) => format!(
                    "Exceeds the limit (4300 digits) for integer string conversion: value has {n} digits; use sys.set_int_max_str_digits() to increase the limit"),
            },
        });
    }
    Ok(integer(raw)?)
}

/// `row_boundary_amounts` reads `int(amount or 0)` and `int(fee or 0)`.
pub(super) fn python_int_or_zero(raw: &Value) -> RefreshResult<BigInt> {
    if truthy(raw) {
        python_int(raw)
    } else {
        Ok(BigInt::default())
    }
}

/// `_pair_group_spec(rows, require_full_source=full)`.
pub(super) fn group_spec(group: &[Value], full: bool) -> RefreshResult<Value> {
    let mut ordered = group.to_vec();
    ordered.sort_by_key(|r| (text(r, "created_at"), text(r, "id")));
    let hashes = ordered
        .iter()
        .map(|r| store::hash(&project(r, spec::FIELDS), false))
        .collect::<Result<Vec<_>>>()?;
    for row in &mut ordered {
        for key in [
            "out_tx_amount",
            "out_fee",
            "in_tx_amount",
            "in_fee",
            "out_amount",
        ] {
            let raw = value(row, key);
            if key == "out_amount" && (matches!(raw, Value::Null) || raw == s("")) {
                continue;
            }
            let parsed = python_int_or_zero(&raw)?;
            set(row, key, Value::Int(parsed));
        }
    }
    spec::group_hashed(&ordered, full, &hashes).map_err(Error::App)
}

/// What `_activate_native_review` authors besides the spec.
pub(super) struct NativeReview<'a> {
    pub term_kind: &'a str,
    pub source_hash: &'a str,
    pub change_reason: &'a str,
    pub evidence: Value,
}

/// `_activate_native_review` with `authored_source="migration"` and
/// `evidence_kind="legacy_review_migration"`: create (or revise the active
/// revision of) the spec's lineage and activate it at the review's time.
pub(super) fn activate_native_review(
    c: &Connection,
    row: &Value,
    mut plan: Value,
    review: &NativeReview<'_>,
    sources: &dyn Sources,
) -> RefreshResult<String> {
    let workspace = text(row, "workspace_id");
    let profile = text(row, "profile_id");
    let scope = Scope {
        workspace_id: &workspace,
        profile_id: &profile,
    };
    let prior = rows(c, "SELECT id FROM custody_components WHERE profile_id=? AND lineage_id=? AND state='active' ORDER BY revision DESC LIMIT 1",
        params![sql::bind(&value(row, "profile_id"), 1)?, text(&plan, "lineage_id")])?.into_iter().next();
    if let Some(prior) = &prior {
        if !rows(
            c,
            "SELECT id FROM custody_components WHERE id=?",
            [text(&plan, "component_id")],
        )?
        .is_empty()
        {
            spec::retarget(&mut plan, &text(prior, "id"));
        }
    }
    let terms = match get(&plan, "terms") {
        Some(terms) if truthy(terms) => terms.clone(),
        _ => {
            super::legacy_migration::legacy_terms(row, &plan, review.term_kind, review.source_hash)?
        }
    };
    let mut input = plan.clone();
    for (key, v) in [
        ("economic_terms", terms),
        ("evidence_kind", s("legacy_review_migration")),
        ("evidence_grade", s("reviewed")),
        ("evidence", review.evidence.clone()),
        ("conversion_metadata", dict::<&str>([])),
        ("notes", value(row, "notes")),
        ("authored_source", s("migration")),
        ("created_at", s(py_str(&value(row, "created_at")))),
        (
            "change_reason",
            if prior.is_some() {
                s(review.change_reason)
            } else {
                Value::Null
            },
        ),
    ] {
        set(&mut input, key, v);
    }
    let id = if let Some(prior) = &prior {
        let when = components::timestamp(&value(row, "created_at"))?;
        components::revise(
            c,
            &text(prior, "id"),
            &text(&plan, "component_id"),
            &input,
            &FixedTime(py_str(&when)),
        )?
    } else {
        components::create(c, &scope, &input, "migration", sources)?
    };
    components::activate(
        c,
        &scope,
        &id,
        &FixedTime(py_str(&value(row, "created_at"))),
    )?;
    Ok(id)
}

/// `get_component(conn, id)`: materialized in the component's own profile.
pub(super) fn component(c: &Connection, id: &str) -> Result<Value> {
    let header = rows(
        c,
        "SELECT workspace_id,profile_id FROM custody_components WHERE id=?",
        [id],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| {
        AppError::new("not_found", "custody component was not found")
            .with_details(dict([("component_id", s(id))]))
    })?;
    let workspace = text(&header, "workspace_id");
    let profile = text(&header, "profile_id");
    components::get_component(
        c,
        &Scope {
            workspace_id: &workspace,
            profile_id: &profile,
        },
        id,
        false,
    )
}

/// `supersede_component` in the component's own profile.
pub(super) fn supersede(
    c: &Connection,
    id: &str,
    reason: &str,
    superseded_at: Option<&Value>,
    sources: &dyn Sources,
) -> RefreshResult<()> {
    let header = rows(
        c,
        "SELECT workspace_id,profile_id FROM custody_components WHERE id=?",
        [id],
    )?
    .into_iter()
    .next()
    .unwrap_or_else(|| dict([("workspace_id", s("")), ("profile_id", s(""))]));
    let workspace = text(&header, "workspace_id");
    let profile = text(&header, "profile_id");
    components::supersede(
        c,
        &Scope {
            workspace_id: &workspace,
            profile_id: &profile,
        },
        id,
        &s(reason),
        superseded_at,
        sources,
    )?;
    Ok(())
}

/// `_consolidate_pair_group`: one atomic component per connected pair
/// group, reusing an exact active component or revising the group's lineage.
/// Returns whether anything was activated or linked.
pub(super) fn consolidate(
    c: &Connection,
    group: &[Value],
    sources: &dyn Sources,
) -> RefreshResult<bool> {
    atomic(c, "legacy_pair_group", || {
        let mut plan = group_spec(group, true)?;
        let mut linked: BTreeSet<_> = group
            .iter()
            .map(|r| value(r, "component_id"))
            .filter(|v| !matches!(v, Value::Null) && *v != s(""))
            .map(|v| py_str(&v))
            .collect();
        if linked.len() == 1 {
            let current = component(c, linked.first().unwrap())?;
            if text(&current, "effective_state") == "active"
                && store::list(&current, "economic_terms").len() == group.len()
            {
                return Ok(false);
            }
        }
        let exact = text(&plan, "component_id");
        if !rows(c, "SELECT id FROM custody_components WHERE id=?", [&exact])?.is_empty() {
            let exact_component = component(c, &exact)?;
            if text(&exact_component, "effective_state") == "active"
                && store::list(&exact_component, "economic_terms").len() == group.len()
            {
                link(c, group, &exact)?;
                return Ok(true);
            }
            linked.insert(exact.clone());
            spec::retarget(&mut plan, &exact);
        }
        let mut parent = None;
        for id in &linked {
            if parent.is_none()
                && text(&component(c, id)?, "lineage_id") == text(&plan, "lineage_id")
            {
                parent = Some(id.clone());
            }
        }
        for id in &linked {
            if Some(id) != parent.as_ref() {
                supersede(
                    c,
                    id,
                    "consolidated into authored pair group",
                    None,
                    sources,
                )?;
            }
        }
        let base = &group[0];
        let hash = store::hash(&project(base, spec::FIELDS), false)?;
        let mut ids: Vec<_> = group.iter().map(|r| text(r, "id")).collect();
        ids.sort();
        let evidence = dict([
            ("legacy_table", s("transaction_pairs")),
            (
                "legacy_source_ids",
                Value::List(ids.iter().map(|id| s(id.as_str())).collect()),
            ),
        ]);
        let id = activate_native_review(
            c,
            base,
            plan,
            &NativeReview {
                term_kind: "transaction_pair",
                source_hash: &hash,
                change_reason: "consolidate legacy reviewed pair group",
                evidence,
            },
            sources,
        )?;
        link(c, group, &id)?;
        Ok(true)
    })
}

/// `_record_migration_issue`: a durable, retried fail-closed blocker per row.
pub(super) fn record_issue(
    c: &Connection,
    group: &[Value],
    error: &Error,
    sources: &dyn Sources,
) -> Result<()> {
    let Error::App(error) = error else {
        return Err(AppError::new("internal_error", "non-domain legacy refusal"));
    };
    let now = sources.now(c)?;
    for row in group {
        let table = text(row, "legacy_table");
        let legacy = text(row, "id");
        let ids: BTreeSet<_> = [
            value(row, "out_transaction_id"),
            value(row, "in_transaction_id"),
        ]
        .into_iter()
        .filter(|v| !matches!(v, Value::Null) && *v != s(""))
        .map(|v| py_str(&v))
        .collect();
        c.execute(
            "INSERT INTO custody_authored_migration_issues(
            id,workspace_id,profile_id,legacy_table,legacy_source_id,issue_code,
            transaction_ids_json,details_json,resolved_at,created_at,updated_at)
            VALUES(?,?,?,?,?,?,?,?,NULL,?,?)
            ON CONFLICT(legacy_table,legacy_source_id) DO UPDATE SET issue_code=excluded.issue_code,
            transaction_ids_json=excluded.transaction_ids_json,details_json=excluded.details_json,
            resolved_at=NULL,updated_at=excluded.updated_at",
            params![
                spec::stable(&["custody_authored_migration_issue", &table, &legacy]),
                sql::bind(&value(row, "workspace_id"), 2)?,
                sql::bind(&value(row, "profile_id"), 3)?,
                table,
                legacy,
                if error.code.is_empty() {
                    "custody_legacy_migration_failed"
                } else {
                    error.code.as_str()
                },
                store::json(&Value::List(ids.iter().map(|id| s(id.as_str())).collect()))?,
                store::json(
                    error
                        .details
                        .as_ref()
                        .filter(|v| truthy(v))
                        .unwrap_or(&dict::<&str>([]))
                )?,
                now,
                now
            ],
        )
        .map_err(sql::error)?;
    }
    Ok(())
}

/// `_resolve_migration_issues`.
pub(super) fn resolve_issues(c: &Connection, group: &[Value], sources: &dyn Sources) -> Result<()> {
    let now = sources.now(c)?;
    for row in group {
        c.execute("UPDATE custody_authored_migration_issues SET resolved_at=COALESCE(resolved_at,?),updated_at=?
            WHERE legacy_table=? AND legacy_source_id=? AND resolved_at IS NULL",
            params![now,now,text(row,"legacy_table"),sql::bind(&value(row,"id"),4)?]).map_err(sql::error)?;
    }
    Ok(())
}

pub fn refresh_drifted_legacy_pair_components(
    c: &Connection,
    scope: &Scope<'_>,
    sources: &dyn Sources,
) -> std::result::Result<ConsolidationResult, Error> {
    sql::capture_sqlite(|| refresh(c, scope, sources))
}

fn refresh(
    c: &Connection,
    scope: &Scope<'_>,
    sources: &dyn Sources,
) -> RefreshResult<ConsolidationResult> {
    if c.transaction_state(Some("main")).map_err(sql::error)? != TransactionState::Write {
        return Err(AppError::new(
            "internal_error",
            "journal pre-build requires the caller's write transaction",
        )
        .into());
    }
    atomic(c, "legacy_pair_refresh", || {
        let mut repairable = BTreeSet::new();
        for row in rows(c, "SELECT DISTINCT component_id FROM transaction_pairs WHERE profile_id=? AND deleted_at IS NULL AND component_id IS NOT NULL", [scope.profile_id])? {
            let id = text(&row, "component_id");
            let component = components::get_component(c, scope, &id, true)?;
            let validation = value(&component, "validation");
            let issues: BTreeSet<_> = store::list(&validation, "issues").iter().map(|i| text(i, "code")).collect();
            if text(&component,"state") == "active" && text(&component,"effective_state") == "draft"
                && text(&component,"authored_source") == "migration" && text(&component,"evidence_kind") == "legacy_review_migration"
                && text(&value(&component,"evidence_status"),"status") == "evidence_mismatch"
                && issues == BTreeSet::from(["component_evidence_commitment_invalid".to_owned()])
                && truthy(&value(&value(&validation,"anchors"),"valid")) {
                repairable.insert(id);
            }
        }
        if repairable.is_empty() {
            return Ok(ConsolidationResult {
                unchanged: 1,
                ..Default::default()
            });
        }
        migrate_pair_groups(c, Some(&repairable), sources)
    })
}

/// `_migrate_legacy_pair_groups`: every connected active pair group, or
/// only the groups linked entirely to `repair` components.
pub(super) fn migrate_pair_groups(
    c: &Connection,
    repair: Option<&BTreeSet<String>>,
    sources: &dyn Sources,
) -> RefreshResult<ConsolidationResult> {
    let pairs = pair_rows(c, false)?;
    atomic(c, "custody_pair_consolidation", || {
        let mut result = ConsolidationResult::default();
        for group in connected(&pairs) {
            if let Some(repair) = repair {
                let linked: BTreeSet<_> = group
                    .iter()
                    .map(|r| value(r, "component_id"))
                    .filter(|v| !matches!(v, Value::Null) && *v != s(""))
                    .map(|v| py_str(&v))
                    .collect();
                if linked.is_empty() || !linked.is_subset(repair) {
                    continue;
                }
            }
            match consolidate(c, &group, sources) {
                Ok(activated) => {
                    resolve_issues(c, &group, sources)?;
                    if activated {
                        result.activated += 1;
                    } else {
                        result.unchanged += 1;
                    }
                }
                Err(error) if domain(&error) => {
                    record_issue(c, &group, &error, sources)?;
                    result.skipped += 1;
                }
                Err(error) => return Err(error),
            }
        }
        Ok(result)
    })
}
