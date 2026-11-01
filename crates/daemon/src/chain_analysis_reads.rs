use kassiber_envelope::{dict, s, type_name, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;

pub const KINDS: &[&str] = &[
    "ui.chain_analysis.query",
    "ui.chain_analysis.entropy",
    "ui.chain_analysis.datasets.list",
    "ui.chain_analysis.datasets.get",
    "ui.chain_analysis.datasets.query",
    "ui.chain_analysis.cases.list",
    "ui.chain_analysis.cases.get",
    "ui.chain_analysis.cases.compare",
    "ui.chain_analysis.labels.list",
];

pub fn execute(
    book: &mut dyn Book,
    kind: &str,
    args: Option<&Value>,
) -> Result<Value, kassiber_envelope::AppError> {
    let empty = dict::<&str>([]);
    let args = args.filter(|v| **v != Value::Null).unwrap_or(&empty);
    if !matches!(args, Value::Dict(_)) {
        return Err(AppError::new("validation", "daemon args must be an object")
            .with_details(dict([("type", s(type_name(args)))])));
    }
    book.chain_analysis_reads(
        None,
        None,
        kind.trim_start_matches("ui.chain_analysis."),
        args,
    )
}
