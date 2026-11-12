//! A silent-payment wallet's scan route (`_silent_payment_sync_adapter`): the
//! backend's private local scanner file, or one POST of the scan request to
//! its server-assisted scan path. Reading the file is local, so only the
//! server-assisted route carries a transport the gate authorizes.
use crate::silent_payments::scan::{self, Failure, Plan};
use kassiber_chain::{Backend, HttpClient};
use kassiber_envelope::{dict, get, py_str, s, set, AppError};
use kassiber_net::{Consent, Gate};
use kassiber_pyjson::Value as Py;
use serde_json::{json, Value};
use std::{
    io::Read,
    path::{Path, PathBuf},
    time::Instant,
};

type Result<T> = std::result::Result<T, AppError>;

fn py(value: &Value) -> Py {
    kassiber_pytext::loads(&value.to_string()).expect("JSON value")
}

fn json(value: &Py) -> Result<Value> {
    kassiber_pyjson::dumps(value, &Default::default())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .ok_or_else(|| AppError::new("internal_error", "Scanner output cannot be encoded"))
}

/// `backends.backend_value(backend, *keys)`: the first non-blank flat value.
fn backend_value(backend: &Py, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| scan::str_or_none(get(backend, key).unwrap_or(&Py::Null)))
}

pub(super) struct Route {
    plan: Plan,
    /// `normalize_backend_kind(backend["kind"])`.
    pub(super) kind: String,
    backend: Py,
    /// The wallet configuration's `sp_descriptor`, as the server request
    /// carries it.
    descriptor: Py,
    file: Option<String>,
    path: Option<String>,
    wallet_id: String,
    wallet_label: String,
}

impl Route {
    /// `discover_wallet_backend` up to the adapter: the backend's chain
    /// instance, `build_plan`, `validate_backend_for_wallet` and
    /// `validate_backend_capability`. The transport exists only for a
    /// server-assisted scan that has a scan path to request.
    pub(super) fn prepare(
        backend: &Value,
        config: &Value,
        wallet_id: &str,
        wallet_label: &str,
    ) -> Result<(Self, Option<Backend>)> {
        if let Some(instance) = backend
            .get("chain_instance_id")
            .filter(|v| crate::descriptors::truthy(v))
        {
            if Some(instance) != config.get("chain_instance_id") {
                return Err(AppError::new(
                    "book_network_mismatch",
                    "Backend belongs to another local chain instance",
                ));
            }
        }
        let plan = scan::build_plan(config)?;
        let raw = py(backend);
        let kind = crate::utxos::backend_kind(get(&raw, "kind").unwrap_or(&Py::Null));
        // `validate_backend_for_wallet(backend, chain, network)`.
        let name = py_str(get(&raw, "name").unwrap_or(&Py::Null));
        let app = |e: crate::descriptors::Error| e.into_app();
        if let Some(chain) = backend_value(&raw, &["chain"]) {
            let expected = crate::descriptors::normalize_chain(&chain).map_err(app)?;
            if expected != plan.chain {
                return Err(AppError::new(
                    "app_error",
                    format!(
                    "Backend '{name}' is configured for {expected}, but source refresh requires {}",
                    plan.chain
                ),
                ));
            }
        }
        if let Some(network) = backend_value(&raw, &["network"]) {
            let expected =
                crate::descriptors::normalize_network(&plan.chain, &network).map_err(app)?;
            if expected != plan.network {
                return Err(AppError::new(
                    "app_error",
                    format!(
                    "Backend '{name}' is configured for {expected}, but source refresh requires {}",
                    plan.network
                ),
                ));
            }
        }
        scan::validate_backend_capability(&raw, &plan, &kind)?;
        let file = backend_value(&raw, &scan::SCAN_FILE_FIELDS);
        let path = backend_value(&raw, &scan::SCAN_PATH_FIELDS);
        let transport = if plan.scan_mode == "server_assisted" && path.is_some() {
            Some(Backend::new(backend, false)?)
        } else {
            None
        };
        let descriptor = config.get("sp_descriptor").map_or(Py::Null, py);
        Ok((
            Self {
                plan,
                kind,
                backend: raw,
                descriptor,
                file,
                path,
                wallet_id: wallet_id.into(),
                wallet_label: wallet_label.into(),
            },
            transport,
        ))
    }

