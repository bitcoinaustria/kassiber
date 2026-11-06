//! `summarize_tool_call(tool, arguments)` (`kassiber/ai/tools.py:3731`): the
//! short consent text. Model-supplied strings are shown unredacted, as in
//! Python; this is local display only (design doc §7 item 12).

use kassiber_envelope::{get, truthy, AppError};
use kassiber_pyjson::{Key, PyStr, Value};

use crate::catalog::ToolEntry;
use crate::pyre;

/// Code points being assembled into a Python string.
#[derive(Default)]
struct Text(Vec<u32>);

impl Text {
    fn push(mut self, part: &str) -> Self {
        self.0.extend(part.chars().map(u32::from));
        self
    }

    fn py(mut self, part: &PyStr) -> Self {
        self.0.extend(part.code_points());
        self
    }

    fn done(self) -> PyStr {
        PyStr::from_code_points(self.0).expect("code points of strings")
    }
}

fn text(parts: &str) -> PyStr {
    PyStr::from(parts)
}

/// `value if isinstance(value, str) else None`.
fn string<'a>(arguments: &'a Value, key: &str) -> Option<&'a PyStr> {
    match get(arguments, key) {
        Some(Value::Str(value)) => Some(value),
        _ => None,
    }
}

/// `value.strip()` when `isinstance(value, str) and value.strip()`.
fn stripped(arguments: &Value, key: &str) -> Option<PyStr> {
    string(arguments, key)
        .map(kassiber_pytext::strip_py)
        .filter(|value| value.code_points().next().is_some())
}

fn list_len(value: Option<&Value>) -> usize {
    match value {
        Some(Value::List(items)) => items.len(),
        _ => 0,
    }
}

fn boolean(arguments: &Value, key: &str) -> Option<bool> {
    match get(arguments, key) {
        Some(Value::Bool(value)) => Some(*value),
        _ => None,
    }
}

fn toggle_verb(enabled: bool) -> &'static str {
    if enabled {
        "enable"
    } else {
        "disable"
    }
}

fn map_text(value: &PyStr, rule: impl FnOnce(&str) -> String) -> PyStr {
    pyre::with_surrogates(value, rule)
}

fn with_target(prefix: &str, target: Option<PyStr>, fallback: &str) -> PyStr {
    match target {
        Some(target) => Text::default().push(prefix).py(&target).done(),
        None => text(fallback),
    }
}

/// The `label` of a `new_wallet` object, else the `wallet` argument.
fn import_target(arguments: &Value) -> Option<PyStr> {
    let label = match get(arguments, "new_wallet") {
        Some(new_wallet @ Value::Dict(_)) => get(new_wallet, "label"),
        _ => get(arguments, "wallet"),
    };
    match label {
        Some(Value::Str(label)) => Some(kassiber_pytext::strip_py(label))
            .filter(|label| label.code_points().next().is_some()),
        _ => None,
    }
}

/// Python's name for an unhashable value's type.
fn unhashable(value: &Value) -> Option<&'static str> {
    match value {
        Value::List(_) => Some("list"),
        Value::Dict(_) => Some("dict"),
        _ => None,
    }
}

/// A component action other than `create`: what the consent card says.
fn component_effect(name: &str, arguments: &Value, action: &Value) -> Result<PyStr, AppError> {
    if let Some(kind) = unhashable(action) {
        // Python looks the action up in a dict before the schema is checked,
        // and the chat turn fails (tests/rust_port/repro_component_summary_unhashable.py).
        return Err(AppError::new(
            "internal_error",
            format!("unhashable type: '{kind}'"),
        ));
    }
    let component = match stripped(arguments, "component_id") {
        Some(target) => Text::default()
            .push("custody component ")
            .py(&target)
            .done(),
        None => text("custody component"),
    };
    let action = match action {
        Value::Str(action) => action.as_str(),
        _ => None,
    };
    let (prefix, suffix) = match action {
        Some("revise") if get(arguments, "activate") == Some(&Value::Bool(true)) => {
            ("Replace ", " with a new active revision")
        }
        Some("revise") => ("Save a draft revision of ", ""),
        Some("supersede") => ("Retire ", " (anchors return to raw booking)"),
        Some("undo") => ("Restore superseded ", " as a draft"),
        Some("activate") => ("Activate draft ", ""),
        _ => ("Change ", ""),
    };
    if name == "ui.transfers.components.plan" {
        let mut lowered = prefix.to_owned();
        lowered[..1].make_ascii_lowercase();
        return Ok(Text::default()
            .push("Preview: ")
            .push(&lowered)
            .py(&component)
            .push(suffix)
            .done());
    }
    Ok(Text::default()
        .push(prefix)
        .py(&component)
        .push(suffix)
        .done())
}

