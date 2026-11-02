use kassiber_envelope::{dict, s, type_name, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;
use kassiber_wallets::analysis_index::{reads::Local, runtime::Runtime};
use std::sync::Arc;

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
    "ui.chain_analysis.psbt.analyze",
    "ui.chain_analysis.psbt.compare",
    "ui.chain_analysis.psbt.entropy",
    "ui.chain_analysis.entropy.start",
    "ui.chain_analysis.psbt.entropy.start",
    "ui.chain_analysis.jobs.get",
    "ui.chain_analysis.jobs.cancel",
    "internal.chain_analysis.stage",
    "ui.chain_analysis.datasets.preview",
    "ui.chain_analysis.datasets.preview.start",
];

pub fn execute(
    book: &mut dyn Book,
    runtime: &Arc<Runtime>,
    kind: &str,
    args: Option<&Value>,
) -> Result<Value, kassiber_envelope::AppError> {
    let empty = dict::<&str>([]);
    let args = args.filter(|v| **v != Value::Null).unwrap_or(&empty);
    if !matches!(args, Value::Dict(_)) {
        return Err(AppError::new("validation", "daemon args must be an object")
            .with_details(dict([("type", s(type_name(args)))])));
    }
    let operation = match kind {
        "internal.chain_analysis.stage" => "internal.stage",
        kind => kind.trim_start_matches("ui.chain_analysis."),
    };
    book.chain_analysis_reads(
        None,
        None,
        operation,
        args,
        &mut Local {
            runtime: Some(Arc::clone(runtime)),
            ..Local::default()
        },
    )
}
