//! Supersede reviewed payout custody without rewriting immutable terms.
use super::*;
use component_store as store;
use kassiber_envelope::{s, set, truthy};
use kassiber_store::rusqlite::params;
use sql::{atomic, error};
use std::collections::BTreeSet;

pub(super) fn execute(
    c: &Connection,
    scope: &Scope<'_>,
    args: &Value,
    sources: &dyn Sources,
) -> Result<Value> {
    atomic(c, || {
        let id = text(args, "payout_id");
        let mut row = records::payout_records(c, scope)?
            .into_iter()
            .find(|r| text(r, "id") == id)
            .ok_or_else(|| AppError::new("not_found", "Direct swap payout not found"))?;
        if truthy(&value(&row, "deleted_at")) {
            return Ok(row);
        }
        let now = sources.now(c)?;
        let source = get(args, "authored_source")
            .filter(|v| !matches!(v, Value::Null))
            .map(py_str)
            .unwrap_or_else(|| "cli".into());
        c.execute("UPDATE custody_components SET state='superseded',superseded_at=?,change_reason=COALESCE(?,change_reason) WHERE id=? AND state!='superseded'",params![now,format!("direct_swap_payout review deleted by {source}"),text(&row,"component_id")]).map_err(error)?;
        store::invalidate(c, scope.profile_id)?;
        let snapshot = store::snapshot(c, scope)?;
        let computed = kassiber_custody::components::compute(&snapshot)
            .map_err(|e| AppError::new("internal_error", e.message))?;
        c.execute(
            "DELETE FROM custody_component_transaction_memberships WHERE profile_id=?",
            [scope.profile_id],
        )
        .map_err(error)?;
        for component in computed["components"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["effective_state"] == "active")
        {
            let ids: BTreeSet<_> = component["legs"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|l| l["transaction_id"].as_str())
                .collect();
            for tx in ids {
                c.execute("INSERT INTO custody_component_transaction_memberships(component_id,profile_id,transaction_id,created_at) VALUES(?,?,?,?)",params![component["id"].as_str().unwrap(),scope.profile_id,tx,now]).map_err(error)?;
            }
        }
        store::invalidate(c, scope.profile_id)?;
        set(&mut row, "deleted_at", s(now));
        Ok(row)
    })
}
