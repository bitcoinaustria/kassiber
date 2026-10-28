//! Secrets commands bypass ordinary database bootstrap and schema migration.
use crate::argparse::Namespace;
use kassiber_envelope::{dict, s, set, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::{BookStore, RuntimePaths};
pub const COMMANDS: &[&str] = &[
    "secrets.init",
    "secrets.init-resume",
    "secrets.verify",
    "secrets.change-passphrase",
    "secrets.remember-unlock",
    "secrets.forget-unlock",
    "secrets.status",
    "secrets.migrate-credentials",
];
pub fn execute(
    store: &dyn BookStore,
    paths: &RuntimePaths,
    command: &str,
    args: &Namespace,
) -> Result<Value, AppError> {
    store.local_service(command, &bound_args(paths, args))
}

fn bound_args(paths: &RuntimePaths, args: &Namespace) -> Value {
    let mut bound = dict(args.items().iter().map(|(k, v)| (k.as_str(), v.clone())));
    // Bootstrap has resolved the user's selection. A later catalog selection
    // change must not redirect encryption to a different book.
    set(&mut bound, "data_root", s(paths.data_root.as_str()));
    set(&mut bound, "env_file", s(paths.env_file.as_str()));
    set(&mut bound, "project", Value::Null);
    bound
}

#[cfg(test)]
mod tests {
    use super::*;
    use kassiber_envelope::get;

    #[test]
    fn encryption_stays_bound_to_the_bootstrapped_book() {
        for explicit in [false, true] {
            let mut args = Namespace::default();
            if explicit {
                args.set("project", s("stale-project-name"));
                args.set("data_root", s("relative-root"));
                args.set("env_file", s("relative.env"));
            }
            args.set("new_passphrase_fd", Value::from(37));
            let paths = RuntimePaths {
                data_root: "/resolved/book/data".into(),
                env_file: "/resolved/book/config/backends.env".into(),
                ..RuntimePaths::default()
            };
            let bound = bound_args(&paths, &args);
            assert_eq!(get(&bound, "data_root"), Some(&s(paths.data_root.as_str())));
            assert_eq!(get(&bound, "env_file"), Some(&s(paths.env_file.as_str())));
            assert_eq!(get(&bound, "project"), Some(&Value::Null));
            assert_eq!(get(&bound, "new_passphrase_fd"), Some(&Value::from(37)));
        }
    }
}