/// `summarize_tool_call(tool, arguments)`. An error where Python raises.
pub fn summarize_tool_call(tool: &ToolEntry, arguments: &Value) -> Result<PyStr, AppError> {
    let name = tool.name.as_str();
    Ok(match name {
        "ui.review.plan" | "ui.review.apply" => {
            let artifact = if name == "ui.review.apply" {
                get(arguments, "artifact")
            } else {
                Some(arguments)
            };
            let operations = match artifact {
                Some(artifact @ Value::Dict(_)) => get(artifact, "operations"),
                _ => None,
            };
            let verb = if name == "ui.review.apply" {
                "Apply"
            } else {
                "Preview"
            };
            text(&format!(
                "{verb} {} reviewed accounting change(s)",
                list_len(operations)
            ))
        }
        "ui.wallets.sync" => with_target(
            "Refresh source ",
            stripped(arguments, "wallet"),
            "Refresh all watch-only sources",
        ),
        "ui.journals.process" => text("Process journals"),
        "ui.wallets.import_attachment.apply" => with_target(
            "Import the attached file into ",
            import_target(arguments),
            "Import the attached file",
        ),
        "ui.imports.rollback" => text("Delete the rows an import run created"),
        "ui.wallets.import_document.apply" => with_target(
            "Import rows from the attached document into ",
            import_target(arguments),
            "Import rows from the attached document",
        ),
        "ui.attachments.add_from_chat" => {
            let what = match stripped(arguments, "file") {
                Some(file) => Text::default().push("attached ").py(&file).done(),
                None => text("the attached file"),
            };
            match stripped(arguments, "transaction") {
                Some(transaction) => Text::default()
                    .push("Keep ")
                    .py(&what)
                    .push(" as evidence on ")
                    .py(&transaction)
                    .done(),
                None => Text::default()
                    .push("Keep ")
                    .py(&what)
                    .push(" as evidence")
                    .done(),
            }
        }
        "ui.journals.quarantine.resolve" => {
            let transaction = stripped(arguments, "transaction")
                .or_else(|| {
                    stripped(arguments, "group_id")
                        .map(|group| Text::default().push("the root of ").py(&group).done())
                })
                .unwrap_or_else(|| text("quarantined transaction"));
            if matches!(get(arguments, "action"), Some(Value::Str(action)) if action.is("exclude"))
            {
                Text::default()
                    .push("Exclude ")
                    .py(&transaction)
                    .push(" from accounting")
                    .done()
            } else {
                Text::default()
                    .push("Apply reviewed price to ")
                    .py(&transaction)
                    .done()
            }
        }
        "ui.rates.rebuild" => with_target(
            "Fetch spot prices for ",
            stripped(arguments, "pair"),
            "Fetch missing spot prices and reprocess journals",
        ),
        "ui.rates.latest" => {
            let target =
                stripped(arguments, "pair").unwrap_or_else(|| text("the active book pair"));
            match stripped(arguments, "source") {
                Some(source) => Text::default()
                    .push("Fetch latest ")
                    .py(&target)
                    .push(" rate from ")
                    .py(&source)
                    .done(),
                None => Text::default()
                    .push("Fetch latest ")
                    .py(&target)
                    .push(" market rate")
                    .done(),
            }
        }
        "ui.maintenance.configure" => {
            // Every accepted argument appears, so a prompt never describes
            // the opposite of the change it asks about.
            let mut parts: Vec<PyStr> = Vec::new();
            let report_read = match arguments {
                Value::Dict(_) if get(arguments, "report_read_sync").is_some() => {
                    get(arguments, "report_read_sync")
                }
                _ => get(arguments, "auto_sync_before_report_reads"),
            };
            if let Some(Value::Bool(enabled)) = report_read {
                parts.push(text(&format!(
                    "{} freshness refresh before report reads",
                    toggle_verb(*enabled)
                )));
            }
            for (key, label) in [
                ("background_enabled", "daemon-owned background refresh"),
                (
                    "bitcoin_rail_carrying_value",
                    "the carrying-value default for BTC/L-BTC rail changes",
                ),
            ] {
                if let Some(enabled) = boolean(arguments, key) {
                    parts.push(text(&format!("{} {label}", toggle_verb(enabled))));
                }
            }
            if let Some(Value::Dict(classes)) = get(arguments, "source_classes") {
                let mut classes: Vec<(&Key, &Value)> =
                    classes.iter().map(|(k, v)| (k, v)).collect();
                classes.sort_by(|(a, _), (b, _)| match (a, b) {
                    (Key::Str(a), Key::Str(b)) => a.cmp(b),
                    _ => std::cmp::Ordering::Equal,
                });
                for (key, value) in classes {
                    let Key::Str(class) = key else { continue };
                    parts.push(
                        Text::default()
                            .push(toggle_verb(truthy(value)))
                            .push(" refresh for ")
                            .py(class)
                            .done(),
                    );
                }
            }
            if let Some(provider) = stripped(arguments, "market_rate_provider") {
                parts.push(
                    Text::default()
                        .push("set the market-rate provider to ")
                        .py(&provider)
                        .done(),
                );
            }
            if parts.is_empty() {
                return Ok(text("Change freshness policy"));
            }
            let mut out = Text::default().push("Change freshness policy: ");
            for (index, part) in parts.iter().enumerate() {
                if index > 0 {
                    out = out.push("; ");
                }
                out = out.py(part);
            }
            out.done()
        }
        "ui.maintenance.run" => {
            let mode = match arguments {
                Value::Dict(_) if get(arguments, "sync").is_some() => {
                    get(arguments, "sync").cloned()
                }
                _ => Some(Value::Str(text("if_enabled"))),
            };
            match mode {
                Some(Value::Str(mode)) if mode.is("always") => {
                    text("Refresh sources and process journals")
                }
                Some(Value::Str(mode)) if mode.is("never") => {
                    text("Process journals without source refresh")
                }
                _ => text("Run maintenance using current settings"),
            }
        }
        "ui.source_funds.sources.create" => with_target(
            "Create source-funds source ",
            stripped(arguments, "label"),
            "Create source-funds source",
        ),
        "ui.source_funds.links.create" => match stripped(arguments, "to_transaction") {
            Some(target) => {
                let label = stripped(arguments, "link_type").unwrap_or_else(|| text("link"));
                Text::default()
                    .push("Create ")
                    .py(&label)
                    .push(" source-funds link to ")
                    .py(&target)
                    .done()
            }
            None => text("Create source-funds link"),
        },
        "ui.source_funds.links.review" => match stripped(arguments, "link") {
            Some(link) => match stripped(arguments, "state") {
                Some(state) => Text::default()
                    .push("Mark source-funds link ")
                    .py(&link)
                    .push(" as ")
                    .py(&state)
                    .done(),
                None => Text::default()
                    .push("Review source-funds link ")
                    .py(&link)
                    .done(),
            },
            None => text("Review source-funds link"),
        },
        "ui.source_funds.suggest" => with_target(
            "Seed source-funds suggestions for ",
            stripped(arguments, "target_transaction"),
            "Seed source-funds suggestions",
        ),
        "ui.source_funds.links.bulk_review" => with_target(
            "Review deterministic source-funds suggestions for ",
            stripped(arguments, "target_transaction"),
            "Review deterministic source-funds suggestions",
        ),
        "ui.source_funds.sources.attach" => with_target(
            "Attach existing evidence to source ",
            stripped(arguments, "source"),
            "Attach existing evidence to source",
        ),
        "ui.source_funds.links.attach" => with_target(
            "Attach existing evidence to link ",
            stripped(arguments, "link"),
            "Attach existing evidence to source-funds link",
        ),
        "ui.source_funds.assemble" => with_target(
            "Assemble source-funds history for ",
            stripped(arguments, "target_transaction"),
            "Assemble source-funds history",
        ),
        "ui.transactions.metadata.update" => with_target(
            "Update reviewed metadata for ",
            stripped(arguments, "transaction"),
            "Update transaction metadata",
        ),
        "ui.transactions.history.revert" => with_target(
            "Revert an audited edit on ",
            stripped(arguments, "transaction"),
            "Revert transaction edit",
        ),
        "ui.loans.mark" => match (stripped(arguments, "txid"), string(arguments, "as")) {
            (Some(target), Some(role)) => Text::default()
                .push("Mark ")
                .py(&target)
                .push(" as loan ")
                .py(role)
                .done(),
            _ => text("Mark loan transaction"),
        },
        "ui.loans.link" => match list_len(get(arguments, "txids")) {
            0 => text("Link loan transactions"),
            count => text(&format!("Link {count} loan transactions")),
        },
        "ui.loans.unmark" => with_target(
            "Remove loan mark from ",
            stripped(arguments, "txid"),
            "Remove loan transaction mark",
        ),
        "ui.transfers.payouts.create" => match (
            stripped(arguments, "tx_out"),
            stripped(arguments, "payout_asset"),
            stripped(arguments, "payout_amount"),
        ) {
            (Some(target), Some(asset), Some(amount)) => Text::default()
                .push("Record ")
                .py(&amount)
                .push(" ")
                .py(&asset)
                .push(" direct payout for ")
                .py(&target)
                .done(),
            _ => text("Create direct swap payout"),
        },
        "ui.transfers.payouts.delete" => with_target(
            "Delete direct payout ",
            stripped(arguments, "payout_id"),
            "Delete direct swap payout",
        ),
        "ui.transfers.update" => with_target(
            "Update reviewed transfer pair ",
            stripped(arguments, "pair_id"),
            "Update reviewed transfer pair",
        ),
        "ui.transfers.components.plan" | "ui.transfers.components.apply" => {
            let create = Value::Str(text("create"));
            let action = get(arguments, "action").unwrap_or(&create);
            if !matches!(action, Value::Str(action) if action.is("create")) {
                return component_effect(name, arguments, action);
            }
            let count = list_len(get(arguments, "components"));
            let noun = if count == 1 {
                "component"
            } else {
                "components"
            };
            let count = if count == 0 {
                "custody".to_owned()
            } else {
                count.to_string()
            };
            if name == "ui.transfers.components.plan" {
                text(&format!("Preview {count} gap-resolution {noun}"))
            } else {
                text(&format!("Create {count} gap-resolution {noun}"))
            }
        }
        "ui.reports.export" => match (string(arguments, "report"), string(arguments, "format")) {
            (Some(report), Some(format)) => Text::default()
                .push("Export ")
                .py(&map_text(report, |report| report.replace('_', " ")))
                .push(" as ")
                .py(&map_text(format, kassiber_pytext::upper))
                .done(),
            _ => text("Export report artifact"),
        },
        "ui.source_funds.export" => match string(arguments, "format") {
            Some(format) => Text::default()
                .push("Export source-funds ")
                .py(format)
                .done(),
            None => text("Export source-funds case"),
        },
        _ => match tool.summary_template.as_deref() {
            Some(template) if !template.is_empty() => text(template),
            _ => text(name),
        },
    })
}
