//! Validation preceding the daemon's explicit local-auth round trip.
use super::*;

pub const KINDS: &[&str] = &[
    "ui.workspace.delete",
    "ui.profiles.reset_data",
    "backends.reveal_token",
    "wallets.reveal_descriptor",
];
pub struct Authorization {
    pub scope: &'static str,
    pub label: String,
    pub ack_key: &'static str,
    pub ack_value: &'static str,
    pub ack_hint: String,
}

/// Context comes from the currently owned book, never from renderer arguments.
pub fn prepare(kind: &str, args: &Value, context: &Value) -> Result<Authorization> {
    let (scope, label, reveal) = if ["backends.reveal_token", "wallets.reveal_descriptor"]
        .contains(&kind)
    {
        let noun = if kind == "wallets.reveal_descriptor" {
            "wallet"
        } else {
            "backend"
        };
        let target = ["name", "wallet", "backend"]
            .into_iter()
            .filter_map(|k| get(args, k))
            .find(|v| truthy(v));
        let Some(Value::Str(target)) = target.filter(|v| matches!(v, Value::Str(_))) else {
            return Err(AppError::new(
                "validation",
                format!("{noun} reveal request requires a string `name` (or `wallet`/`backend`)"),
            ));
        };
        (
            if noun == "wallet" {
                "reveal_descriptor"
            } else {
                "reveal_token"
            },
            format!(
                "Re-enter database passphrase to reveal {noun} {}",
                kassiber_pytext::repr_py(target)
            ),
            true,
        )
    } else {
        let reset = kind == "ui.profiles.reset_data";
        let (verb, noun, confirmation, scope) = if reset {
            ("Reset", "book", "RESET", "reset_book_data")
        } else {
            ("Deletion", "books set", "DELETE", "delete_workspace")
        };
        if get(args, "confirm") != Some(&s(confirmation)) {
            return Err(
                AppError::new("validation", format!("{verb} confirmation is required.")).with_hint(
                    if reset {
                        "Ask the user to confirm the destructive book reset."
                    } else {
                        "Ask the user to confirm the destructive books set deletion."
                    },
                ),
            );
        }
        let key = if reset { "profile" } else { "workspace" };
        let name = text(context, &format!("{key}_label"));
        if name.is_empty() {
            return Err(
                AppError::new("validation", format!("No current {noun} is selected.")).with_hint(
                    if reset {
                        "Select a book before resetting its data."
                    } else {
                        "Select a books set before deleting it."
                    },
                ),
            );
        }
        if get(args, &format!("confirm_{key}")) != Some(&s(name.as_str())) {
            return Err(AppError::new(
                "validation",
                if reset {
                    "Book name confirmation is required."
                } else {
                    "Books set name confirmation is required."
                },
            )
            .with_hint(if reset {
                "Ask the user to type the exact current book name before resetting it."
            } else {
                "Ask the user to type the exact current books set name before deleting it."
            })
            .with_details(dict([(format!("expected_{key}"), s(name.as_str()))])));
        }
        if reset
            && optional(args, "clear_shared_rates").is_some_and(|v| !matches!(v, Value::Bool(_)))
        {
            return Err(AppError::new("validation", "Shared rate-cache reset flag must be a boolean.")
                .with_hint("Send clear_shared_rates as true only when the shared fiat-rate cache should be cleared.")
                .with_details(dict([("field", s("clear_shared_rates"))])));
        }
        let name = kassiber_pytext::repr_str(&name);
        (
            scope,
            if reset {
                format!("Re-enter database passphrase to reset book {name}")
            } else {
                format!("Re-enter database passphrase to delete books set {name}")
            },
            false,
        )
    };
    let ack_value = if reveal {
        "COPY LOCAL SECRET"
    } else {
        "DELETE LOCAL DATA"
    };
    Ok(Authorization {
        scope,
        label,
        ack_key: if reveal {
            "plaintext_reveal_ack"
        } else {
            "plaintext_delete_ack"
        },
        ack_value,
        ack_hint: if reveal {
            "Ask the user to type 'COPY LOCAL SECRET' before revealing plaintext local secrets."
                .into()
        } else {
            "Ask the user to type 'DELETE LOCAL DATA' before changing plaintext local data.".into()
        },
    })
}

/// Scoped retained ledger rows block destructive setup operations.
pub fn require_no_legacy(conn: &Connection, profiles: &[String], action: &str) -> Result<()> {
    if profiles.is_empty() {
        return Ok(());
    }
    for row in rows(
        conn,
        r"SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'gl\_%' ESCAPE '\' ORDER BY name",
        [],
    )? {
        let table = text(&row, "name").replace('"', "\"\"");
        let scoped = rows(conn, &format!("PRAGMA table_info(\"{table}\")"), [])?
            .iter()
            .any(|r| get(r, "name") == Some(&s("profile_id")));
        let populated = if scoped {
            let marks = vec!["?"; profiles.len()].join(",");
            !rows(
                conn,
                &format!("SELECT 1 FROM \"{table}\" WHERE profile_id IN ({marks}) LIMIT 1"),
                rusqlite::params_from_iter(profiles),
            )?
            .is_empty()
        } else {
            !rows(conn, &format!("SELECT 1 FROM \"{table}\" LIMIT 1"), [])?.is_empty()
        };
        if populated {
            return Err(AppError::new("legacy_ledger_present", format!("This book holds archived general-ledger data from an earlier Kassiber version, so {action} is blocked."))
                .with_hint("General accounting was removed. Back up the project first, then run `kassiber maintenance purge-legacy-ledger` to review and permanently remove the archived ledger data."));
        }
    }
    Ok(())
}
