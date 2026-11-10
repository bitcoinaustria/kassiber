use super::{csv, Identity};
use crate::{config, database_error, descriptors, lifecycle};
use kassiber_envelope::{dict, get, py_str, set, truthy, AppError};
use kassiber_pyjson::{Key, Options, PyStr, Value as Py};
use kassiber_setup::references::{resolve, Entity};
use kassiber_store::rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

fn s(value: impl AsRef<str>) -> Py {
    kassiber_envelope::s(value.as_ref())
}

pub struct Request {
    pub workspace: Option<Py>,
    pub profile: Option<Py>,
    pub label: String,
    pub account: Option<String>,
    pub backend: Option<String>,
    pub network: Option<String>,
    pub gap_limit: Option<Py>,
    pub file: String,
    pub source_set: Option<Py>,
}

struct Template {
    section: &'static str,
    label: &'static str,
    purpose: u32,
    account: u32,
    script: &'static str,
}

const TEMPLATES: &[Template] = &[
    Template {
        section: "deposit",
        label: "Deposit Legacy",
        purpose: 44,
        account: 0,
        script: "p2pkh",
    },
    Template {
        section: "deposit",
        label: "Deposit Nested SegWit",
        purpose: 49,
        account: 0,
        script: "p2sh-p2wpkh",
    },
    Template {
        section: "deposit",
        label: "Deposit Native SegWit",
        purpose: 84,
        account: 0,
        script: "p2wpkh",
    },
    Template {
        section: "deposit",
        label: "Deposit PayNym",
        purpose: 47,
        account: 0,
        script: "p2pkh",
    },
    Template {
        section: "badbank",
        label: "Badbank / Toxic Change",
        purpose: 84,
        account: 2147483644,
        script: "p2wpkh",
    },
    Template {
        section: "premix",
        label: "Premix",
        purpose: 84,
        account: 2147483645,
        script: "p2wpkh",
    },
    Template {
        section: "postmix",
        label: "Postmix",
        purpose: 84,
        account: 2147483646,
        script: "p2wpkh",
    },
    Template {
        section: "ricochet",
        label: "Ricochet Legacy",
        purpose: 44,
        account: 2147483647,
        script: "p2pkh",
    },
    Template {
        section: "ricochet",
        label: "Ricochet Nested SegWit",
        purpose: 49,
        account: 2147483647,
        script: "p2sh-p2wpkh",
    },
    Template {
        section: "ricochet",
        label: "Ricochet Native SegWit",
        purpose: 84,
        account: 2147483647,
        script: "p2wpkh",
    },
];
const SECTIONS: &[&str] = &["deposit", "badbank", "premix", "postmix", "ricochet"];
const PROVENANCE: &[&str] = &[
    "minimum_mix_count",
    "mix_count",
    "mix_count_confidence",
    "target_mix_count",
    "pool_denomination_sat",
    "coordinator_fee_sat",
    "miner_fee_sat",
    "round_txid",
    "round_txids",
    "tx0_role",
    "whirlpool_event",
    "privacy_event",
    "exit_kind",
    "ricochet_hops",
];

