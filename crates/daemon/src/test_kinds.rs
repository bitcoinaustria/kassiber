//! Kinds that exist only in test builds (feature `test-kinds`), so the
//! protocol machinery R1 ships without a business kind that uses it can be
//! driven end to end: streaming records from another thread, unsolicited
//! events, a record for a request id nobody sent, and a secret-store
//! control round trip. They are not classified capabilities and never
//! appear in a release binary.

use std::time::Duration;

use kassiber_envelope::{dict, get, success, AppError};
use kassiber_pyjson::Value;

use crate::control::{secret_store_request, SecretStoreRequest, SECRET_STORE_BRIDGE_TIMEOUT};
use crate::input::Input;
use crate::output::{with_request_id, RequestId};
use crate::{Daemon, Failure, Handled};

pub const KINDS: [&str; 21] = [
    "test.stream",
    "test.journals.refresh",
    "test.emit_event",
    "test.late",
    "test.secret_store",
    "test.freshness.plan",
    "test.freshness.select",
    "test.freshness.invoke",
    "test.freshness.execute",
    "test.freshness.product.execute",
    "test.freshness.cancel",
    "test.freshness.pause",
    "test.freshness.resume",
    "test.freshness.request.cancel",
    "test.freshness.request.revoke",
    "test.freshness.graph.pending",
    "test.freshness.graph.start",
    "test.ai_tool",
    "test.ai_auto_read",
    "test.ai_consent_preview",
    "test.ai_custody_record",
];

fn arg_text(args: Option<&Value>, key: &str) -> String {
    match args.and_then(|args| get(args, key)) {
        Some(Value::Str(text)) => text.as_str().unwrap_or_default().to_owned(),
        _ => String::new(),
    }
}

fn arg_number(args: Option<&Value>, key: &str, default: u64) -> u64 {
    match args.and_then(|args| get(args, key)) {
        Some(Value::Int(n)) => u64::try_from(n.clone()).unwrap_or(default),
        _ => default,
    }
}