    /// `_silent_payment_scan_payload(backend, wallet, plan)`.
    fn payload(
        &self,
        transport: Option<&Backend>,
        gate: &Gate,
        consent: Option<&Consent>,
        version: &str,
        deadline: Instant,
    ) -> std::result::Result<Py, Failure> {
        let unavailable = |message: &str, hint: &str| {
            Failure::App(
                AppError::new("silent_payment_scanner_unavailable", message).with_hint(hint),
            )
        };
        if self.plan.scan_mode == "server_assisted" {
            let (Some(path), Some(transport)) = (&self.path, transport) else {
                return Err(unavailable(
                    "Silent Payments server-assisted scanner is not configured for this backend",
                    "Configure silent_payment_scan_path on the selected SP-capable backend.",
                ));
            };
            let int = |v: &Option<kassiber_pyjson::BigInt>| v.clone().map_or(Py::Null, Py::Int);
            let text = |v: &Option<String>| v.as_deref().map_or(Py::Null, s);
            let request = dict([
                ("descriptor", self.descriptor.clone()),
                ("chain", s(self.plan.chain.as_str())),
                ("network", s(self.plan.network.as_str())),
                ("start_height", int(&self.plan.start_height)),
                ("start_date", text(&self.plan.start_date)),
                ("full_history", Py::Bool(self.plan.full_history)),
            ]);
            let body = kassiber_pyjson::dumps(&request, &Default::default())
                .map_err(|_| AppError::new("internal_error", "Scan request cannot be encoded"))?;
            let client = HttpClient::new(transport.clone(), gate.clone(), version);
            let bytes = client.post_json(path, body.as_bytes(), consent, deadline)?;
            return decode(&bytes);
        }
        let Some(file) = &self.file else {
            return Err(unavailable(
                "Silent Payments scanner is not configured for this backend",
                "Configure a local scanner output file or an explicit server-assisted scan path on an SP-capable backend.",
            ));
        };
        read(file)
    }

    /// The adapter's records and meta, and whether Python's coordinator
    /// treats them as an authoritative chain observation: only a complete
    /// scan, every record timestamped, through the Esplora or Electrum
    /// compatibility route or Bitcoin Core RPC's ordinary adapter
    /// (`silent_payment_scan_authoritative`); a custom backend never is.
    /// Raw failures take the compatibility route's wrapping there, the
    /// ordinary `backend_fetch` wrapping otherwise.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn run(
        &self,
        transport: Option<&Backend>,
        gate: &Gate,
        consent: Option<&Consent>,
        version: &str,
        deadline: Instant,
        checkpoint: &Value,
    ) -> Result<(Value, Value, bool)> {
        let name = py_str(get(&self.backend, "name").unwrap_or(&Py::Null));
        let backend_name =
            if kassiber_envelope::truthy(get(&self.backend, "name").unwrap_or(&Py::Null)) {
                name.clone()
            } else {
                String::new()
            };
        let compatibility = matches!(self.kind.as_str(), "esplora" | "electrum");
        let result = self
            .payload(transport, gate, consent, version, deadline)
            .and_then(|payload| {
                scan::normalize(
                    &payload,
                    (&backend_name, &self.kind),
                    &self.plan,
                    &py(checkpoint),
                    (&self.wallet_id, &self.wallet_label),
                )
            });
        let normalized = match result {
            Ok(normalized) => normalized,
            Err(Failure::App(error)) => return Err(error),
            Err(Failure::Raw { message, .. }) if compatibility => {
                let text = |key: &str| match get(&self.backend, key) {
                    Some(Py::Str(t)) => {
                        kassiber_pytext::strip(&py_str(&Py::Str(t.clone()))).to_owned()
                    }
                    Some(Py::Int(n)) => n.to_string(),
                    _ => String::new(),
                };
                let credentials = kassiber_credentials::backend_credentials(text);
                let credentials: Vec<&str> = credentials.iter().map(String::as_str).collect();
                let error = kassiber_envelope::redaction::redact_operational_text(
                    &kassiber_chain::withhold_credentials(&message, &credentials),
                );
                return Err(AppError::new(
                    "backend_sync_failed",
                    format!("Bitcoin compatibility observation failed for backend '{name}'"),
                )
                .retryable(true)
                .with_details(dict([
                    ("observer_route", s("compatibility")),
                    ("error", s(error.as_str())),
                ])));
            }
            Err(Failure::Raw { kind, message }) => {
                let label = if self.wallet_label.is_empty() {
                    "Wallet"
                } else {
                    &self.wallet_label
                };
                let detail = if message.is_empty() {
                    kind.to_owned()
                } else {
                    message
                };
                let field = |key: &str| get(&self.backend, key).cloned().unwrap_or(Py::Null);
                return Err(AppError::new(
                    "backend_sync_failed",
                    format!(
                        "Source refresh failed for {label} during backend_fetch: {}",
                        crate::btcpay::provenance::redact_backend_text(
                            kassiber_pytext::strip(&detail)
                        )
                    ),
                )
                .with_hint("Test the selected sync backend in Settings, then retry refresh. If it still fails, include this error's details from Logs.")
                .retryable(true)
                .with_details(dict([
                    ("wallet", s(label)),
                    ("phase", s("backend_fetch")),
                    ("error_type", s(kind)),
                    ("backend", field("name")),
                    ("backend_kind", field("kind")),
                    ("chain", field("chain")),
                    ("network", field("network")),
                    ("has_backend_url", Py::Bool(kassiber_envelope::truthy(&field("url")))),
                ])));
            }
        };
        let mut meta = normalized.meta;
        set(&mut meta, "observer_route", s("silent_payments"));
        if compatibility {
            set(
                &mut meta,
                "observer_compatibility_reason",
                s("silent_payment"),
            );
        }
        let records = Py::List(normalized.records);
        let authoritative = (compatibility || self.kind == "bitcoinrpc")
            && normalized.complete
            && match &records {
                Py::List(items) => items.iter().all(|record| {
                    !matches!(get(record, "confirmed_at"), None | Some(Py::Null))
                        && get(record, "confirmed_at") != Some(&s(""))
                }),
                _ => false,
            };
        Ok((json(&records)?, json(&meta)?, authoritative))
    }

    /// `sync_target(plan)`.
    pub(super) fn target(&self) -> Value {
        scan::sync_target(&self.plan)
    }

    pub(super) fn chain(&self) -> &str {
        &self.plan.chain
    }

    pub(super) fn network(&self) -> &str {
        &self.plan.network
    }
}

