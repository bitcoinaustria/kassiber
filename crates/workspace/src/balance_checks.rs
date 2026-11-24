//! Balance checkpoints as the CLI, daemon, desktop and assistant see them
//! (docs/reference/balance-checkpoints.md). The wallets crate stores and
//! compares; this module adds the chain checkpoints derived from the stored
//! UTXO inventory and adapts request arguments. Every read is local.
use kassiber_envelope::{dict, get, py_str, s, truthy, AppError};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::Connection;
use kassiber_wallets::checkpoints;

use crate::Result;

/// Operations, by their CLI leaf name.
pub const OPERATIONS: [&str; 5] = ["list", "add", "remove", "import", "export"];
/// The most CSV text one import accepts.
const MAX_CSV_BYTES: usize = 8 * 1024 * 1024;

fn text_arg(args: &Value, key: &str) -> Result<Option<String>> {
    match get(args, key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Str(value)) => Ok(value
            .as_str()
            .map(|v| kassiber_pytext::strip(v).to_owned())
            .filter(|v| !v.is_empty())),
        Some(_) => Err(AppError::new(
            "validation",
            format!("{key} must be a string"),
        )),
    }
}

fn required(args: &Value, key: &str, operation: &str) -> Result<String> {
    text_arg(args, key)?.ok_or_else(|| {
        AppError::new(
            "validation",
            format!("checkpoints {operation} requires {key}"),
        )
    })
}

/// Refuse arguments an operation does not take, before any work.
fn check_fields(operation: &str, args: &Value, allowed: &[&str]) -> Result<()> {
    let Value::Dict(fields) = args else {
        return Ok(());
    };
    let mut unknown: Vec<String> = fields
        .iter()
        .map(|(key, _)| match key {
            kassiber_pyjson::Key::Str(key) => py_str(&Value::Str(key.clone())),
            _ => "<non-string key>".into(),
        })
        .filter(|key| !allowed.contains(&key.as_str()))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    unknown.sort();
    Err(AppError::new(
        "validation",
        format!("checkpoints {operation} received unsupported arguments"),
    )
    .with_details(dict([(
        "unsupported",
        Value::List(unknown.into_iter().map(s).collect()),
    )])))
}

/// Every checkpoint of the profile (or one wallet) compared with the book:
/// stored ones, chain checkpoints from the last stored UTXO refresh and
/// Lightning node checkpoints from the last stored node sync.
pub fn report(
    conn: &Connection,
    profile: &str,
    wallet: Option<&str>,
    history: bool,
) -> Result<Value> {
    let wallet_id = match wallet {
        Some(reference) => Some(kassiber_wallets::resolve(conn, profile, reference)?.id),
        None => None,
    };
    let derived = crate::overview::chain_checkpoints(conn, profile)?;
    let mut payload = checkpoints::report(conn, profile, wallet_id.as_deref(), derived, history)?;
    // The state a later add or remove binds to (`expected_input_version`).
    if let Some(journals @ Value::Dict(_)) = get(&payload, "journals").cloned() {
        let mut journals = journals;
        kassiber_envelope::set(
            &mut journals,
            "input_version",
            Value::Int(input_version(conn, profile)?.into()),
        );
        kassiber_envelope::set(&mut payload, "journals", journals);
    }
    Ok(payload)
}

/// The profile's journal input version.
fn input_version(conn: &Connection, profile: &str) -> Result<i64> {
    conn.query_row(
        "SELECT journal_input_version FROM profiles WHERE id=?",
        [profile],
        |r| r.get(0),
    )
    .map_err(kassiber_wallets::database_error)
}

