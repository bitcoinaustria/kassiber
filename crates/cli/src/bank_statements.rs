//! Bank statements as source-of-funds evidence: `documents import-statement`,
//! `documents statement-imports`, `documents rollback-statement` and
//! `source-funds bank-lines ...`. Each command is the daemon kind of the same
//! name; its options are passed under the daemon's argument names.
use crate::argparse::Namespace;
use kassiber_envelope::{dict, s, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;
use kassiber_transactions::bank_statements::{Command, KINDS};

/// The daemon kind a CLI command runs, if it is a bank-statement command.
pub fn kind(path: &str) -> Option<&'static str> {
    KINDS
        .iter()
        .find(|(_, cli)| *cli == path)
        .map(|(kind, _)| *kind)
}

/// The command's options under the daemon's argument names.
fn args(ns: &Namespace) -> Value {
    const NAMES: &[(&str, &str)] = &[
        ("file", "file_path"),
        ("statement_format", "format"),
        ("column_map", "column_map"),
        ("currency", "currency"),
        ("account", "account"),
        ("label", "label"),
        ("dry_run", "dry_run"),
        ("import", "import_id"),
        ("direction", "direction"),
        ("document", "document"),
        ("limit", "limit"),
        ("line", "line_ids"),
    ];
    let mut pairs: Vec<(&str, Value)> = NAMES
        .iter()
        .filter_map(|(option, name)| ns.get(option).map(|v| (*name, v.clone())))
        .collect();
    if ns.flag("confirm") {
        pairs.push(("confirm", s("DELETE")));
    }
    match ns.get("transaction") {
        Some(list @ Value::List(_)) => pairs.push(("transaction_ids", list.clone())),
        Some(one) => pairs.push(("transaction", one.clone())),
        None => {}
    }
    dict(pairs)
}

pub fn execute(book: &mut dyn Book, kind: &str, ns: &Namespace) -> Result<Value, AppError> {
    let command = Command::from_args(kind, &args(ns))?;
    book.bank_statements(ns.get("workspace"), ns.get("profile"), &command)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(argv: &[&str]) -> Result<Command, AppError> {
        let argv: Vec<String> = argv.iter().map(|a| a.to_string()).collect();
        let cx = crate::argparse::Context {
            width: 78,
            version: "test",
        };
        let ns = crate::argparse::parse_args(&argv, cx).unwrap();
        let path = format!(
            "{}.{}",
            argv[0],
            argv[1..]
                .iter()
                .take_while(|a| !a.starts_with('-'))
                .cloned()
                .collect::<Vec<_>>()
                .join(".")
        );
        Command::from_args(kind(&path).expect("bank command"), &args(&ns))
    }

    #[test]
    fn cli_options_reach_the_shared_command() {
        let import = [
            "documents",
            "import-statement",
            "--file",
            "s.csv",
            "--format",
            "csv",
            "--currency",
            "eur",
            "--dry-run",
            "--account",
            "AT61",
        ];
        let Ok(Command::Import(request)) = command(&import) else {
            panic!("import")
        };
        assert_eq!((request.file.as_str(), request.dry_run), ("s.csv", true));
        assert_eq!(request.options.currency.as_deref(), Some("eur"));
        let accept = [
            "source-funds",
            "bank-lines",
            "accept",
            "--line",
            "L1",
            "--line",
            "L2",
            "--transaction",
            "T1",
        ];
        let Ok(Command::Accept {
            lines,
            transactions,
        }) = command(&accept)
        else {
            panic!("accept")
        };
        assert_eq!(
            (lines, transactions),
            (vec!["L1".to_owned(), "L2".into()], vec!["T1".to_owned()])
        );
        let Ok(Command::Rollback { confirm, .. }) = command(&[
            "documents",
            "rollback-statement",
            "--import",
            "I",
            "--confirm",
        ]) else {
            panic!("rollback")
        };
        assert!(confirm);
        let Ok(Command::Suggest { transaction }) = command(&[
            "source-funds",
            "bank-lines",
            "suggest",
            "--transaction",
            "T",
        ]) else {
            panic!("suggest")
        };
        assert_eq!(transaction.as_deref(), Some("T"));
        for (_, path) in KINDS {
            assert!(crate::describe::IMPLEMENTED.contains(path), "{path}");
            assert!(crate::spec::rust_only_new(path), "{path}");
        }
    }
}