pub(crate) fn handle(
    daemon: &mut Daemon,
    kind: &kassiber_pyjson::PyStr,
    request: &Value,
    request_id: &RequestId,
    input: &mut Input,
) -> Option<Handled> {
    let kind = kind.as_str()?;
    if !KINDS.contains(&kind) {
        return None;
    }
    let args = get(request, "args");
    Some(match kind {
        // The journal step of the assistant's scoped `ui.journals.process`
        // tool: Python's `_journals_refresh_payload(conn, auto_pair=False)`
        // on the daemon's own connection. The chat loop that owns its
        // consent and scope binding is not native yet.
        "test.journals.refresh" => (|| {
            if let Some(locked) = daemon.open_lazily(request_id)? {
                return Ok((Some(locked), false));
            }
            let auto_pair = matches!(
                args.and_then(|v| get(v, "auto_pair")),
                Some(Value::Bool(true))
            );
            let context = daemon.book_mut()?.context()?;
            let output = daemon.output.clone();
            let cancel = kassiber_wallets::scan::Cancellation::default();
            let result =
                daemon
                    .book_mut()?
                    .refresh_journals(&context, auto_pair, &cancel, &mut |_| Ok(()))?;
            output
                .write(&with_request_id(
                    success("ui.journals.process", result.payload().clone()),
                    request_id,
                ))
                .map_err(|_| AppError::new("cancelled", "Journal output closed"))?;
            result.after_reply()?;
            Ok((None, false))
        })(),
        "test.freshness.select" | "test.freshness.invoke" => (|| {
            let product_kind = arg_text(args, "kind");
            if !crate::freshness::product::RUN_KINDS.contains(&product_kind.as_str()) {
                return Err(
                    AppError::new("validation", "Expected a freshness product kind").into(),
                );
            }
            let product_args = args.and_then(|args| get(args, "args"));
            let product_id = if let Some(request) = args.and_then(|args| get(args, "request")) {
                RequestId::of(request)
            } else {
                args.and_then(|args| get(args, "request_id"))
                    .map_or(request_id.clone(), |id| RequestId::Value(id.clone()))
            };
            if kind == "test.freshness.invoke" {
                daemon.freshness_before_request(&product_kind)?;
                return crate::freshness::product::handle(
                    daemon,
                    &product_kind,
                    product_args,
                    &product_id,
                );
            }
            let supported = crate::freshness::product::supported(
                daemon,
                &product_kind,
                product_args,
                &product_id,
            )?;
            Ok((
                Some(with_request_id(
                    success(kind, dict([("supported", Value::Bool(supported))])),
                    request_id,
                )),
                false,
            ))
        })(),
        "test.freshness.plan" | "test.freshness.execute" | "test.freshness.product.execute" => {
            (|| {
                use kassiber_wallets::freshness::planning::{Request, Scope};
                if let Some(locked) = daemon.open_lazily(request_id)? {
                    return Ok((Some(locked), false));
                }
                if let Some(locked) = daemon.check_expected_scope(
                    args.filter(|v| get(v, "expected_scope").is_some()),
                    request_id,
                )? {
                    return Ok((Some(locked), false));
                }
                let workspace = arg_text(args, "workspace_id");
                let wallet = arg_text(args, "wallet");
                let flag = |key| {
                    args.and_then(|v| get(v, key)).and_then(|v| match v {
                        Value::Bool(value) => Some(*value),
                        _ => None,
                    })
                };
                let selection = Request {
                    scope: if workspace.is_empty() {
                        Scope::ActiveProfile
                    } else {
                        Scope::Workspace(workspace)
                    },
                    wallet: (!wallet.is_empty()).then_some(wallet),
                    rates: flag("rates"),
                    journals: flag("journals"),
                    force_full: flag("force_full").unwrap_or(false),
                    auto_pair: flag("auto_pair").unwrap_or(false),
                    wallets_sync: false,
                };
                if kind != "test.freshness.plan" {
                    daemon.start_freshness(
                        if kind == "test.freshness.product.execute" {
                            "ui.freshness.run"
                        } else {
                            kind
                        },
                        request_id,
                        &selection,
                        kassiber_wallets::freshness::execution::Options {
                            run: flag("run").unwrap_or(true),
                            limit: None,
                        },
                        args.unwrap_or(&dict::<&str>([])),
                    )?;
                    return Ok((None, false));
                }
                daemon.freshness_before_request(kind)?;
                let payload = daemon
                    .book_mut()?
                    .freshness_plan(&selection)?
                    .specifications();
                Ok((
                    Some(with_request_id(success(kind, payload), request_id)),
                    false,
                ))
            })()
        }
        "test.freshness.graph.start" => (|| {
            daemon.start_graph()?;
            Ok((
                Some(with_request_id(success(kind, dict::<&str>([])), request_id)),
                false,
            ))
        })(),
        "test.freshness.graph.pending" => Ok((
            Some(with_request_id(
                success(
                    kind,
                    dict([(
                        "grants",
                        Value::from(daemon.freshness_requests.graph_pending()),
                    )]),
                ),
                request_id,
            )),
            false,
        )),
        "test.freshness.request.cancel" | "test.freshness.request.revoke" => (|| {
            if let Some(locked) = daemon.check_expected_scope(
                args.filter(|v| get(v, "expected_scope").is_some()),
                request_id,
            )? {
                return Ok((Some(locked), false));
            }
            daemon.reap_freshness(false)?;
            let payload = daemon
                .freshness_requests
                .control(args.unwrap_or(&dict::<&str>([])), kind.ends_with("revoke"))?;
            Ok((
                Some(with_request_id(success(kind, payload), request_id)),
                false,
            ))
        })(),
        "test.freshness.cancel" | "test.freshness.pause" | "test.freshness.resume" => (|| {
            if let Some(locked) = daemon.open_lazily(request_id)? {
                return Ok((Some(locked), false));
            }
            if let Some(locked) = daemon.check_expected_scope(
                args.filter(|v| get(v, "expected_scope").is_some()),
                request_id,
            )? {
                return Ok((Some(locked), false));
            }
            let action = kind.rsplit('.').next().unwrap_or("");
            let payload = daemon
                .book_mut()?
                .freshness_control(action, args.unwrap_or(&dict::<&str>([])))?;
            Ok((
                Some(with_request_id(success(kind, payload), request_id)),
                false,
            ))
        })(),
        // `count` deltas `delay_ms` apart from a worker thread, then the
        // terminal record; the request loop keeps serving meanwhile.
        "test.stream" => {
            let count = arg_number(args, "count", 2);
            let delay = Duration::from_millis(arg_number(args, "delay_ms", 50));
            let output = daemon.output.clone();
            let request_id = request_id.clone();
            std::thread::spawn(move || {
                for index in 0..count {
                    std::thread::sleep(delay);
                    let delta = success("test.stream.delta", dict([("index", Value::from(index))]));
                    if output.write(&with_request_id(delta, &request_id)).is_err() {
                        return;
                    }
                }
                let done = success("test.stream", dict([("count", Value::from(count))]));
                let _ = output.write(&with_request_id(done, &request_id));
            });
            Ok((None, false))
        }
        "test.emit_event" => (|| {
            daemon
                .output
                .event("test.event", dict([("n", Value::from(1))]))?;
            Ok((
                Some(with_request_id(
                    success("test.emit_event", dict([("ok", Value::Bool(true))])),
                    request_id,
                )),
                false,
            ))
        })(),
        // A straggler for a request the supervisor never sent, then the
        // real answer.
        "test.late" => (|| {
            let ghost = success("test.late.delta", dict([("late", Value::Bool(true))]));
            daemon.output.write(&with_request_id(
                ghost,
                &RequestId::Value(kassiber_envelope::s("ghost-1")),
            ))?;
            Ok((
                Some(with_request_id(
                    success("test.late", dict([("ok", Value::Bool(true))])),
                    request_id,
                )),
                false,
            ))
        })(),
        "test.secret_store" => {
            let op = arg_text(args, "op");
            let provider_name = arg_text(args, "provider_name");
            let store_id = arg_text(args, "store_id");
            let service = arg_text(args, "service");
            let account = arg_text(args, "account");
            let timeout = Duration::from_millis(arg_number(
                args,
                "timeout_ms",
                u64::try_from(SECRET_STORE_BRIDGE_TIMEOUT.as_millis()).unwrap_or(15_000),
            ));
            let bridge = SecretStoreRequest {
                op: if op.is_empty() { "get" } else { &op },
                provider_name: &provider_name,
                store_id: &store_id,
                service: &service,
                account: &account,
                secret: None,
            };
            match secret_store_request(&daemon.output, input, &bridge, timeout) {
                Ok(data) => Ok((
                    Some(with_request_id(
                        success("test.secret_store", data),
                        request_id,
                    )),
                    false,
                )),
                Err(error) => Err(Failure::from(error)),
            }
        }
        "test.ai_tool" => ai_tool(daemon, args, request_id),
        "test.ai_consent_preview" => ai_consent_preview(daemon, args, request_id),
        "test.ai_custody_record" => ai_custody_record(daemon, args, request_id),
        "test.ai_auto_read" => ai_auto_read(daemon, args, request_id),
        _ => Err(Failure::App(AppError::new(
            "unsupported_kind",
            "unknown test kind",
        ))),
    })
}

