//! Backup commands resolve the book once, then use the archive service.
use crate::argparse::Namespace;
use kassiber_envelope::{dict, s, set, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::{BookStore, RuntimePaths};
pub const COMMANDS: &[&str] = &["backup.export", "backup.import"];

pub fn execute(
    store: &dyn BookStore,
    paths: &RuntimePaths,
    command: &str,
    args: &Namespace,
) -> Result<Value, AppError> {
    let mut bound = dict(args.items().iter().map(|(k, v)| (k.as_str(), v.clone())));
    set(&mut bound, "data_root", s(paths.data_root.as_str()));
    set(&mut bound, "env_file", s(paths.env_file.as_str()));
    set(&mut bound, "project", Value::Null);
    store.local_service(command, &bound)
}
