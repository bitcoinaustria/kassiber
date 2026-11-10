//! Disposable lifecycle crash probe. Never installed or packaged.
use kassiber_envelope::AppError;
use kassiber_ownership::OwnerNamespace;
use kassiber_paths::{Context, ResolveOptions};
use kassiber_secrets::{
    credentials, policy, service, CredentialReader, CredentialStore, NativeCredentialReader,
    Passphrase,
};
use serde_json::{json, Value};
use std::{
    io::{self, Write},
    path::Path,
};
struct ProbeStore {
    refuse_write: bool,
}
impl CredentialReader for ProbeStore {
    fn get_password(
        &self,
        service: &str,
        account: &str,
    ) -> kassiber_secrets::Result<Option<Passphrase>> {
        NativeCredentialReader.get_password(service, account)
    }
}
impl CredentialStore for ProbeStore {
    fn available(&self) -> bool {
        NativeCredentialReader.available()
    }
    fn set_password(
        &self,
        service: &str,
        account: &str,
        secret: &Passphrase,
    ) -> kassiber_secrets::Result<()> {
        if self.refuse_write {
            return Err(kassiber_paths::Error::new(
                kassiber_paths::ErrorKind::SecretStoreUnavailable,
                "probe refused refresh",
            ));
        }
        NativeCredentialReader.set_password(service, account, secret)
    }
    fn delete_password(&self, service: &str, account: &str) -> kassiber_secrets::Result<()> {
        NativeCredentialReader.delete_password(service, account)
    }
}
fn observe(request: &Value, label: &str) -> Result<(), AppError> {
    if request["kill_at"].as_str() == Some(label) {
        println!("{}", json!({"checkpoint": label}));
        io::stdout().flush().unwrap();
        let mut release = String::new();
        io::stdin().read_line(&mut release).unwrap();
    }
    Ok(())
}
fn run(request: &Value) -> Result<kassiber_pyjson::Value, AppError> {
    let root = Path::new(request["data_root"].as_str().unwrap());
    let namespace = OwnerNamespace::isolated(request["owners"].as_str().unwrap().into());
    let paths = Context::from_env()
        .unwrap()
        .resolve_for_bootstrap(&ResolveOptions {
            data_root: Some(root.into()),
            env_file: request["env_file"].as_str().map(Into::into),
            ..Default::default()
        })
        .map_err(service::storage_error)?;
    let secret = kassiber_secrets::read_passphrase_from_fd(request["fd"].as_i64().unwrap() as i32)
        .map_err(service::storage_error)?;
    match request["op"].as_str().unwrap() {
        "credentials" => {
            let conn =
                kassiber_store::encryption::open_container(&paths.database, Some(secret.expose()))
                    .map_err(service::storage_error)?;
            credentials::migrate_observed(&conn, &paths.env_file, &mut |label| {
                observe(request, label)
            })
        }
        "remember" => policy::enroll_observed(
            &namespace,
            root,
            &secret,
            &NativeCredentialReader,
            &mut |label| observe(request, label),
        ),
        "forget" => {
            let book = kassiber_store::Book::open(
                root,
                kassiber_store::OpenOptions {
                    passphrase: Some(secret.expose()),
                    ..Default::default()
                },
            )
            .map_err(service::storage_error)?;
            drop(book);
            policy::forget_observed(
                &namespace,
                root,
                &NativeCredentialReader,
                false,
                &mut |label| observe(request, label),
            )
        }
        "rotate" | "rotate-failed-refresh" => {
            let new = kassiber_secrets::read_passphrase_from_fd(
                request["new_fd"].as_i64().unwrap() as i32,
            )
            .map_err(service::storage_error)?;
            service::rotate_observed(
                &namespace,
                &paths,
                &secret,
                &new,
                &ProbeStore {
                    refuse_write: request["op"] == "rotate-failed-refresh",
                },
                None,
                &mut |label| {
                    observe(request, label).map_err(|_| {
                        kassiber_paths::Error::new(
                            kassiber_paths::ErrorKind::Io,
                            "probe observation failed",
                        )
                    })
                },
            )
        }
        _ => Err(AppError::new("validation", "unknown probe operation")),
    }
}
fn main() {
    let mut line = String::new();
    io::stdin().read_line(&mut line).unwrap();
    let request: Value = serde_json::from_str(&line).unwrap();
    match run(&request) {
        Ok(data) => println!(
            "{}",
            kassiber_pyjson::dumps(&data, &Default::default()).unwrap()
        ),
        Err(error) => {
            println!(
                "{}",
                kassiber_pyjson::dumps(&error.envelope(None), &Default::default()).unwrap()
            );
            std::process::exit(1);
        }
    }
}