fn field<'a>(v: &'a Py, key: &str) -> &'a Py {
    get(v, key).unwrap_or(&Py::Null)
}
fn strip(v: &Py) -> Py {
    if *v == Py::Null {
        return Py::Null;
    }
    let value = match v {
        Py::Str(v) => v.clone(),
        _ => PyStr::from(py_str(v)),
    };
    let points = value.code_points().collect::<Vec<_>>();
    let first = points
        .iter()
        .position(|p| !kassiber_pytext::is_space(*p))
        .unwrap_or(points.len());
    let last = points
        .iter()
        .rposition(|p| !kassiber_pytext::is_space(*p))
        .map_or(first, |p| p + 1);
    if first == last {
        Py::Null
    } else {
        Py::Str(PyStr::from_code_points(points[first..last].iter().copied()).unwrap())
    }
}
fn render(v: &Py) -> String {
    match v {
        Py::Str(value) if value.has_surrogates() => value
            .code_points()
            .map(|p| char::from_u32(p).unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect(),
        _ => py_str(v),
    }
}
fn or(v: &Py, default: &str) -> String {
    let v = strip(v);
    if v == Py::Null {
        default.into()
    } else {
        render(&v)
    }
}
fn validation(message: &str) -> AppError {
    AppError::new("validation", message)
}
fn integer(v: &Py) -> Result<Option<Py>, AppError> {
    match v {
        Py::Int(_) => Ok(Some(v.clone())),
        Py::Bool(v) => Ok(Some(Py::from(i32::from(*v)))),
        Py::Float(v) if !v.is_finite() => Err(AppError::new(
            "internal_error",
            if v.is_nan() {
                "cannot convert float NaN to integer"
            } else {
                "cannot convert float infinity to integer"
            },
        )),
        Py::Float(v) => Ok(kassiber_pytext::parse_int(&format!("{:.0}", v.trunc()))
            .ok()
            .map(Py::Int)),
        Py::Str(v) => Ok(v
            .as_str()
            .and_then(|v| kassiber_pytext::parse_int(v).ok())
            .map(Py::Int)),
        _ => Ok(None),
    }
}
fn normalize_network(v: &str) -> Result<String, AppError> {
    descriptors::normalize_network("bitcoin", v).map_err(|e| match e {
        descriptors::Error::Value(message) => AppError::new("app_error", message),
        descriptors::Error::App(e) => e,
    })
}
fn gap(raw: &Py, default: Option<&Py>) -> Result<Py, AppError> {
    let raw = if *raw == Py::Null || *raw == s("") {
        default.unwrap_or(&Py::Null)
    } else {
        raw
    };
    let value = if *raw == Py::Null || *raw == s("") {
        Py::from(100)
    } else {
        match integer(raw) {
            Err(e) if matches!(raw, Py::Float(v) if v.is_nan()) => {
                let _ = e;
                return Err(validation("Descriptor gap limit must be an integer"));
            }
            result => {
                result?.ok_or_else(|| validation("Descriptor gap limit must be an integer"))?
            }
        }
    };
    let value = if !truthy(&value) {
        Py::from(100)
    } else {
        value
    };
    let Py::Int(n) = &value else { unreachable!() };
    if *n < 0.into() {
        return Err(validation("Descriptor gap limit must be positive"));
    }
    if *n > 5000.into() {
        return Err(validation("Descriptor gap limit must be 5000 or lower"));
    }
    Ok(if *n < 100.into() {
        Py::from(100)
    } else {
        value
    })
}
fn root(t: &Template, network: &str) -> String {
    format!(
        "m/{}'/{}'/{}'",
        t.purpose,
        if network == "main" { 0 } else { 1 },
        t.account
    )
}
fn template(section: &str, script: &str, path: &str) -> Result<&'static Template, AppError> {
    let section = kassiber_pytext::lower(kassiber_pytext::strip(section)).replace('_', "-");
    let script = kassiber_pytext::lower(kassiber_pytext::strip(script));
    let candidates = TEMPLATES
        .iter()
        .filter(|t| t.section == section && t.script == script)
        .collect::<Vec<_>>();
    candidates.iter().find(|t| [root(t, "main"), root(t, "test")].contains(&kassiber_pytext::strip(path).to_owned())).copied()
        .or_else(|| candidates.first().copied()).ok_or_else(|| validation("Explicit Samourai source has an unsupported section/script type")
            .with_hint("Use Deposit, Badbank, Premix, Postmix, or Ricochet with p2pkh, p2sh-p2wpkh, or p2wpkh."))
}
fn txid(v: &Py) -> Option<Py> {
    let raw = or(if truthy(v) { v } else { &Py::Null }, "");
    let normalized = kassiber_pytext::lower(&raw);
    if normalized.chars().count() != 64 {
        return None;
    }
    let bytes = normalized.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if matches!(bytes[at], 9..=13 | 32) {
            at += 1;
        } else if bytes.get(at + 1).is_some_and(u8::is_ascii_hexdigit)
            && bytes[at].is_ascii_hexdigit()
        {
            at += 2;
        } else {
            return None;
        }
    }
    Some(s(normalized))
}
fn metadata(raw: &Py) -> Result<Py, AppError> {
    let mut merged = match field(raw, "samourai") {
        Py::Dict(v) => Py::Dict(v.clone()),
        _ => dict::<&str>([]),
    };
    for key in PROVENANCE {
        if let Some(value) = get(raw, key) {
            set(&mut merged, key, value.clone());
        }
    }
    let mut result = dict::<&str>([]);
    if let Py::Dict(fields) = &merged {
        for (key, value) in fields {
            let Key::Str(key) = key else {
                continue;
            };
            let Some(key) = key.as_str() else {
                continue;
            };
            if !PROVENANCE.contains(&key) {
                continue;
            }
            let normalized = match key {
                "minimum_mix_count"
                | "mix_count"
                | "target_mix_count"
                | "pool_denomination_sat"
                | "coordinator_fee_sat"
                | "miner_fee_sat"
                | "ricochet_hops" => {
                    let number = match integer(value) {
                        Err(_) if matches!(value, Py::Float(v) if v.is_nan()) => None,
                        result => result?,
                    };
                    number.filter(|n| matches!(n, Py::Int(n) if *n >= 0.into()))
                }
                "round_txid" => txid(value),
                "round_txids" => {
                    if let Py::List(v) = value {
                        let v = v.iter().filter_map(txid).collect::<Vec<_>>();
                        (!v.is_empty()).then_some(Py::List(v))
                    } else {
                        None
                    }
                }
                "mix_count_confidence"
                | "tx0_role"
                | "whirlpool_event"
                | "privacy_event"
                | "exit_kind" => {
                    let options: &[&str] = match key {
                        "mix_count_confidence" => &["minimum", "exact", "estimated", "unknown"],
                        "tx0_role" => &["deposit", "premix", "badbank", "fee"],
                        "whirlpool_event" => &[
                            "tx0",
                            "premix_pending",
                            "first_mix",
                            "remix",
                            "mix_to_wallet",
                            "external_spend",
                        ],
                        "privacy_event" => &[
                            "coinjoin",
                            "payjoin",
                            "tx0",
                            "first_mix",
                            "remix",
                            "ricochet",
                            "exit",
                        ],
                        _ => &[
                            "cold_storage",
                            "external_spend",
                            "ricochet",
                            "toxic_change_spend",
                        ],
                    };
                    let v = kassiber_pytext::lower(&or(value, ""));
                    options.contains(&v.as_str()).then(|| s(v))
                }
                _ => None,
            };
            if let Some(v) = normalized {
                set(&mut result, key, v);
            }
        }
    }
    Ok(result)
}
fn descriptor_config(config: &Py) -> Result<Value, AppError> {
    let mut value = json!({});
    for key in [
        "chain",
        "network",
        "descriptor",
        "change_descriptor",
        "gap_limit",
    ] {
        if let Some(v) = get(config, key) {
            let raw = kassiber_pyjson::dumps(v, &Options::default()).unwrap();
            value[key] = serde_json::from_str(&raw).map_err(|_| {
                validation("Samourai descriptor material is malformed")
                    .with_hint("Check the descriptor or xpub set before importing.")
            })?;
        }
    }
    Ok(value)
}
fn load_plan(config: &Py) -> Result<Option<descriptors::Plan>, AppError> {
    descriptors::load(&descriptor_config(config)?).map_err(|e| match e {
        descriptors::Error::App(e) => e,
        descriptors::Error::Value(_) => validation("Samourai descriptor material is malformed")
            .with_hint("Check the descriptor or xpub set before importing."),
    })
}

