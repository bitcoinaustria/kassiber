//! What the operator broker needs from the CLI grammar
//! (`kassiber/operator/service.py` `_parse_argv`).
//!
//! The broker classifies a submitted command line with the same argparse
//! emulation the CLI runs, so admission, the child and a direct run agree on
//! which leaf a command is, which capability it needs, and which book-scope
//! destinations it declares (Python tests `hasattr(parsed, "workspace")`).

use crate::argparse::{parse_args, Context};
use kassiber_envelope::{command_path, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::capabilities::{cli_capability, Capability};

/// A classified command line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Classification {
    /// `command_path(args)`: `transactions.list`.
    pub command_path: String,
    pub capability: Capability,
    /// Whether the leaf declares `--workspace`, and its value when given.
    pub workspace: Option<Option<String>>,
    pub profile: Option<Option<String>>,
    /// `bool(getattr(parsed, "install", False))`.
    pub install: bool,
    /// The global project locators, empty strings included.
    pub data_root: Option<String>,
    pub project: Option<String>,
    pub env_file: Option<String>,
}

fn text(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::Str(text)) => Some(text.as_str().unwrap_or_default().to_owned()),
        _ => None,
    }
}

/// `_parse_argv(argv)`. A command line argparse refuses (or answers with
/// help or a version) is `operator_invalid_command`; a leaf without a
/// capability is `operator_unclassified_command`.
pub fn classify(argv: &[String]) -> Result<Classification, AppError> {
    let namespace = parse_args(
        argv,
        Context {
            width: 78,
            version: "",
        },
    )
    .map_err(|_| {
        AppError::new(
            "operator_invalid_command",
            "the broker received invalid command arguments",
        )
    })?;
    let path = command_path(|name| namespace.text(name));
    let Some(capability) = cli_capability(&path) else {
        return Err(AppError::new(
            "operator_unclassified_command",
            "the broker received an unclassified command",
        )
        .with_hint("Upgrade Kassiber or classify the command before brokered use.")
        .with_details(kassiber_envelope::dict([(
            "command",
            kassiber_envelope::s(path.as_str()),
        )])));
    };
    let scope = |name: &str| {
        namespace
            .has(name)
            .then(|| namespace.text(name).map(str::to_owned))
    };
    Ok(Classification {
        workspace: scope("workspace"),
        profile: scope("profile"),
        install: namespace
            .get("install")
            .is_some_and(kassiber_envelope::truthy),
        data_root: text(namespace.get("data_root")),
        project: text(namespace.get("project")),
        env_file: text(namespace.get("env_file")),
        command_path: path,
        capability,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    #[test]
    fn leaves_scope_and_locators_are_reported() {
        let parsed = classify(&argv(&[
            "--data-root",
            "/book",
            "--machine",
            "transactions",
            "list",
            "--workspace",
            "w",
        ]))
        .unwrap();
        assert_eq!(parsed.command_path, "transactions.list");
        assert_eq!(parsed.capability, Capability::Read);
        assert_eq!(parsed.workspace, Some(Some("w".into())));
        assert_eq!(parsed.profile, Some(None));
        assert_eq!(parsed.data_root.as_deref(), Some("/book"));
        assert_eq!(parsed.project, None);
        let status = classify(&argv(&["status"])).unwrap();
        assert_eq!((status.workspace, status.profile), (None, None));
        for refused in [
            &["--definitely-invalid"][..],
            &["--help"],
            &["--version"],
            &[],
        ] {
            assert_eq!(
                classify(&argv(refused)).unwrap_err().code,
                "operator_invalid_command",
                "{refused:?}"
            );
        }
        let install = classify(&argv(&["backup", "import", "a.kassiber", "--install"])).unwrap();
        assert!(install.install);
        assert_eq!(install.capability, Capability::Admin);
    }
}