/// A write bound to the book state its caller read: an `expected_input_version`
/// (the assistant always passes one) must still be the profile's, checked
/// inside the write's savepoint.
fn bound(conn: &Connection, profile: &str, args: &Value) -> Result<()> {
    let expected = match get(args, "expected_input_version") {
        None | Some(Value::Null) => return Ok(()),
        Some(Value::Int(n)) => i64::try_from(n.clone()).ok(),
        Some(_) => None,
    }
    .ok_or_else(|| AppError::new("validation", "expected_input_version must be an integer"))?;
    let current = input_version(conn, profile)?;
    if current != expected {
        return Err(AppError::new(
            "stale_context",
            "The book changed since its balance checks were read",
        )
        .with_hint("Read the balance checks again, then retry with their input version.")
        .with_details(dict([
            ("expected_input_version", Value::Int(expected.into())),
            ("current_input_version", Value::Int(current.into())),
        ]))
        .retryable(true));
    }
    Ok(())
}

/// The wallets whose latest checkpoints differ from the book, or whose chain
/// comparison waits for the next sync (`provisional`), for readiness
/// warnings and notifications: `[{wallet_id, wallet, origin, asset, as_of,
/// difference_msat}]`, one per differing latest checkpoint.
pub fn mismatches(conn: &Connection, profile: &str) -> Result<Vec<Value>> {
    let report = report(conn, profile, None, false)?;
    let Some(Value::List(entries)) = get(&report, "checkpoints") else {
        return Ok(vec![]);
    };
    Ok(entries
        .iter()
        // A chain comparison deferred to the next sync stays visible too.
        .filter(|e| {
            get(e, "status") == Some(&s("differs"))
                || get(e, "reason") == Some(&s("after_last_sync"))
        })
        .map(|e| {
            let field = |key: &str| get(e, key).cloned().unwrap_or(Value::Null);
            dict([
                ("status", field("status")),
                ("reason", field("reason")),
                (
                    "provisional",
                    Value::Bool(get(e, "status") != Some(&s("differs"))),
                ),
                ("wallet_id", field("wallet_id")),
                ("wallet", field("wallet")),
                ("origin", field("origin")),
                ("asset", field("asset")),
                ("as_of", field("as_of")),
                ("difference_msat", field("difference_msat")),
                ("difference_btc", field("difference_btc")),
            ])
        })
        .collect())
}