pub fn load_payload(
    data: &Py,
    network: &str,
    default_gap: Option<&Py>,
) -> Result<Vec<Py>, AppError> {
    if !matches!(data, Py::Dict(_)) {
        return Err(validation(
            "Samourai descriptor/xpub set must be a JSON object",
        ));
    }
    let declared = strip(field(data, "network"));
    let declared = if declared != Py::Null {
        if let Py::Str(value) = &declared {
            if value.has_surrogates() {
                let message = PyStr::from_code_points(
                    "Unsupported bitcoin network '"
                        .chars()
                        .map(u32::from)
                        .chain(value.code_points())
                        .chain(std::iter::once(u32::from('\''))),
                )
                .unwrap();
                return Err(AppError::new("app_error", "Unsupported bitcoin network")
                    .with_python_message(message));
            }
        }
        Some(normalize_network(&py_str(&declared))?)
    } else {
        None
    };
    let normalized = normalize_network(if network.is_empty() {
        declared.as_deref().unwrap_or("main")
    } else {
        network
    })?;
    if let Some(declared) = declared {
        if declared != normalized {
            return Err(
                validation("Samourai source-set network does not match --network")
                    .with_hint(format!(
                        "Use --network {declared} or choose a matching descriptor/xpub source set."
                    ))
                    .with_details(dict([
                        ("source_set_network", s(declared)),
                        ("requested_network", s(normalized)),
                    ])),
            );
        }
    }
    let sources = [field(data, "children"), field(data, "sources")]
        .into_iter()
        .find(|v| truthy(v))
        .cloned()
        .unwrap_or(Py::List(vec![]));
    let xpubs = if truthy(field(data, "xpubs")) {
        field(data, "xpubs").clone()
    } else {
        Py::List(vec![])
    };
    let (Py::List(sources), Py::List(xpubs)) = (sources, xpubs) else {
        return Err(validation(
            "Samourai source set children/xpubs must be lists",
        ));
    };
    let mut output = vec![];
    for (is_xpub, raw) in sources
        .iter()
        .map(|v| (false, v))
        .chain(xpubs.iter().map(|v| (true, v)))
    {
        if !matches!(raw, Py::Dict(_)) {
            return Err(validation(if is_xpub {
                "Samourai xpub entries must be objects"
            } else {
                "Samourai source entries must be objects"
            }));
        }
        let descriptor = strip(field(raw, "descriptor"));
        if !is_xpub && descriptor == Py::Null {
            return Err(validation(
                "Samourai descriptor source is missing descriptor",
            ));
        }
        let section = or(field(raw, "section"), "deposit");
        let script = match strip(field(raw, "script_type")) {
            Py::Null if is_xpub => "p2wpkh".into(),
            Py::Null => {
                let compact = kassiber_pytext::lower(
                    &render(&descriptor)
                        .chars()
                        .filter(|c| !kassiber_pytext::is_space(*c as u32))
                        .collect::<String>(),
                );
                if compact.starts_with("pkh(") {
                    "p2pkh".into()
                } else if compact.starts_with("sh(wpkh(") {
                    "p2sh-p2wpkh".into()
                } else if compact.starts_with("wpkh(") {
                    "p2wpkh".into()
                } else {
                    return Err(validation(
                        "Samourai descriptor source has an unsupported descriptor type",
                    )
                    .with_hint("Use pkh(), sh(wpkh()), or wpkh() account descriptors."));
                }
            }
            v => render(&v),
        };
        let path = strip(field(raw, "root_path"));
        if is_xpub && path == Py::Null {
            return Err(validation("Samourai xpub source is missing root_path")
                .with_hint("Include the BIP32 account path so provenance is explicit."));
        }
        let xpub = strip(field(raw, "xpub"));
        if is_xpub && xpub == Py::Null {
            return Err(validation("Samourai xpub source is missing xpub"));
        }
        let t = template(&section, &script, &or(&path, ""))?;
        let expected = root(t, &normalized);
        let provided_path = if path == Py::Null { s(&expected) } else { path };
        if provided_path != s(&expected) {
            return Err(validation(
                "Explicit Samourai source uses a path outside the supported account map",
            )
            .with_hint(format!("Use {expected} for {} on {normalized}.", t.label))
            .with_details(dict([
                ("expected_root_path", s(expected)),
                ("provided_root_path", provided_path),
            ])));
        }
        let path = expected;
        let gap = gap(field(raw, "gap_limit"), default_gap)?;
        let mut meta = dict([
            ("role", s("child")),
            ("section", s(t.section)),
            ("script_type", s(t.script)),
            ("root_path", s(&path)),
            ("gap_limit", gap.clone()),
            (
                "privacy_boundary",
                Py::Bool(["badbank", "premix", "postmix"].contains(&t.section)),
            ),
        ]);
        if ["badbank", "premix", "postmix"].contains(&t.section) {
            set(&mut meta, "whirlpool", Py::Bool(true));
        }
        if t.section == "badbank" {
            set(&mut meta, "toxic_change", Py::Bool(true));
        }
        if t.purpose == 47 {
            set(&mut meta, "paynym", Py::Bool(true));
            set(
                &mut meta,
                "scanned_without_explicit_descriptor",
                Py::Bool(false),
            );
        }
        if t.section == "postmix" {
            set(&mut meta, "minimum_mix_count", Py::from(1));
            set(&mut meta, "mix_count_confidence", s("minimum"));
        }
        if !is_xpub {
            if let Py::Dict(fields) = metadata(raw)? {
                for (k, v) in fields {
                    if let Key::Str(k) = k {
                        set(&mut meta, k.as_str().unwrap(), v);
                    }
                }
            }
        }
        let mut config = dict([
            ("chain", s("bitcoin")),
            ("network", s(&normalized)),
            ("descriptor", descriptor),
            ("gap_limit", gap),
            ("samourai", meta),
        ]);
        if is_xpub {
            if [field(raw, "fingerprint"), &xpub]
                .iter()
                .any(|v| matches!(v,Py::Str(v) if v.has_surrogates()))
            {
                return Err(validation("Samourai descriptor material is malformed")
                    .with_hint("Check the descriptor or xpub set before importing."));
            }
            let fingerprint = kassiber_pytext::lower(&or(field(raw, "fingerprint"), "00000000"));
            for (branch, key) in [(0, "descriptor"), (1, "change_descriptor")] {
                let origin = format!("[{fingerprint}/{}]{}/{branch}/*", &path[2..], py_str(&xpub));
                let descriptor = match t.script {
                    "p2pkh" => format!("pkh({origin})"),
                    "p2sh-p2wpkh" => format!("sh(wpkh({origin}))"),
                    _ => format!("wpkh({origin})"),
                };
                set(&mut config, key, s(descriptor));
            }
        } else if let change @ Py::Str(_) = strip(field(raw, "change_descriptor")) {
            set(&mut config, "change_descriptor", change);
        }
        let plan = load_plan(&config)?;
        if !is_xpub {
            let origin_error = || {
                validation("Samourai descriptor origin does not match the declared account path")
                .with_hint("Use descriptors with key origins that match the declared Samourai section/root_path.")
                .with_details(dict([("expected_root_path", s(&path))]))
            };
            let plan = plan.ok_or_else(origin_error)?;
            let actual = plan
                .branches
                .iter()
                .map(|b| b.index)
                .collect::<BTreeSet<_>>();
            let missing = [0, 1]
                .into_iter()
                .filter(|b| !actual.contains(b))
                .collect::<Vec<_>>();
            if !missing.is_empty() {
                return Err(validation("Samourai descriptor source is missing a required receive/change branch")
                .with_hint("Include both descriptor and change_descriptor, or a ranged descriptor covering branches 0 and 1.")
                .with_details(dict([("section",s(t.section)),("root_path",s(&path)),("missing_branches",Py::List(missing.iter().map(|v| Py::from(*v as i32)).collect()))])));
            }
            for branch in &plan.branches {
                let target = plan.derive(Some(branch.index), 0, 1).map_err(|e| match e {
                    descriptors::Error::App(e) => e,
                    _ => origin_error(),
                })?;
                let expected = json!([format!("{path}/{}/0", branch.index)]);
                if target.first().and_then(|v| v.get("derivation_paths")) != Some(&expected) {
                    return Err(origin_error());
                }
            }
        } else {
            let mut meta = field(&config, "samourai").clone();
            if let Py::Dict(fields) = metadata(raw)? {
                for (k, v) in fields {
                    if let Key::Str(k) = k {
                        set(&mut meta, k.as_str().unwrap(), v);
                    }
                }
            }
            set(&mut config, "samourai", meta);
        }
        output.push(dict([
            ("section", s(t.section)),
            (
                "label",
                if is_xpub {
                    s(t.label)
                } else {
                    let label = strip(field(raw, "label"));
                    if label == Py::Null {
                        s(t.label)
                    } else {
                        label
                    }
                },
            ),
            ("config", config),
        ]));
    }
    if output.is_empty() {
        return Err(validation(
            "Samourai source set does not contain any descriptor or xpub sources",
        ));
    }
    Ok(output)
}