/// The executor state a test supplies (`maintenance_state`), and the data
/// root frozen for the turn (the daemon's own unless overridden).
fn ai_runtime(daemon: &Daemon, args: Option<&Value>) -> Result<(Value, String), Failure> {
    let state = match args.and_then(|args| get(args, "state")) {
        None | Some(Value::Null) => dict::<&str>([]),
        Some(state @ Value::Dict(_)) => state.clone(),
        Some(_) => return Err(AppError::new("validation", "state must be an object").into()),
    };
    let data_root = match args.and_then(|args| get(args, "runtime_data_root")) {
        Some(Value::Str(root)) => root.as_str().unwrap_or_default().to_owned(),
        _ => daemon.data_root.clone(),
    };
    Ok((state, data_root))
}

/// `test.ai_tool`: one read call through the AI executor, as the chat loop
/// runs it (execute, record provenance, redact), with the record and the
/// model-visible bytes.
fn ai_tool(daemon: &mut Daemon, args: Option<&Value>, request_id: &RequestId) -> Handled {
    use kassiber_ai::executor::{execute_read_only, model_output, tool_result_record, Runtime};
    let (mut state, data_root) = ai_runtime(daemon, args)?;
    let call = ai_call(args);
    let custody = custody_audit(args);
    let planned = matches!(
        args.and_then(|args| get(args, "planned_auto_read")),
        Some(Value::Bool(true))
    );
    // As the chat loop dispatches: by `kind_class`, so an egressing read
    // still runs through the read executor.
    let mutating = kassiber_ai::catalog::get_tool_py(&call.name)
        .is_some_and(|entry| entry.kind_class == kassiber_ai::KindClass::Mutating);
    let result = {
        let mut runtime = Runtime {
            data_root: &data_root,
            state: &mut state,
        };
        let mut host = crate::ai_tools::Host {
            daemon: &mut *daemon,
        };
        if mutating {
            kassiber_ai::mutate::execute_mutating(&call, &mut runtime, &mut host, custody.as_ref())
        } else {
            execute_read_only(&call, &mut runtime, &mut host, planned)
        }
    };
    let display = kassiber_ai::catalog::get_tool_py(&call.name)
        .map(|entry| kassiber_pyjson::PyStr::from(entry.name.as_str()))
        .unwrap_or_else(|| call.name.clone());
    kassiber_ai::provenance::record_tool_usage(&mut state, &display, &result);
    let data = dict([
        ("tool_result", tool_result_record(&call.call_id, &result)),
        ("output", kassiber_envelope::s(model_output(&result))),
        ("state", state),
    ]);
    Ok((
        Some(with_request_id(success("test.ai_tool", data), request_id)),
        false,
    ))
}