/// One operation. `cli` names the arguments as the CLI parser does (scope
/// flags included); the daemon passes its request arguments.
pub fn execute(
    conn: &Connection,
    profile: &str,
    operation: &str,
    args: &Value,
    cli: bool,
) -> Result<Value> {
    let scope: &[&'static str] = if cli { &["workspace", "profile"] } else { &[] };
    let allow =
        |own: &[&'static str]| -> Vec<&'static str> { own.iter().chain(scope).copied().collect() };
    match operation {
        "list" => {
            check_fields(operation, args, &allow(&["wallet", "history"]))?;
            let wallet = text_arg(args, "wallet")?;
            let history = get(args, "history").is_some_and(truthy);
            report(conn, profile, wallet.as_deref(), history)
        }
        "add" => {
            check_fields(
                operation,
                args,
                &allow(if cli {
                    &["wallet", "btc", "date", "asset", "note", "attachment"]
                } else {
                    &[
                        "wallet",
                        "btc",
                        "date",
                        "asset",
                        "note",
                        "attachment",
                        "expected_input_version",
                    ]
                }),
            )?;
            let wallet = required(args, "wallet", operation)?;
            let btc = required(args, "btc", operation)?;
            let date = required(args, "date", operation)?;
            let asset = text_arg(args, "asset")?;
            let note = text_arg(args, "note")?;
            let attachment = text_arg(args, "attachment")?;
            kassiber_store::begin_write_savepoint(conn, "checkpoint_add")
                .map_err(kassiber_wallets::database_error)?;
            let result = bound(conn, profile, args).and_then(|()| {
                checkpoints::add_manual(
                    conn,
                    profile,
                    &checkpoints::Manual {
                        wallet: &wallet,
                        btc: &btc,
                        as_of: &date,
                        asset: asset.as_deref(),
                        note: note.as_deref(),
                        attachment: attachment.as_deref(),
                    },
                )
            });
            finish(conn, "checkpoint_add", result)
        }
        "remove" => {
            check_fields(
                operation,
                args,
                &allow(if cli {
                    &["id"]
                } else {
                    &["id", "expected_input_version"]
                }),
            )?;
            let id = required(args, "id", operation)?;
            kassiber_store::begin_write_savepoint(conn, "checkpoint_remove")
                .map_err(kassiber_wallets::database_error)?;
            let result =
                bound(conn, profile, args).and_then(|()| checkpoints::remove(conn, profile, &id));
            finish(conn, "checkpoint_remove", result)
        }
        "import" => {
            // The CLI reads a file it is given; the desktop sends the text it
            // read through its own picker and never a path.
            check_fields(
                operation,
                args,
                &allow(if cli { &["file"] } else { &["csv", "source"] }),
            )?;
            let (csv, source) = match (get(args, "csv"), text_arg(args, "file")?) {
                (Some(Value::Str(csv)), None) => (
                    csv.as_str()
                        .ok_or_else(|| AppError::new("validation", "csv must be text"))?
                        .to_owned(),
                    text_arg(args, "source")?.unwrap_or_else(|| "upload".into()),
                ),
                (None | Some(Value::Null), Some(file)) => {
                    let path = std::path::Path::new(&file);
                    let size = std::fs::metadata(path)
                        .map_err(|e| AppError::new("filesystem_error", e.to_string()))?
                        .len();
                    if size > MAX_CSV_BYTES as u64 {
                        return Err(too_large());
                    }
                    let bytes = std::fs::read(path)
                        .map_err(|e| AppError::new("filesystem_error", e.to_string()))?;
                    let text = String::from_utf8(bytes).map_err(|_| {
                        AppError::new("validation", "Checkpoint CSV is not UTF-8 text")
                    })?;
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "file".into());
                    (text, name)
                }
                _ => {
                    return Err(AppError::new(
                        "validation",
                        "checkpoints import needs either csv text or a file",
                    ))
                }
            };
            if csv.len() > MAX_CSV_BYTES {
                return Err(too_large());
            }
            checkpoints::import_csv(conn, profile, &csv, &source)
        }
        "export" => {
            check_fields(
                operation,
                args,
                &allow(if cli {
                    &["wallet", "file"]
                } else {
                    &["wallet"]
                }),
            )?;
            let wallet_id = match text_arg(args, "wallet")? {
                Some(reference) => Some(kassiber_wallets::resolve(conn, profile, &reference)?.id),
                None => None,
            };
            let csv = checkpoints::export_csv(conn, profile, wallet_id.as_deref())?;
            let rows = csv.lines().count().saturating_sub(1);
            if let Some(file) = text_arg(args, "file")? {
                write_new(&file, &csv)?;
                return Ok(dict([
                    ("file", s(file)),
                    ("rows", Value::Int((rows as i64).into())),
                ]));
            }
            Ok(dict([
                ("csv", s(csv)),
                ("rows", Value::Int((rows as i64).into())),
            ]))
        }
        other => Err(AppError::new(
            "validation",
            format!("Unknown checkpoints operation '{other}'"),
        )),
    }
}

fn too_large() -> AppError {
    AppError::new(
        "validation",
        format!(
            "Checkpoint CSV is larger than {} MiB",
            MAX_CSV_BYTES / 1024 / 1024
        ),
    )
}

/// Write an export to a new file; an existing file is never overwritten.
fn write_new(file: &str, text: &str) -> Result<()> {
    use std::io::Write;
    let mut handle = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(file)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                AppError::new("conflict", format!("{file} already exists"))
                    .with_hint("Choose a new file name; exports never overwrite a file.")
            } else {
                AppError::new("filesystem_error", e.to_string())
            }
        })?;
    handle
        .write_all(text.as_bytes())
        .map_err(|e| AppError::new("filesystem_error", e.to_string()))
}

