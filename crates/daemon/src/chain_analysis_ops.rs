use kassiber_envelope::{dict, s, type_name, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;

pub const KINDS: &[&str] = &[
    "ui.chain_analysis.cases.delete",
    "ui.chain_analysis.cases.save",
    "ui.chain_analysis.datasets.discard",
    "ui.chain_analysis.datasets.revoke",
    "ui.chain_analysis.labels.delete",
    "ui.chain_analysis.labels.import",
    "ui.chain_analysis.labels.upsert",
];

pub fn execute(book: &mut dyn Book, kind: &str, args: Option<&Value>) -> Result<Value, AppError> {
    let args = args.unwrap_or(&Value::Null);
    if !matches!(args, Value::Null | Value::Dict(_)) {
        return Err(AppError::new("validation", "daemon args must be an object")
            .with_details(dict([("type", s(type_name(args)))])));
    }
    book.chain_analysis_ops(
        None,
        None,
        kind.strip_prefix("ui.chain_analysis.").unwrap(),
        args,
    )
}
