//! Explicit development-only import interruption probe; never packaged.
use kassiber_envelope::{dict, get, py_str, s, AppError};
use kassiber_pyjson::{dumps, Options, Value};
use std::{
    io::{self, Write},
    path::Path,
};

fn run(request: &Value) -> Result<Value, AppError> {
    let field = |name| get(request, name).map(py_str).unwrap_or_default();
    let data = field("data_root");
    let owners = field("owners");
    let archive = field("archive");
    let fd = get(request, "fd")
        .and_then(|v| match v {
            Value::Int(n) => i32::try_from(n.clone()).ok(),
            _ => None,
        })
        .ok_or_else(|| AppError::new("validation", "probe descriptor required"))?;
    let pass = kassiber_secrets::read_passphrase_from_fd(fd)
        .map_err(kassiber_secrets::service::storage_error)?;
    let namespace = kassiber_ownership::OwnerNamespace::isolated(owners.into());
    let _maintenance = kassiber_store::restore::maintenance(&namespace, Path::new(&data), None)
        .map_err(kassiber_secrets::service::storage_error)?;
    let result = kassiber_backup::import_with_observer(
        Path::new(&archive),
        Some(pass.expose()),
        None,
        &mut |point| {
            if field("kill_at") == point {
                println!(
                    "{}",
                    dumps(&dict([("checkpoint", s(point))]), &Options::default()).unwrap()
                );
                io::stdout()
                    .flush()
                    .map_err(|e| AppError::new("internal_error", e.to_string()))?;
                let mut line = String::new();
                io::stdin()
                    .read_line(&mut line)
                    .map_err(|e| AppError::new("internal_error", e.to_string()))?;
            }
            Ok(())
        },
    )?;
    let recovery = kassiber_store::restore::install(&result.staging(), Path::new(&data))
        .map_err(kassiber_secrets::service::storage_error)?;
    Ok(dict([(
        "recovery",
        recovery.map_or(Value::Null, |p| s(p.to_string_lossy().as_ref())),
    )]))
}
fn main() {
    let mut line = String::new();
    io::stdin().read_line(&mut line).unwrap();
    let request = kassiber_pytext::loads(&line).unwrap();
    match run(&request) {
        Ok(data) => println!("{}", dumps(&data, &Options::default()).unwrap()),
        Err(error) => {
            eprintln!("{}", error.message);
            std::process::exit(1);
        }
    }
}