fn finish(conn: &Connection, name: &str, result: Result<Value>) -> Result<Value> {
    match &result {
        Ok(_) => conn.execute_batch(&format!("RELEASE {name}")),
        Err(_) => conn.execute_batch(&format!("ROLLBACK TO {name};RELEASE {name}")),
    }
    .map_err(kassiber_wallets::database_error)?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        let creation =
            kassiber_pytext::loads(include_str!("../../store/fixtures/creation.json")).unwrap();
        let Some(Value::List(statements)) = get(&creation, "statements") else {
            panic!("statements")
        };
        for sql in statements {
            conn.execute_batch(&py_str(sql)).unwrap();
        }
        conn.execute_batch(
            "INSERT INTO workspaces(id,label,created_at) VALUES('w','W','2025');
             INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('p','w','P','2025');
             INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES
               ('cold','w','p','Cold','descriptor','{\"descriptor\":\"wpkh(synthetic)\",\"chain\":\"bitcoin\",\"network\":\"mainnet\"}','2025'),
               ('never','w','p','Never synced','descriptor','{\"descriptor\":\"wpkh(other)\"}','2025'),
               ('file','w','p','File','custom','{}','2025');
             INSERT INTO wallet_utxo_refreshes(wallet_id,workspace_id,profile_id,chain,network,observed_count,active_count,last_seen_at)
               VALUES('cold','w','p','bitcoin','mainnet',2,2,'2025-03-01T10:00:00Z');
             INSERT INTO wallet_utxos(id,workspace_id,profile_id,wallet_id,chain,network,asset,amount,txid,vout,outpoint,confirmation_status,first_seen_at,last_seen_at,spent_at) VALUES
               ('u1','w','p','cold','bitcoin','mainnet','BTC',100000000001,'aa',0,'aa:0','confirmed','x','x',NULL),
               ('u2','w','p','cold','bitcoin','mainnet','BTC',5,'bb',0,'bb:0','confirmed','x','x',NULL),
               ('u3','w','p','cold','bitcoin','mainnet','BTC',7,'cc',0,'cc:0','confirmed','x','x','2025-02-01');",
        )
        .unwrap();
        conn
    }

    #[test]
    fn chain_checkpoints_read_the_stored_utxo_set_as_of_the_last_refresh() {
        let conn = book();
        let derived = crate::overview::chain_checkpoints(&conn, "p").unwrap();
        assert_eq!(derived.len(), 1, "{derived:?}");
        let chain = &derived[0];
        assert_eq!(chain.wallet_id, "cold");
        assert_eq!(chain.amount_msat, 100_000_000_006);
        assert_eq!(chain.as_of, "2025-03-01T10:00:00Z");
        assert_eq!(chain.origin, "chain");
        assert_eq!(chain.id, None);
        // No transactions yet: listed, but not comparable.
        let report = report(&conn, "p", Some("Cold"), false).unwrap();
        let Some(Value::List(rows)) = get(&report, "checkpoints") else {
            panic!()
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(get(&rows[0], "reason"), Some(&s("no_history")));
        assert!(mismatches(&conn, "p").unwrap().is_empty());
    }

    /// A Liquid wallet whose last sync saw no coin reports zero LBTC, not
    /// zero BTC, so it is compared with the wallet's LBTC postings.
    #[test]
    fn an_empty_liquid_inventory_is_a_zero_lbtc_checkpoint() {
        let conn = book();
        conn.execute_batch(
            "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES
               ('liquid','w','p','Liquid','descriptor','{\"descriptor\":\"ct(synthetic)\",\"chain\":\"liquid\",\"network\":\"liquidv1\"}','2025');
             INSERT INTO wallet_utxo_refreshes(wallet_id,workspace_id,profile_id,chain,network,observed_count,active_count,last_seen_at)
               VALUES('liquid','w','p','liquid','liquidv1',0,0,'2025-03-01T10:00:00Z');
             INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,occurred_at,direction,asset,amount,created_at)
               VALUES('l1','w','p','liquid','l1','2025-02-01T00:00:00Z','inbound','LBTC',5000,'2025');
             INSERT INTO journal_quantity_postings(posting_id,workspace_id,profile_id,transaction_id,occurred_at,asset,location_kind,location_id,amount_msat,state,created_at)
               VALUES('pl1','w','p','l1','2025-02-01T00:00:00Z','LBTC','wallet','liquid',5000,'booked','2025');
             UPDATE profiles SET last_processed_at='2025-03-02',
               last_processed_tx_count=(SELECT COUNT(*) FROM transactions WHERE profile_id='p' AND excluded=0),
               last_processed_input_version=journal_input_version WHERE id='p';",
        )
        .unwrap();
        conn.execute(
            "UPDATE profiles SET journal_semantics_version=? WHERE id='p'",
            [kassiber_store::semantics::JOURNAL_CALCULATION_SEMANTICS_VERSION],
        )
        .unwrap();
        let derived = crate::overview::chain_checkpoints(&conn, "p").unwrap();
        let liquid: Vec<_> = derived.iter().filter(|c| c.wallet_id == "liquid").collect();
        assert_eq!(liquid.len(), 1, "{derived:?}");
        assert_eq!(
            (liquid[0].asset.as_str(), liquid[0].amount_msat),
            ("LBTC", 0)
        );
        let report = report(&conn, "p", Some("Liquid"), false).unwrap();
        let Some(Value::List(rows)) = get(&report, "checkpoints") else {
            panic!()
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(get(&rows[0], "status"), Some(&s("differs")));
        assert_eq!(get(&rows[0], "book_msat"), Some(&Value::from(5000)));
    }

    /// A chain comparison deferred to the next sync still reaches the
    /// mismatch summary (readiness, the wallets list), marked provisional.
    #[test]
    fn a_deferred_chain_comparison_stays_in_the_mismatch_summary() {
        let conn = book();
        conn.execute_batch(
            "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,occurred_at,direction,asset,amount,created_at) VALUES
               ('c1','w','p','cold','c1','2025-02-01T00:00:00Z','inbound','BTC',100000000000,'2025'),
               ('c2','w','p','cold','c2','2025-03-01T10:00:05Z','inbound','BTC',6,'2025');
             INSERT INTO journal_quantity_postings(posting_id,workspace_id,profile_id,transaction_id,occurred_at,asset,location_kind,location_id,amount_msat,state,created_at) VALUES
               ('pc1','w','p','c1','2025-02-01T00:00:00Z','BTC','wallet','cold',100000000000,'booked','2025'),
               ('pc2','w','p','c2','2025-03-01T10:00:05Z','BTC','wallet','cold',6,'booked','2025');
             UPDATE profiles SET last_processed_at='2025-03-02',
               last_processed_tx_count=(SELECT COUNT(*) FROM transactions WHERE profile_id='p' AND excluded=0),
               last_processed_input_version=journal_input_version WHERE id='p';",
        )
        .unwrap();
        conn.execute(
            "UPDATE profiles SET journal_semantics_version=? WHERE id='p'",
            [kassiber_store::semantics::JOURNAL_CALCULATION_SEMANTICS_VERSION],
        )
        .unwrap();
        let found = mismatches(&conn, "p").unwrap();
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(get(&found[0], "wallet_id"), Some(&s("cold")));
        assert_eq!(get(&found[0], "status"), Some(&s("unknown")));
        assert_eq!(get(&found[0], "reason"), Some(&s("after_last_sync")));
        assert_eq!(get(&found[0], "provisional"), Some(&Value::Bool(true)));
        assert_eq!(get(&found[0], "difference_msat"), Some(&Value::from(6)));
    }

    #[test]
    fn the_daemon_shape_refuses_paths_and_unknown_arguments() {
        let conn = book();
        let args = dict([("file", s("/etc/passwd"))]);
        assert_eq!(
            execute(&conn, "p", "import", &args, false)
                .unwrap_err()
                .code,
            "validation"
        );
        let args = dict([("wallet", s("Cold")), ("bogus", s("x"))]);
        assert_eq!(
            execute(&conn, "p", "list", &args, false).unwrap_err().code,
            "validation"
        );
        let args = dict([("workspace", s("w"))]);
        assert_eq!(
            execute(&conn, "p", "list", &args, false).unwrap_err().code,
            "validation"
        );
        assert!(execute(&conn, "p", "list", &args, true).is_ok());
        let exported = execute(&conn, "p", "export", &dict::<&str>([]), false).unwrap();
        assert_eq!(get(&exported, "rows"), Some(&Value::from(0)));
    }
}
