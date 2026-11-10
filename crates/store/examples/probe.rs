//! Test-only probe. Cargo examples are not installed or packaged as CLI tools.
use kassiber_ownership::{OwnerKind, OwnerNamespace};
use kassiber_paths::{Context, Error, ErrorKind, Platform, ResolveOptions, Result};
use kassiber_secrets::{
    CredentialReader, NativeCredentialReader, Passphrase, UnlockOptions, UnlockSession,
};
use kassiber_store::{map_sqlite_error, Book, OpenOptions};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
};
fn string<'a>(request: &'a Value, key: &str) -> Result<&'a str> {
    request[key]
        .as_str()
        .ok_or_else(|| Error::new(ErrorKind::Validation, "probe argument is missing"))
}
fn namespace(request: &Value) -> Result<OwnerNamespace> {
    Ok(OwnerNamespace::isolated(PathBuf::from(string(
        request,
        "owner_root",
    )?)))
}
fn open(request: &Value) -> Result<Book> {
    let mut session = UnlockSession::default();
    if let Some(fd) = request["fd"].as_i64() {
        session.prime_fd(
            i32::try_from(fd)
                .map_err(|_| Error::new(ErrorKind::InvalidPassphrase, "invalid fd"))?,
        )?;
    }
    let expected = request["expected_id"].as_str();
    let root = Path::new(string(request, "data_root")?);
    session.open(
        root,
        UnlockOptions {
            non_interactive: true,
            expected_database_identity: expected,
        },
        &NativeCredentialReader,
        |pass, id| {
            Book::open(
                root,
                OpenOptions {
                    passphrase: pass,
                    expected_database_identity: id,
                    ..Default::default()
                },
            )
        },
    )
}
fn run(request: &Value, input: &mut impl BufRead) -> Result<Value> {
    match string(request, "op")? {
        "environment-home" => Ok(json!({"home":Context::from_env()?.home})),
        "resolve" => {
            let platform = serde_json::from_value::<Platform>(request["platform"].clone())
                .map_err(|_| Error::new(ErrorKind::Validation, "invalid platform"))?;
            let context = Context {
                platform,
                home: PathBuf::from(string(request, "home")?),
                environ: request["environ"]
                    .as_object()
                    .map(|m| {
                        m.iter()
                            .map(|(k, v)| (k.into(), v.as_str().unwrap_or("").into()))
                            .collect::<BTreeMap<_, _>>()
                    })
                    .unwrap_or_default(),
            };
            let options = serde_json::from_value::<ResolveOptions>(request["options"].clone())
                .map_err(|_| Error::new(ErrorKind::Validation, "invalid resolve options"))?;
            Ok(serde_json::to_value(context.resolve(&options)?).expect("paths JSON"))
        }
        "roots" => {
            let context = Context {
                platform: serde_json::from_value(request["platform"].clone())
                    .map_err(|_| Error::new(ErrorKind::Validation, "invalid platform"))?,
                home: PathBuf::from(string(request, "home")?),
                environ: request["environ"]
                    .as_object()
                    .map(|m| {
                        m.iter()
                            .map(|(k, v)| (k.into(), v.as_str().unwrap_or("").into()))
                            .collect()
                    })
                    .unwrap_or_default(),
            };
            let data = context.effective_data_root(Path::new(string(request, "data_root")?))?;
            Ok(
                json!({"native":context.native_state_root(),"default":context.default_state_root()?,"data_root":data,"state_root":context.state_root_for_data(&data)?,"database":kassiber_paths::resolve_database_path(&data)}),
            )
        }
        "identity" => {
            let p =
                namespace(request)?.canonical_project(Path::new(string(request, "data_root")?))?;
            Ok(
                json!({"identity":p.identity,"database":p.database,"lock_path":p.lock_path,"alias_lock_path":p.alias_lock_path,"local_lock_path":p.local_lock_path}),
            )
        }
        "lock-hold" | "lock-once" => {
            let ns = namespace(request)?;
            let project = ns.canonical_project(Path::new(string(request, "data_root")?))?;
            let role = match string(request, "role")? {
                "desktop" => OwnerKind::Desktop,
                "broker" => OwnerKind::Broker,
                _ => return Err(Error::new(ErrorKind::Validation, "invalid role")),
            };
            let _lease = ns.acquire(project, role, "r1-synthetic-probe")?;
            if request["op"] == "lock-hold" {
                println!("{}", json!({"ok":true,"data":{"holding":true}}));
                io::stdout().flush().map_err(Error::io)?;
                let mut release = String::new();
                input.read_line(&mut release).map_err(Error::io)?;
            }
            Ok(json!({"released":true}))
        }
        // A Rust-launched child inherits the owner-lock duplicates; the probe
        // then releases its own lease, so only the child holds the locks.
        #[cfg(unix)]
        "lock-child" => {
            use kassiber_ipc::process::{ChildCommand, Stdio};
            let ns = namespace(request)?;
            let project = ns.canonical_project(Path::new(string(request, "data_root")?))?;
            let role = match string(request, "role")? {
                "desktop" => OwnerKind::Desktop,
                "broker" => OwnerKind::Broker,
                _ => return Err(Error::new(ErrorKind::Validation, "invalid role")),
            };
            let lease = ns.acquire(project, role, "r6-inherited-probe")?;
            let inherited = lease.duplicate_for_child()?;
            let mut command = ChildCommand::new("/bin/sh");
            command.arg("-c").arg("cat >/dev/null").stdin(Stdio::Pipe);
            for token in inherited.tokens() {
                command.inherit(token);
            }
            let mut child = command.spawn().map_err(Error::io)?;
            inherited.close();
            lease.release();
            println!(
                "{}",
                json!({"ok":true,"data":{"holding":true,"child":child.id()}})
            );
            io::stdout().flush().map_err(Error::io)?;
            let mut release = String::new();
            input.read_line(&mut release).map_err(Error::io)?;
            let status = child.wait().map_err(Error::io)?;
            Ok(json!({"released":true,"child_exit":status.code()}))
        }
        #[cfg(windows)]
        "inherited-handle" => {
            use std::os::windows::io::{FromRawHandle, OwnedHandle};
            let raw = request["handle"]
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| Error::new(ErrorKind::Validation, "inherited handle missing"))?;
            // SAFETY: the Windows-only test launches this process with exactly
            // this inherited handle; the owned reader closes the child copy.
            let handle = unsafe { OwnedHandle::from_raw_handle(raw as _) };
            let secret = kassiber_secrets::read_passphrase_from_inherited_handle(handle)?;
            Ok(
                json!({"length":secret.expose().len(),"matches":secret.expose()==request["expected"].as_str().unwrap_or("")}),
            )
        }
        "fd" => {
            let secret = kassiber_secrets::read_passphrase_from_fd(
                request["fd"]
                    .as_i64()
                    .and_then(|n| i32::try_from(n).ok())
                    .ok_or_else(|| Error::new(ErrorKind::Validation, "fd missing"))?,
            )?;
            // Compare in-process; never return passphrases to the test output.
            Ok(
                json!({"length":secret.expose().len(),"matches":secret.expose()==request["expected"].as_str().unwrap_or("")}),
            )
        }
        "open" | "write-busy" | "open-hold" => {
            let mut book = open(request)?;
            if request["op"] == "open-hold" {
                println!("{}", json!({"ok":true,"data":{"holding":true}}));
                io::stdout().flush().map_err(Error::io)?;
                let mut release = String::new();
                input.read_line(&mut release).map_err(Error::io)?;
                book.transaction(|tx| {tx.execute("INSERT OR REPLACE INTO settings VALUES('r1_interop_held_rust','Rust after Python')",[]).map_err(map_sqlite_error)?;Ok(())})?;
            }
            if request["op"] == "write-busy" {
                book.connection()
                    .busy_timeout(std::time::Duration::from_millis(10))
                    .map_err(map_sqlite_error)?;
                book.transaction(|tx| {
                    tx.execute(
                        "INSERT INTO settings(key,value) VALUES('r1_interop_busy','must rollback')",
                        [],
                    )
                    .map_err(map_sqlite_error)?;
                    Ok(())
                })?;
            }
            if let Some(key) = request["write_key"].as_str() {
                if !key.starts_with("r1_interop_") {
                    return Err(Error::new(
                        ErrorKind::Validation,
                        "synthetic setting required",
                    ));
                }
                let value = string(request, "write_value")?;
                book.transaction(|tx| { tx.execute("INSERT INTO settings(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[key,value]).map_err(map_sqlite_error)?; Ok(()) })?;
            }
            let mut counts = serde_json::Map::new();
            let mut rows = serde_json::Map::new();
            for table in ["workspaces", "profiles"] {
                let mut stmt = book
                    .connection()
                    .prepare(&format!("SELECT label FROM {table} ORDER BY id"))
                    .map_err(map_sqlite_error)?;
                let labels = stmt
                    .query_map([], |r| r.get::<_, String>(0))
                    .map_err(map_sqlite_error)?
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(map_sqlite_error)?;
                rows.insert(table.into(), json!(labels));
            }
            for table in [
                "workspaces",
                "profiles",
                "accounts",
                "wallets",
                "transactions",
            ] {
                let n = book
                    .connection()
                    .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| {
                        r.get::<_, i64>(0)
                    })
                    .map_err(map_sqlite_error)?;
                counts.insert(table.into(), json!(n));
            }
            Ok(
                json!({"instance_id":book.instance_id(),"encrypted":book.encrypted(),"counts":counts,"rows":rows,"fingerprint":book.schema_fingerprint()?,"journal_mode":book.connection().query_row("PRAGMA journal_mode",[],|r|r.get::<_,String>(0)).map_err(map_sqlite_error)?,"sqlite_version":book.connection().query_row("SELECT sqlite_version()",[],|r|r.get::<_,String>(0)).map_err(map_sqlite_error)?}),
            )
        }
        "policy-unlock" => {
            use std::cell::{Cell, RefCell};
            struct MockReader<'a> {
                secret: RefCell<Option<Passphrase>>,
                reads: Cell<usize>,
                account: &'a str,
            }
            impl CredentialReader for MockReader<'_> {
                fn get_password(&self, service: &str, account: &str) -> Result<Option<Passphrase>> {
                    assert_eq!(service, kassiber_secrets::CLI_PASSPHRASE_SERVICE);
                    assert_eq!(account, self.account);
                    self.reads.set(self.reads.get() + 1);
                    Ok(self.secret.borrow_mut().take())
                }
            }
            let secret = request["fd"]
                .as_i64()
                .map(|fd| kassiber_secrets::read_passphrase_from_fd(fd as i32))
                .transpose()?;
            let reader = MockReader {
                secret: RefCell::new(secret),
                reads: Cell::new(0),
                account: request["account"].as_str().unwrap_or(""),
            };
            let mut session = UnlockSession::default();
            let root = Path::new(string(request, "data_root")?);
            let expected = request["expected_id"].as_str();
            let mut result = Ok(json!({}));
            for _ in 0..2 {
                result = session.open(
                    root,
                    UnlockOptions {
                        non_interactive: true,
                        expected_database_identity: expected,
                    },
                    &reader,
                    |pass, id| {
                        let book = Book::open(
                            root,
                            OpenOptions {
                                passphrase: pass,
                                expected_database_identity: id,
                                ..Default::default()
                            },
                        )?;
                        Ok(json!({"instance_id":book.instance_id()}))
                    },
                );
                if result.is_err() {
                    break;
                }
            }
            match result {
                Ok(mut data) => {
                    data["keyring_reads"] = json!(reader.reads.get());
                    Ok(data)
                }
                Err(mut e) => {
                    e.details["keyring_reads"] = json!(reader.reads.get());
                    e.details["stale_notices"] = json!(session.take_notices().len());
                    Err(e)
                }
            }
        }
        "backup" => {
            let passphrase = request["fd"]
                .as_i64()
                .map(|fd| kassiber_secrets::read_passphrase_from_fd(fd as i32))
                .transpose()?;
            let source = Book::open(
                Path::new(string(request, "data_root")?),
                OpenOptions {
                    passphrase: passphrase.as_ref().map(Passphrase::expose),
                    ..Default::default()
                },
            )?;
            let mut destination = Book::open(
                Path::new(string(request, "destination")?),
                OpenOptions {
                    passphrase: passphrase.as_ref().map(Passphrase::expose),
                    ..Default::default()
                },
            )?;
            source.hot_backup(&mut destination)?;
            Ok(json!({"instance_id":destination.instance_id()}))
        }
        "keyring-read" => {
            let service = string(request, "service")?;
            if !service.starts_with("Kassiber R1 synthetic ") {
                return Err(Error::new(
                    ErrorKind::Validation,
                    "synthetic service required",
                ));
            }
            let secret =
                NativeCredentialReader.get_password(service, string(request, "account")?)?;
            let replacement = kassiber_secrets::read_passphrase("Rust R1 '🔐".as_bytes())?;
            NativeCredentialReader.update_existing_password(
                service,
                string(request, "account")?,
                &replacement,
            )?;
            Ok(json!({"matches":secret.as_ref().map(Passphrase::expose)==Some("Python R1 '🔑")}))
        }
        _ => Err(Error::new(ErrorKind::Validation, "unknown probe operation")),
    }
}
fn main() {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut line = String::new();
    let result = input
        .read_line(&mut line)
        .map_err(Error::io)
        .and_then(|_| {
            serde_json::from_str::<Value>(&line)
                .map_err(|_| Error::new(ErrorKind::Validation, "invalid probe request"))
        })
        .and_then(|request| run(&request, &mut input));
    match result {
        Ok(data) => println!("{}", json!({"ok":true,"data":data})),
        Err(error) => {
            println!("{}", json!({"ok":false,"error":error}));
            std::process::exit(1);
        }
    }
}