pub fn load_file(path: &str, network: &str, gap: Option<&Py>) -> Result<Vec<Py>, AppError> {
    let expanded = if path.starts_with('~') {
        kassiber_paths::Context::from_env()
            .and_then(|context| context.expand_user(std::path::Path::new(path)))
            .map_err(|_| AppError::new("internal_error", "Could not determine home directory."))?
            .to_string_lossy()
            .into_owned()
    } else {
        path.into()
    };
    let text = csv::read_text_direct(&expanded).map_err(|e| {
        if e.message.starts_with("'utf-8' codec") {
            e
        } else {
            AppError::new("not_found", "Could not read Samourai descriptor/xpub set")
                .with_hint("Choose a readable local JSON file.")
                .with_details(dict([("path", s(path))]))
        }
    })?;
    let payload = kassiber_pytext::loads(&text).map_err(|e| {
        if e.is_decode_error() {
            validation("Samourai descriptor/xpub set is not valid JSON")
                .with_hint("Choose a JSON file with children, sources, or xpubs entries.")
        } else {
            AppError::new("internal_error", e.to_string())
        }
    })?;
    load_payload(&payload, network, gap)
}

fn bind(value: &Py) -> Result<String, AppError> {
    match value {
        Py::Str(value) => value
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| crate::unicode_binding_error(value)),
        _ => Ok(py_str(value)),
    }
}
pub fn execute(
    conn: &Connection,
    request: Request,
    identity: Option<&Identity>,
) -> Result<Py, AppError> {
    let inline = request.source_set.as_ref().filter(|v| **v != Py::Null);
    let selected = usize::from(!request.file.is_empty()) + usize::from(inline.is_some());
    if selected != 1 {
        return Err(validation("Choose exactly one Samourai import source")
            .with_hint("Use a descriptor/xpub source-set file or inline source_set payload."));
    }
    let network = normalize_network(request.network.as_deref().unwrap_or("main"))?;
    let sources = if let Some(payload) = inline {
        load_payload(payload, &network, request.gap_limit.as_ref())?
    } else {
        load_file(&request.file, &network, request.gap_limit.as_ref())?
    };
    let label = kassiber_pytext::strip(&request.label);
    if label.is_empty() {
        return Err(validation("Samourai group label is required"));
    }
    let workspace = resolve(conn, Entity::Workspace, request.workspace.as_ref(), None)?;
    let workspace = py_str(field(&workspace, "id"));
    let profile = resolve(
        conn,
        Entity::Profile,
        request.profile.as_ref(),
        Some(&workspace),
    )?;
    let profile = py_str(field(&profile, "id"));
    let account = lifecycle::account(
        conn,
        &profile,
        request
            .account
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or("treasury"),
    )?;
    let mut at = 0;
    let mut next = || -> Result<String, AppError> {
        if let Some(identity) = identity {
            let id = identity
                .ids
                .get(at)
                .ok_or_else(|| AppError::new("internal_error", "Import test identities exhausted"))?
                .clone();
            at += 1;
            Ok(id)
        } else {
            lifecycle::uuid(conn)
        }
    };
    let group_id = next()?;
    let mut labels = vec![s(label)];
    for source in &sources {
        let Py::Str(child) = field(source, "label") else {
            unreachable!()
        };
        labels.push(Py::Str(
            PyStr::from_code_points(
                format!("{label} - ")
                    .chars()
                    .map(u32::from)
                    .chain(child.code_points()),
            )
            .unwrap(),
        ));
    }
    let mut counts = BTreeMap::new();
    for label in &labels {
        let Py::Str(label) = label else {
            unreachable!()
        };
        *counts.entry(label.clone()).or_insert(0) += 1;
    }
    let duplicates = counts
        .iter()
        .filter(|(_, count)| **count > 1)
        .map(|(v, _)| Py::Str(v.clone()))
        .collect::<Vec<_>>();
    if !duplicates.is_empty() {
        return Err(
            validation("Samourai import contains duplicate wallet labels")
                .with_hint(
                    "Give duplicate source entries distinct labels or remove the duplicate entry.",
                )
                .with_details(dict([("duplicate_labels", Py::List(duplicates))])),
        );
    }
    let mut conflicts = BTreeSet::new();
    for label in &labels {
        let label = bind(label)?;
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM wallets WHERE profile_id=? AND label=?",
                [&profile, &label],
                |r| r.get(0),
            )
            .map_err(database_error)?;
        if count > 0 {
            conflicts.insert(label);
        }
    }
    if !conflicts.is_empty() {
        return Err(AppError::new(
            "conflict",
            "Samourai import would overwrite existing wallet labels",
        )
        .with_hint("Choose a different group label or remove the existing wallet first.")
        .with_details(dict([(
            "conflicting_labels",
            Py::List(conflicts.into_iter().map(s).collect()),
        )])));
    }
    let mut parent_config = dict([
        ("chain", s("bitcoin")),
        ("network", s(&network)),
        (
            "gap_limit",
            request
                .gap_limit
                .clone()
                .filter(truthy)
                .unwrap_or(Py::from(100)),
        ),
        (
            "samourai",
            dict([
                ("role", s("parent")),
                ("group_id", s(&group_id)),
                ("source", s("source_set")),
                (
                    "sections",
                    Py::List(SECTIONS.iter().map(|v| s(*v)).collect()),
                ),
                (
                    "bip47",
                    s("recognised_not_scanned_without_explicit_descriptors"),
                ),
                ("watch_only", Py::Bool(true)),
            ]),
        ),
    ]);
    let backend = request
        .backend
        .as_ref()
        .filter(|s| !s.is_empty())
        .map(|v| kassiber_pytext::lower(kassiber_pytext::strip(v)));
    if let Some(v) = &backend {
        set(&mut parent_config, "backend", s(v));
    }
    kassiber_store::begin_write_savepoint(conn, "samourai_import").map_err(database_error)?;
    let result = (|| {
        let mut insert = |label: &Py, kind: &str, config: &Py| -> Result<Py, AppError> {
            config::validate(kind, &descriptor_config(config)?)?;
            let id = next()?;
            let created = identity
                .map(|i| Ok(i.timestamp.clone()))
                .unwrap_or_else(|| lifecycle::now(conn))?;
            let stored = kassiber_pyjson::dumps(
                config,
                &Options {
                    sort_keys: true,
                    ..Default::default()
                },
            )
            .unwrap();
            conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,account_id,label,kind,config_json,created_at) VALUES(?,?,?,?,?,?,?,?)",params![id,workspace,profile,account,bind(label)?,kind,stored,created]).map_err(database_error)?;
            lifecycle::execute(
                conn,
                lifecycle::Operation::Get {
                    profile_id: profile.clone(),
                    wallet: id,
                },
            )
        };
        let parent = insert(&labels[0], "samourai", &parent_config)?;
        let parent_id = field(&parent, "id").clone();
        let mut children = vec![];
        for (source, label_value) in sources.iter().zip(&labels[1..]) {
            let mut config = field(source, "config").clone();
            let mut meta = field(&config, "samourai").clone();
            for (k, v) in [
                ("role", s("child")),
                ("group_id", s(&group_id)),
                ("group_label", s(label)),
                ("parent_wallet_id", parent_id.clone()),
                ("source", s("source_set")),
            ] {
                set(&mut meta, k, v);
            }
            set(&mut config, "samourai", meta);
            if let Some(v) = &backend {
                set(&mut config, "backend", s(v));
            }
            children.push(insert(label_value, "descriptor", &config)?);
        }
        let mut warnings = vec![dict([("code",s("bip47_not_auto_scanned")),("message",s("Samourai payment-code paths are recognised but are not scanned unless supplied as explicit descriptors."))])];
        if sources.iter().any(|v| field(v, "section") == &s("postmix")) {
            warnings.push(dict([("code",s("postmix_gap_watch")),("message",s("Postmix discovery uses a widened gap limit; raise it for old wallets with long unused-address runs."))]));
        }
        Ok(dict([
            ("group", parent),
            ("children", Py::List(children)),
            ("warnings", Py::List(warnings)),
        ]))
    })();
    if result.is_err() {
        conn.execute_batch("ROLLBACK TO samourai_import")
            .map_err(database_error)?;
    }
    conn.execute_batch("RELEASE samourai_import")
        .map_err(database_error)?;
    result
}

#[cfg(test)]
mod tests;
