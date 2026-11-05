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
    "ui.chain_analysis.ai_context",
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
    let result = book.chain_analysis_reads(
        None,
        None,
        operation,
        args,
        &mut Local {
            runtime: Some(Arc::clone(runtime)),
            ..Local::default()
        },
    )?;
    if operation != "ai_context" {
        return Ok(result);
    }
    // The assistant receives the investigation through `ca-ref:` handles
    // only (`project_ai_result`), whichever provider the user picks next.
    let (_, profile) = book.resolve_scope(None, None)?;
    kassiber_ai::analysis::project_ai_result(&result, &mut |value| {
        book.chain_analysis_issue_ai_references(&profile, &[value.to_owned()])?
            .into_iter()
            .next()
            .ok_or(kassiber_ai::ToolError::Unexpected)
    })
    .map_err(|error| match error {
        kassiber_ai::ToolError::App(error) => error,
        _ => AppError::new(
            "internal_error",
            "chain-analysis AI references are unavailable",
        ),
    })
}