/// `CustodyAiConsentAudit(**args["custody_audit"])`, when supplied.
fn custody_audit(args: Option<&Value>) -> Option<kassiber_ai::mutate::CustodyAudit> {
    let audit = args.and_then(|args| get(args, "custody_audit"))?;
    let text = |key: &str| match get(audit, key) {
        Some(Value::Str(text)) => text.as_str().unwrap_or_default().to_owned(),
        _ => String::new(),
    };
    Some(kassiber_ai::mutate::CustodyAudit {
        provider_kind: text("provider_kind"),
        model: text("model"),
        consent_decision: text("consent_decision"),
        consent_requested_at: text("consent_requested_at"),
        consent_decided_at: text("consent_decided_at"),
    })
}

/// `test.ai_consent_preview`: the consent card the chat loop shows before a
/// tool runs (summary, redacted or recomputed arguments, review preview).
fn ai_consent_preview(
    daemon: &mut Daemon,
    args: Option<&Value>,
    request_id: &RequestId,
) -> Handled {
    use kassiber_ai::executor::Runtime;
    let (mut state, data_root) = ai_runtime(daemon, args)?;
    let call = ai_call(args);
    let card = match kassiber_ai::catalog::get_tool_py(&call.name) {
        Some(entry) => {
            let mut runtime = Runtime {
                data_root: &data_root,
                state: &mut state,
            };
            let mut host = crate::ai_tools::Host {
                daemon: &mut *daemon,
            };
            kassiber_ai::previews::consent_request(entry, &call, &mut runtime, &mut host)
        }
        None => Value::Null,
    };
    let data = dict([("card", card), ("state", state)]);
    Ok((
        Some(with_request_id(
            success("test.ai_consent_preview", data),
            request_id,
        )),
        false,
    ))
}

