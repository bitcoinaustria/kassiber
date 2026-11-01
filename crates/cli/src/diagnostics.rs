//! Diagnostics transport; report collection/redaction/export live in the service crate.
use crate::argparse::Namespace;
use kassiber_diagnostics::report;
use kassiber_envelope::{dict, s, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::{Book, Bootstrap};
pub fn arguments(args: &Namespace, bootstrap: Option<&Bootstrap>) -> Value {
    let mut result = dict(args.items().iter().map(|(k, v)| (k.as_str(), v.clone())));
    if let Some(bootstrap) = bootstrap {
        let p = &bootstrap.paths;
        for (key, value) in [
            ("data_root", s(p.data_root.as_str())),
            ("env_file", s(p.env_file.as_str())),
            (
                "project_id",
                p.project_id.as_deref().map(s).unwrap_or(Value::Null),
            ),
            (
                "project_root",
                p.project_root.as_deref().map(s).unwrap_or(Value::Null),
            ),
            (
                "project_selection_explicit",
                Value::Bool(bootstrap.selection_explicit),
            ),
        ] {
            kassiber_envelope::set(&mut result, key, value);
        }
    }
    result
}
pub fn execute(
    book: &mut dyn Book,
    args: &Namespace,
    bootstrap: &Bootstrap,
    version: &str,
    failure: &mut FailureContext,
) -> Result<Value, AppError> {
    let args = arguments(args, Some(bootstrap));
    let report = book.diagnostics(&args, version, None).inspect_err(|_| {
        failure.unhandled_type = Some("OperationalError");
    })?;
    let saved = if kassiber_envelope::get(&args, "save") == Some(&Value::Bool(true)) {
        report::save(&report, "auto", &args)
            .map_err(|e| {
                failure.unhandled_type = Some(kassiber_diagnostics::report::io_exception_type(&e));
                AppError::new("internal_error", e.to_string())
            })?
            .map(|v| v.payload())
            .unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    Ok(dict([("report", report), ("saved", saved)]))
}
pub fn write_error(
    args: &Value,
    version: &str,
    error: &AppError,
    report: Option<Result<Value, AppError>>,
    stderr: &mut dyn std::io::Write,
) {
    let Some(Value::Str(target)) = kassiber_envelope::get(args, "diagnostics_out") else {
        return;
    };
    let Some(target) = target.as_str().filter(|t| !t.is_empty()) else {
        return;
    };
    let result = (|| {
        let report = match report {
            Some(report) => report.map_err(|e| std::io::Error::other(e.message))?,
            None => report::collect(
                args,
                version,
                &report::now().map_err(std::io::Error::other)?,
                None,
                kassiber_diagnostics::state::checks(None),
                Some(error),
            ),
        };
        report::save(&report, target, args)
    })();
    match result {
        Ok(Some(saved)) => {
            let _ = writeln!(
                stderr,
                "diagnostics: wrote public report to {}",
                saved.location()
            );
        }
        Ok(None) => {}
        Err(error) => {
            let _ = writeln!(
                stderr,
                "diagnostics: failed to write public report: {}",
                kassiber_diagnostics::sanitize_text(&error.to_string())
            );
        }
    }
}

/// Error context retained only until this command completes; never logged.
pub struct FailureContext {
    pub args: Value,
    pub report: Option<Result<Value, AppError>>,
    pub unhandled_type: Option<&'static str>,
    pub journal_replay_shape: Option<String>,
}
impl FailureContext {
    pub fn annotate(&self, report: &mut Value) {
        if let Some(exception_type) = self.unhandled_type {
            kassiber_diagnostics::report::exception_identity(report, exception_type, true);
        }
    }
}
