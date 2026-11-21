//! Bank-statement kinds (docs/reference/bank-statements.md). Evidence only:
//! no kind here writes transactions, journals or balances. Arguments are
//! read by the shared `Command::from_args`, as for the CLI.
use kassiber_envelope::{dict, s, type_name, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;
use kassiber_transactions::bank_statements::{Command, KINDS};

pub fn handles(kind: &str) -> bool {
    KINDS.iter().any(|(k, _)| *k == kind)
}

pub fn kinds() -> impl Iterator<Item = &'static str> {
    KINDS.iter().map(|(kind, _)| *kind)
}

pub fn execute(book: &mut dyn Book, kind: &str, args: Option<&Value>) -> Result<Value, AppError> {
    let empty = dict::<&str>([]);
    let args = args.filter(|v| !matches!(v, Value::Null)).unwrap_or(&empty);
    if !matches!(args, Value::Dict(_)) {
        return Err(AppError::new("validation", "daemon args must be an object")
            .with_details(dict([("type", s(type_name(args)))])));
    }
    book.bank_statements(None, None, &Command::from_args(kind, args)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kassiber_runtime::capabilities::{daemon_capability, Capability};

    #[test]
    fn kinds_are_supported_classified_and_validated() {
        let supported = crate::supported_kinds();
        for kind in kinds() {
            assert!(supported.contains(&kind), "{kind}");
            assert!(daemon_capability(kind).is_some(), "{kind}");
        }
        assert_eq!(
            daemon_capability("ui.source_funds.bank_lines.accept"),
            Some(Capability::AccountingDecisions)
        );
        assert_eq!(
            daemon_capability("ui.documents.rollback_statement"),
            Some(Capability::Admin)
        );
        let error =
            |kind: &str, args: Value| Command::from_args(kind, &args).err().unwrap().message;
        assert_eq!(
            error("ui.documents.import_statement", dict::<&str>([])),
            "ui.documents.import_statement requires args.file_path"
        );
        let confirm = dict([("import_id", s("I")), ("confirm", Value::Bool(true))]);
        assert_eq!(
            error("ui.documents.rollback_statement", confirm),
            "ui.documents.rollback_statement confirm must be \"DELETE\""
        );
        let empty = dict([
            ("line_ids", Value::List(vec![])),
            ("transaction_ids", Value::List(vec![s("T")])),
        ]);
        assert_eq!(
            error("ui.source_funds.bank_lines.accept", empty),
            "ui.source_funds.bank_lines.accept requires args.line_ids as a list of ids"
        );
    }
}
