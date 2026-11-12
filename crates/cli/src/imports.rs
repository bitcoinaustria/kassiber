//! The imports command family; decisions and persistence live in kassiber-imports.
use kassiber_envelope::{dict, s, set, AppError};
use kassiber_imports::Operation;
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;

use crate::argparse::Namespace;

pub fn execute(
    namespace: &Namespace,
    book: &mut dyn Book,
) -> Result<(&'static str, Value), AppError> {
    let (_, profile_id) =
        book.resolve_scope(namespace.get("workspace"), namespace.get("profile"))?;
    match namespace.text("imports_command") {
        Some("list") => {
            let limit = namespace.get("limit").cloned().unwrap_or(Value::Null);
            let batches = book.import_runs(Operation::List {
                profile_id,
                wallet: namespace
                    .text("wallet")
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned),
                limit,
            })?;
            Ok(("imports.list", dict([("batches", batches)])))
        }
        Some("rollback") => {
            let batch = namespace.text("batch").unwrap_or_default().to_owned();
            // Python resolves and computes the plan even on a confirmed call.
            let mut plan = book.import_runs(Operation::Plan {
                profile_id: profile_id.clone(),
                batch: batch.clone(),
            })?;
            if namespace.flag("dry_run") || !namespace.flag("confirm") {
                set(&mut plan, "applied", Value::Bool(false));
                set(
                    &mut plan,
                    "hint",
                    s("Re-run with --confirm to delete these transactions."),
                );
                Ok(("imports.rollback.plan", plan))
            } else {
                let result = book.import_runs(Operation::Rollback { profile_id, batch })?;
                Ok(("imports.rollback", result))
            }
        }
        _ => unreachable!("registered imports leaf"),
    }
}
