//! R3 test boundary. Not a CLI command, never installed or packaged.
use kassiber_store::{Book, Error, ErrorKind, Result};
use serde_json::{json, Value};
use std::{
    io::{self, BufRead, Write},
    path::Path,
};

fn field<'a>(request: &'a Value, key: &str) -> Result<&'a str> {
    request[key]
        .as_str()
        .ok_or_else(|| Error::new(ErrorKind::Validation, "missing probe argument"))
}
fn observe(request: &Value, label: &str) -> Result<()> {
    if request["kill_at"].as_str() == Some(label) {
        println!("{}", json!({"checkpoint":label}));
        io::stdout().flush().map_err(Error::io)?;
        let mut release = String::new();
        io::stdin().read_line(&mut release).map_err(Error::io)?;
    }
    if request["fail_at"].as_str() == Some(label) {
        return Err(Error::new(ErrorKind::Io, "injected storage failure"));
    }
    Ok(())
}
fn run(request: &Value) -> Result<Value> {
    let passphrase = request["fd"]
        .as_i64()
        .map(|fd| kassiber_secrets::read_passphrase_from_fd(fd as i32))
        .transpose()?;
    let pass = passphrase
        .as_ref()
        .map(kassiber_secrets::Passphrase::expose);
    let root = Path::new(field(request, "data_root")?);
    match field(request, "op")? {
        "restore-install" | "restore-recover" => {
            let namespace = kassiber_ownership::OwnerNamespace::isolated(
                Path::new(field(request, "owners")?).to_owned(),
            );
            let _maintenance = kassiber_store::restore::maintenance(&namespace, root, None)?;
            let mut points = Vec::new();
            let mut observer = |label: &str| {
                points.push(label.to_owned());
                observe(request, label)
            };
            let recovery = if request["op"] == "restore-install" {
                kassiber_store::restore::install_with_observer(
                    Path::new(field(request, "staging")?),
                    root,
                    &mut observer,
                )?
            } else {
                kassiber_store::restore::recover(root, &mut observer)?;
                None
            };
            Ok(json!({"recovery": recovery, "points": points}))
        }
        "verify-encrypted-file" => {
            let result = kassiber_store::encryption::verify_encrypted(
                Path::new(field(request, "database")?),
                pass.ok_or_else(|| {
                    Error::new(
                        ErrorKind::PassphraseRequired,
                        "probe secret descriptor is required",
                    )
                })?,
            )?;
            Ok(
                json!({"integrity_check":result.integrity_check,"cipher_integrity_check":result.cipher_integrity_check,"sqlite_master_rows":result.sqlite_master_rows}),
            )
        }
        "create-empty-encrypted" => {
            kassiber_store::test_hooks::create_empty_encrypted(
                &root.join("kassiber.sqlite3"),
                pass.ok_or_else(|| {
                    Error::new(
                        ErrorKind::PassphraseRequired,
                        "probe secret descriptor is required",
                    )
                })?,
                &mut |label| observe(request, label),
            )?;
            Ok(json!({"created":true}))
        }
        "migrate" => {
            kassiber_store::test_hooks::migrate(
                root,
                pass,
                field(request, "identity")?,
                field(request, "timestamp")?,
                &mut |label| observe(request, label),
            )?;
            Ok(json!({"migrated":true}))
        }
        "increment" | "write-hold" => {
            let mut book = Book::open(
                root,
                kassiber_store::OpenOptions {
                    passphrase: pass,
                    ..Default::default()
                },
            )?;
            let count = if request["op"] == "write-hold" {
                1
            } else {
                request["count"]
                    .as_u64()
                    .filter(|v| *v <= 1000)
                    .ok_or_else(|| {
                        Error::new(ErrorKind::Validation, "invalid synthetic write count")
                    })?
            };
            for _ in 0..count {
                book.transaction(|tx|{
                    let changed=tx.execute("UPDATE settings SET value=CAST(value AS INTEGER)+1 WHERE key='r3_counter'",[]).map_err(kassiber_store::map_sqlite_error)?;
                    if changed!=1{return Err(Error::new(ErrorKind::Validation,"synthetic counter is missing"));}
                    if request["op"]=="write-hold" {println!("{}",json!({"checkpoint":"writer.holding"}));io::stdout().flush().map_err(Error::io)?;let mut release=String::new();io::stdin().read_line(&mut release).map_err(Error::io)?;}
                    Ok(())
                })?;
            }
            Ok(json!({"writes":count}))
        }
        "json-roundtrip" => Ok(
            json!({"encoded":kassiber_store::test_hooks::json_roundtrip(field(request,"raw")?, request["ascii"].as_bool().unwrap_or(true))?}),
        ),
        "schema-step" => {
            kassiber_store::test_hooks::schema_step(
                root,
                field(request, "step")?,
                request["timestamp"].as_str(),
                &mut |label| observe(request, label),
            )?;
            Ok(json!({"migrated":true}))
        }
        "ownership-history" | "database-identity" => {
            kassiber_store::test_hooks::ownership_history(
                root,
                field(request, "identity")?,
                request["op"] == "database-identity",
                &mut |label| observe(request, label),
            )?;
            Ok(json!({"migrated":true}))
        }
        "graph-install" => {
            kassiber_store::test_hooks::graph(root)?;
            Ok(json!({"installed":true}))
        }
        "msat" => {
            kassiber_store::test_hooks::msat(root, &mut |label| observe(request, label))?;
            Ok(json!({"migrated":true}))
        }
        "state-root" | "project-migration" => {
            let context = kassiber_paths::Context {
                home: field(request, "home")?.into(),
                platform: serde_json::from_value(request["platform"].clone())
                    .map_err(|_| Error::new(ErrorKind::Validation, "invalid platform"))?,
                environ: request["environ"]
                    .as_object()
                    .map(|m| {
                        m.iter()
                            .map(|(k, v)| (k.into(), v.as_str().unwrap_or("").into()))
                            .collect()
                    })
                    .unwrap_or_default(),
            };
            if request["op"] == "project-migration" {
                let entry = kassiber_store::test_hooks::ensure_default_project(
                    &context,
                    request["state_root"].as_str().map(Path::new),
                    field(request, "timestamp")?,
                    &mut |label| observe(request, label),
                )?;
                if !request["ensure_only"].as_bool().unwrap_or(false) {
                    let book = Book::open(
                        &entry.path.join("data"),
                        kassiber_store::OpenOptions {
                            passphrase: pass,
                            ..Default::default()
                        },
                    )?;
                    kassiber_store::test_hooks::validate_project_migration(
                        &context,
                        &book,
                        field(request, "timestamp")?,
                        &mut |label| observe(request, label),
                    )?;
                }
                return Ok(json!({"project":entry}));
            }
            let path = kassiber_store::test_hooks::migrate_root(&context, &mut |label| {
                observe(request, label)
            })?;
            Ok(json!({"state_root":path}))
        }
        "retired" => {
            kassiber_store::test_hooks::retired(
                root,
                pass,
                field(request, "timestamp")?,
                &mut |label| observe(request, label),
            )?;
            Ok(json!({"retired":true}))
        }
        "encrypt" | "resume-encryption" | "rekey" => {
            let namespace =
                kassiber_ownership::OwnerNamespace::isolated(field(request, "owner_root")?.into());
            let pass = pass.ok_or_else(|| {
                Error::new(
                    ErrorKind::PassphraseRequired,
                    "probe secret descriptor is required",
                )
            })?;
            match field(request, "op")? {
                "encrypt" => {
                    let result = kassiber_store::test_hooks::encrypt(
                        &namespace,
                        root,
                        pass,
                        &mut |label| observe(request, label),
                    )?;
                    Ok(
                        json!({"user_version":result.plaintext_user_version,"auto_vacuum":result.plaintext_auto_vacuum}),
                    )
                }
                "resume-encryption" => {
                    kassiber_store::encryption::resume_encryption(&namespace, root, pass)?;
                    Ok(json!({"resumed":true}))
                }
                _ => {
                    let new = kassiber_secrets::read_passphrase_from_fd(
                        request["new_fd"].as_i64().ok_or_else(|| {
                            Error::new(ErrorKind::Validation, "missing new secret descriptor")
                        })? as i32,
                    )?;
                    kassiber_store::test_hooks::rekey(
                        &namespace,
                        root,
                        pass,
                        new.expose(),
                        &mut |label| observe(request, label),
                    )?;
                    Ok(json!({"rekeyed":true}))
                }
            }
        }
        "semantics" => {
            kassiber_store::test_hooks::semantics(
                root,
                pass,
                field(request, "timestamp")?,
                &mut |_| Ok(()),
            )?;
            Ok(json!({"migrated":true}))
        }
        "create" => {
            let book = kassiber_store::test_hooks::create(
                root,
                pass,
                field(request, "identity")?,
                field(request, "timestamp")?,
                &mut |label| {
                    if request["kill_at"].as_str() == Some(label) {
                        // The parent kills us after receiving the durable boundary.
                        println!("{}", json!({"checkpoint":label}));
                        io::stdout().flush().map_err(Error::io)?;
                        let mut release = String::new();
                        io::stdin().read_line(&mut release).map_err(Error::io)?;
                    }
                    if request["fail_at"].as_str() == Some(label) {
                        return Err(Error::new(ErrorKind::Io, "injected storage failure"));
                    }
                    Ok(())
                },
            )?;
            Ok(json!({"identity":book.instance_id()}))
        }
        "open-migrating" => {
            // The product's bootstrap open: the ordered pass, then admission.
            let book = Book::open_migrating(
                root,
                kassiber_store::OpenOptions {
                    passphrase: pass,
                    ..Default::default()
                },
                true,
            )?;
            Ok(json!({"identity":book.instance_id()}))
        }
        "open" => {
            let book = Book::open(
                root,
                kassiber_store::OpenOptions {
                    passphrase: pass,
                    ..Default::default()
                },
            )?;
            Ok(json!({"identity":book.instance_id()}))
        }
        _ => Err(Error::new(
            ErrorKind::Validation,
            "unknown storage probe operation",
        )),
    }
}
fn main() {
    let mut line = String::new();
    let read = { io::stdin().lock().read_line(&mut line) };
    let result = read
        .map_err(Error::io)
        .and_then(|_| {
            serde_json::from_str(&line)
                .map_err(|_| Error::new(ErrorKind::Validation, "invalid probe request"))
        })
        .and_then(|request| run(&request));
    match result {
        Ok(data) => println!("{}", json!({"ok":true,"data":data})),
        Err(error) => {
            println!("{}", json!({"ok":false,"error":error}));
            std::process::exit(1);
        }
    }
}
