pub use super::sql::CapturedError as Error;
use super::{component_rows as components, component_store as store, pair_spec as spec, *};
use kassiber_envelope::{s, set, truthy};
use kassiber_store::rusqlite::{params, TransactionState};
use std::collections::{BTreeMap, BTreeSet};

type RefreshResult<T> = std::result::Result<T, Error>;

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

fn atomic<T>(
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

struct FixedTime(String);

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

fn domain(error: &Error) -> bool {
    let Error::App(error) = error else {
        return false;
    };
    error.code.starts_with("custody_")
        || matches!(
            error.code.as_str(),
            "not_found" | "validation" | "app_error"
        )
}

fn pair_rows(c: &Connection) -> Result<Vec<Value>> {
    rows(
        c,
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
        JOIN wallets iw ON iw.id=i.wallet_id WHERE p.deleted_at IS NULL
        ORDER BY p.profile_id,p.created_at,p.id",
        [],
    )
}

fn connected(rows: &[Value]) -> Vec<Vec<Value>> {
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

fn link(c: &Connection, group: &[Value], id: &str) -> Result<()> {
    for row in group {
        c.execute(
            "UPDATE transaction_pairs SET component_id=? WHERE id=?",
            [id, &text(row, "id")],
        )
        .map_err(sql::error)?;
    }
    Ok(())
}

fn group_spec(group: &[Value]) -> RefreshResult<Value> {
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
            let raw = if truthy(&raw) {
                raw
            } else {
                Value::Int(0.into())
            };
            let parsed = if let Value::Str(text) = &raw {
                kassiber_pytext::parse_int(text.as_str().unwrap_or_default()).map_err(|error| Error::Python {
                    family: "ValueError",
                    message: match error {
                        kassiber_pytext::NumberError::Invalid => format!(
                            "invalid literal for int() with base 10: {}", kassiber_envelope::py_repr(&raw)),
                        kassiber_pytext::NumberError::TooManyDigits(n) => format!(
                            "Exceeds the limit (4300 digits) for integer string conversion: value has {n} digits; use sys.set_int_max_str_digits() to increase the limit"),
                    },
                })?
            } else {
                integer(&raw)?
            };
            set(row, key, Value::Int(parsed));
        }
    }
    spec::group_hashed(&ordered, true, &hashes).map_err(Error::App)
}

fn consolidate(
    c: &Connection,
    scope: &Scope<'_>,
    group: &[Value],
    sources: &dyn Sources,
) -> RefreshResult<bool> {
    atomic(c, "legacy_pair_group", || {
        let mut plan = group_spec(group)?;
        let mut linked: BTreeSet<_> = group
            .iter()
            .map(|r| value(r, "component_id"))
            .filter(|v| !matches!(v, Value::Null) && *v != s(""))
            .map(|v| py_str(&v))
            .collect();
        if linked.len() == 1 {
            let current = components::get_component(c, scope, linked.first().unwrap(), false)?;
            if text(&current, "effective_state") == "active"
                && store::list(&current, "economic_terms").len() == group.len()
            {
                return Ok(false);
            }
        }
        let exact = text(&plan, "component_id");
        if !rows(c, "SELECT id FROM custody_components WHERE id=?", [&exact])?.is_empty() {
            let component = components::get_component(c, scope, &exact, false)?;
            if text(&component, "effective_state") == "active"
                && store::list(&component, "economic_terms").len() == group.len()
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
                && text(
                    &components::get_component(c, scope, id, false)?,
                    "lineage_id",
                ) == text(&plan, "lineage_id")
            {
                parent = Some(id.clone());
            }
        }
        for id in &linked {
            if Some(id) != parent.as_ref() {
                components::supersede(
                    c,
                    scope,
                    id,
                    &s("consolidated into authored pair group"),
                    sources,
                )?;
            }
        }
        let prior = rows(c, "SELECT id FROM custody_components WHERE profile_id=? AND lineage_id=? AND state='active' ORDER BY revision DESC LIMIT 1",
            [scope.profile_id, &text(&plan, "lineage_id")])?.into_iter().next();
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
        let base = &group[0];
        let mut ids: Vec<_> = group.iter().map(|r| text(r, "id")).collect();
        ids.sort();
        let evidence = dict([
            ("legacy_table", s("transaction_pairs")),
            (
                "legacy_source_ids",
                Value::List(ids.iter().map(|id| s(id.as_str())).collect()),
            ),
        ]);
        let mut input = plan.clone();
        for (key, v) in [
            ("economic_terms", value(&plan, "terms")),
            ("evidence_kind", s("legacy_review_migration")),
            ("evidence_grade", s("reviewed")),
            ("evidence", evidence),
            ("conversion_metadata", dict::<&str>([])),
            ("notes", value(base, "notes")),
            ("authored_source", s("migration")),
            ("created_at", value(base, "created_at")),
            (
                "change_reason",
                if prior.is_some() {
                    s("consolidate legacy reviewed pair group")
                } else {
                    Value::Null
                },
            ),
        ] {
            set(&mut input, key, v);
        }
        let id = if let Some(prior) = &prior {
            let when = components::timestamp(&value(base, "created_at"))?;
            components::revise(
                c,
                &text(prior, "id"),
                &text(&plan, "component_id"),
                &input,
                &FixedTime(py_str(&when)),
            )?
        } else {
            components::create(c, scope, &input, "migration", sources)?
        };
        components::activate(c, scope, &id, &FixedTime(text(base, "created_at")))?;
        link(c, group, &id)?;
        Ok(true)
    })
}

fn record_issue(
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
            VALUES(?,?,?,'transaction_pairs',?,?,?,?,NULL,?,?)
            ON CONFLICT(legacy_table,legacy_source_id) DO UPDATE SET issue_code=excluded.issue_code,
            transaction_ids_json=excluded.transaction_ids_json,details_json=excluded.details_json,
            resolved_at=NULL,updated_at=excluded.updated_at",
            params![
                spec::stable(&[
                    "custody_authored_migration_issue",
                    "transaction_pairs",
                    &legacy
                ]),
                text(row, "workspace_id"),
                text(row, "profile_id"),
                legacy,
                error.code,
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

fn resolve_issues(c: &Connection, group: &[Value], sources: &dyn Sources) -> Result<()> {
    let now = sources.now(c)?;
    for row in group {
        c.execute("UPDATE custody_authored_migration_issues SET resolved_at=COALESCE(resolved_at,?),updated_at=?
            WHERE legacy_table='transaction_pairs' AND legacy_source_id=? AND resolved_at IS NULL",
            params![now,now,text(row,"id")]).map_err(sql::error)?;
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
        let mut result = ConsolidationResult::default();
        for group in connected(&pair_rows(c)?) {
            let linked: BTreeSet<_> = group
                .iter()
                .map(|r| value(r, "component_id"))
                .filter(|v| !matches!(v, Value::Null) && *v != s(""))
                .map(|v| py_str(&v))
                .collect();
            if linked.is_empty() || !linked.is_subset(&repairable) {
                continue;
            }
            match consolidate(c, scope, &group, sources) {
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
