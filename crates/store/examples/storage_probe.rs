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
        "state-root" => {
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
                &mut |_| Ok(()),
            )?;
            Ok(json!({"retired":true}))
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
    let result = io::stdin()
        .lock()
        .read_line(&mut line)
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