/// `test.ai_custody_record`: `_record_nonexecuted_custody_ai_call` for a
/// custody proposal that was denied, timed out or cancelled.
fn ai_custody_record(daemon: &mut Daemon, args: Option<&Value>, request_id: &RequestId) -> Handled {
    use kassiber_ai::executor::Runtime;
    let (mut state, data_root) = ai_runtime(daemon, args)?;
    let call = ai_call(args);
    let audit = custody_audit(args)
        .ok_or_else(|| AppError::new("validation", "custody_audit is required"))?;
    let text = |key: &str| arg_text(args, key);
    let recorded = {
        let mut runtime = Runtime {
            data_root: &data_root,
            state: &mut state,
        };
        let mut host = crate::ai_tools::Host {
            daemon: &mut *daemon,
        };
        kassiber_ai::mutate::record_nonexecuted_custody_call(
            &call,
            &mut runtime,
            &mut host,
            &audit,
            &text("execution_status"),
            &text("execution_code"),
        )
    };
    // Python logs a failed append and carries on.
    let data = dict([
        ("recorded", Value::Bool(recorded.is_ok())),
        ("state", state),
    ]);
    Ok((
        Some(with_request_id(
            success("test.ai_custody_record", data),
            request_id,
        )),
        false,
    ))
}

/// The call a test supplies: a provider's raw tool call, or a name, its
/// arguments and a call id.
fn ai_call(args: Option<&Value>) -> kassiber_ai::executor::ParsedCall {
    use kassiber_ai::executor::{parse_tool_call, ParsedCall};
    match args.and_then(|args| get(args, "raw_call")) {
        Some(raw) => parse_tool_call(raw, 0),
        None => ParsedCall::new(
            match args.and_then(|args| get(args, "call_id")) {
                Some(Value::Str(id)) => id.clone(),
                _ => "call_0".into(),
            },
            match args.and_then(|args| get(args, "name")) {
                Some(Value::Str(name)) => name.clone(),
                _ => kassiber_pyjson::PyStr::new(),
            },
            args.and_then(|args| get(args, "arguments"))
                .cloned()
                .unwrap_or_else(|| dict::<&str>([])),
        ),
    }
}

/// `test.ai_auto_read`: the planner and `_run_auto_read_tools` for one
/// question, streaming the status, call and result records.
fn ai_auto_read(daemon: &mut Daemon, args: Option<&Value>, request_id: &RequestId) -> Handled {
    use kassiber_ai::executor::{run_auto_reads, Runtime};
    use kassiber_ai::planner::{insert_auto_tool_context_message, planned_auto_reads};
    let (mut state, data_root) = ai_runtime(daemon, args)?;
    let mut messages = match args.and_then(|args| get(args, "messages")) {
        Some(Value::List(messages)) => messages.clone(),
        _ => return Err(AppError::new("validation", "messages must be a list").into()),
    };
    let kind = match args.and_then(|args| get(args, "system_prompt_kind")) {
        Some(Value::Str(kind)) => kind.as_str().map(str::to_owned),
        _ => None,
    };
    let planned = planned_auto_reads(kind.as_deref(), &messages);
    let mut records = Vec::new();
    let context = {
        let mut runtime = Runtime {
            data_root: &data_root,
            state: &mut state,
        };
        let mut host = crate::ai_tools::Host {
            daemon: &mut *daemon,
        };
        run_auto_reads(
            &planned,
            &mut runtime,
            &mut host,
            &mut |record| records.push(record),
            &|| false,
        )
    };
    for record in records {
        daemon.output.write(&with_request_id(record, request_id))?;
    }
    if let Some(context) = context {
        insert_auto_tool_context_message(&mut messages, &context);
    }
    let planned = planned
        .into_iter()
        .map(|read| Value::List(vec![kassiber_envelope::s(read.name), read.arguments]))
        .collect();
    let data = dict([
        ("planned", Value::List(planned)),
        ("messages", Value::List(messages)),
        ("state", state),
    ]);
    Ok((
        Some(with_request_id(
            success("test.ai_auto_read", data),
            request_id,
        )),
        false,
    ))
}