/// `json.loads(body.decode("utf-8"))` of a server-assisted scan response.
fn decode(bytes: &[u8]) -> std::result::Result<Py, Failure> {
    let text = std::str::from_utf8(bytes).map_err(|e| Failure::Raw {
        kind: "UnicodeDecodeError",
        message: kassiber_chain::utf8_error(bytes, &e),
    })?;
    kassiber_pytext::loads(text).map_err(|e| Failure::Raw {
        kind: json_error_type(&e),
        message: e.to_string(),
    })
}

fn json_error_type(error: &kassiber_pytext::JsonError) -> &'static str {
    use kassiber_pytext::JsonErrorKind;
    match error.kind {
        JsonErrorKind::Decode { .. } => "JSONDecodeError",
        JsonErrorKind::TooDeep { .. } => "RecursionError",
        _ => "ValueError",
    }
}

/// `Path(scan_file).expanduser()`, lexically normalized as pathlib does.
fn expand_user(raw: &str) -> std::result::Result<PathBuf, Failure> {
    let home = || std::env::var_os("HOME").map(PathBuf::from);
    let expanded = if raw == "~" {
        home()
    } else if let Some(rest) = raw.strip_prefix("~/") {
        home().map(|home| home.join(rest))
    } else if raw.starts_with('~') {
        kassiber_paths::Context::from_env()
            .ok()
            .and_then(|context| context.expand_user(Path::new(raw)).ok())
    } else {
        Some(PathBuf::from(raw))
    };
    let path = expanded.ok_or_else(|| Failure::Raw {
        kind: "RuntimeError",
        message: "Could not determine home directory.".into(),
    })?;
    Ok(path.components().collect())
}

/// `_open_private_silent_payment_scan_file` and `json.load`: a regular file
/// this user owns that no one else can read or write, opened without
/// following a final symlink.
fn read(file: &str) -> std::result::Result<Py, Failure> {
    let path = expand_user(file)?;
    let unreadable = || {
        Failure::App(
            AppError::new(
                "silent_payment_scanner_unavailable",
                "Silent Payments local scanner output could not be read",
            )
            .with_hint("Check the backend's silent_payment_scan_file path and keep it as a private regular JSON file.")
            .retryable(true),
        )
    };
    let mut handle = kassiber_paths::open_read_nofollow(&path).map_err(|_| unreadable())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let info = handle.metadata().map_err(|_| unreadable())?;
        let unavailable = |message: &str, hint: &str| {
            AppError::new("silent_payment_scanner_unavailable", message).with_hint(hint)
        };
        if !info.file_type().is_file() {
            return Err(unavailable(
                "Silent Payments local scanner output must be a regular file",
                "Point silent_payment_scan_file at a private regular JSON file.",
            )
            .into());
        }
        if info.uid() != kassiber_paths::current_uid() {
            return Err(unavailable(
                "Silent Payments local scanner output must be owned by the current OS user",
                "Move the scanner JSON to a file owned by the user running Kassiber.",
            )
            .into());
        }
        if info.mode() & 0o077 != 0 {
            return Err(unavailable(
                "Silent Payments local scanner output is readable or writable by other users",
                "Set the scanner JSON file permissions to 0600 before syncing.",
            )
            .with_details(dict([(
                "mode",
                s(format!("0o{:o}", info.mode() & 0o7777).as_str()),
            )]))
            .into());
        }
    }
    let mut bytes = Vec::new();
    handle.read_to_end(&mut bytes).map_err(|_| unreadable())?;
    let text = std::str::from_utf8(&bytes).map_err(|e| Failure::Raw {
        kind: "UnicodeDecodeError",
        message: kassiber_chain::utf8_error(&bytes, &e),
    })?;
    kassiber_pytext::loads(text).map_err(|e| {
        if e.is_decode_error() {
            Failure::App(AppError::new(
                "silent_payment_scanner_invalid",
                "Silent Payments local scanner output is not valid JSON",
            ))
        } else {
            Failure::Raw {
                kind: json_error_type(&e),
                message: e.to_string(),
            }
        }
    })
}

/// The `sync_state` a silent-payment fetch reports: one target, no gap.
pub(super) fn sync_state(route: &Route, checkpoint: &Value) -> Value {
    let target = route.target();
    json!({"chain":route.chain(),"network":route.network(),"target_count":1,"gap_limit":null,
        "descriptor_kind":"silent-payment","policy_asset_id":"","checkpoint":checkpoint,
        "targets":[super::target_summary(&target)]})
}
