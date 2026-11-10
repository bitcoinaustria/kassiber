//! `lwk_descriptor_for_plan` and `lwk_compatibility_reason`: translate the
//! executable embit-spelled plan into LWK's CT descriptor, and name every
//! configuration the native route does not carry. Local and secret-free.
use super::Result;
use crate::descriptors::{Blinding, Plan};
use kassiber_envelope::AppError;
use lwk_wollet::WolletDescriptor;
use serde_json::Value;
use std::str::FromStr;

fn unsupported(message: &str) -> AppError {
    AppError::new("observer_capability_unsupported", message)
}

/// `_LIQUID_BRANCH_STEP_RE` (`/(?P<branch>[01])/\*`) over a descriptor text.
fn branch_steps(text: &str) -> Vec<(usize, char)> {
    let bytes = text.as_bytes();
    let mut steps = vec![];
    let mut i = 0;
    while i + 4 <= bytes.len() {
        if bytes[i] == b'/'
            && matches!(bytes[i + 1], b'0' | b'1')
            && bytes[i + 2] == b'/'
            && bytes[i + 3] == b'*'
        {
            steps.push((i, bytes[i + 1] as char));
            i += 4;
        } else {
            i += 1;
        }
    }
    steps
}

fn replace_steps(text: &str, replacement: &str) -> String {
    let mut out = String::new();
    let mut last = 0;
    for (at, _) in branch_steps(text) {
        out.push_str(&text[last..at]);
        out.push_str(replacement);
        last = at + 4;
    }
    out.push_str(&text[last..]);
    out
}

/// `_canonical_liquid_multipath_text`: combine only provably-equivalent
/// separate receive/change descriptors.
pub(crate) fn canonical_multipath(plan: &Plan) -> Option<String> {
    let [receive, change] = plan.branches.as_slice() else {
        return None;
    };
    let (receive, change) = (receive.descriptor_text(), change.descriptor_text());
    let (r, c) = (branch_steps(receive), branch_steps(change));
    if r.is_empty() || r.len() != c.len() {
        return None;
    }
    if r.iter().any(|(_, b)| *b != '0') || c.iter().any(|(_, b)| *b != '1') {
        return None;
    }
    if replace_steps(receive, "/{branch}/*") != replace_steps(change, "/{branch}/*") {
        return None;
    }
    Some(replace_steps(receive, "/<0;1>/*"))
}

fn plan_text(plan: &Plan) -> Result<String> {
    let first = plan
        .branches
        .first()
        .ok_or_else(|| unsupported("LWK requires a descriptor branch"))?;
    if let [receive, change] = plan.branches.as_slice() {
        if receive.descriptor_text() != change.descriptor_text() {
            return canonical_multipath(plan).ok_or_else(|| {
                unsupported(
                    "Separate Liquid receive/change descriptors are not structurally equivalent",
                )
            });
        }
    }
    Ok(first.descriptor_text().to_owned())
}

/// `_translate_lwk_script_wrapper`: only the outer Elements script wrapper.
pub(crate) fn translate_wrapper(text: &str) -> Result<String> {
    let mut depth = 0i32;
    let mut start = None;
    for (index, character) in text.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 1 => {
                start = Some(index + 1);
                break;
            }
            _ => {}
        }
    }
    let start = start.ok_or_else(|| {
        unsupported("LWK confidential descriptor is missing its script expression")
    })?;
    let script = &text[start..];
    for (source, target) in [
        ("wpkh(", "elwpkh("),
        ("wsh(", "elwsh("),
        ("tr(", "eltr("),
        ("sh(", "elsh("),
    ] {
        if let Some(rest) = script.strip_prefix(source) {
            return Ok(format!("{}{target}{rest}", &text[..start]));
        }
    }
    Ok(text.to_owned())
}

/// The LWK CT descriptor text for one preflighted Liquid plan.
pub fn descriptor_text(plan: &Plan) -> Result<String> {
    let text = plan_text(plan)?;
    let Some(rest) = text.strip_prefix("blinded(") else {
        return Err(unsupported("LWK requires a confidential descriptor"));
    };
    let text = match plan.branches[0].liquid_blinding() {
        Blinding::Slip77(hex) => {
            // `^blinded\(slip77\([^)]*\),` -> `ct(slip77(<hex>),`
            let inner = rest
                .strip_prefix("slip77(")
                .and_then(|r| r.split_once(')'))
                .and_then(|(_, tail)| tail.strip_prefix(','));
            match inner {
                Some(tail) => format!("ct(slip77({hex}),{tail}"),
                None => format!("blinded({rest}"),
            }
        }
        Blinding::PrivateView => {
            // `^blinded\([^,]+,` -> `ct(<key>,`; the key spelling is unchanged.
            match rest.split_once(',') {
                Some((key, tail)) if !key.is_empty() => format!("ct({key},{tail}"),
                _ => format!("blinded({rest}"),
            }
        }
        Blinding::Unusable => return Err(unsupported("LWK requires private view material")),
    };
    translate_wrapper(&text)
}

/// The constructed descriptor's own spelling (with checksum), as Python's
/// `str(lwk.WolletDescriptor(text))`.
pub fn wollet_descriptor_string(plan: &Plan) -> Result<String> {
    wollet_descriptor(plan).map(|d| d.to_string())
}

pub(crate) fn wollet_descriptor(plan: &Plan) -> Result<WolletDescriptor> {
    WolletDescriptor::from_str(&descriptor_text(plan)?)
        .map_err(|_| unsupported("LWK cannot construct this confidential descriptor"))
}

/// `backends.backend_value` over `str_or_none`: the first non-blank value.
/// The native fetch boundary also reads the nested saved `config` mapping.
fn value(backend: &Value, key: &str) -> Option<String> {
    [&backend[key], &backend["config"][key]]
        .into_iter()
        .filter_map(|v| match v {
            Value::Null => None,
            Value::Bool(true) => Some("True".to_owned()),
            Value::Bool(false) => Some("False".to_owned()),
            Value::String(s) => Some(kassiber_pytext::strip(s).to_owned()),
            other => Some(other.to_string()),
        })
        .find(|v| !v.is_empty())
}

/// `util.parse_bool(value, default=False)`.
fn boolean(value: Option<String>) -> Result<bool> {
    let Some(value) = value else {
        return Ok(false);
    };
    match value.to_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(AppError::new(
            "app_error",
            format!("Invalid boolean value: {value}"),
        )),
    }
}

fn onion(url: &str) -> bool {
    let raw = kassiber_pytext::strip(url);
    let rest = raw.split_once("://").map_or(raw, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = if let Some(bracketed) = host.strip_prefix('[') {
        bracketed.split(']').next().unwrap_or("")
    } else {
        host.split(':').next().unwrap_or("")
    };
    host.trim_end_matches('.')
        .to_ascii_lowercase()
        .ends_with(".onion")
}

/// `lwk_compatibility_reason` for a Liquid descriptor plan: `None` selects
/// native LWK over Electrum. Every other reason names a route this port does
/// not carry, which the fetch boundary refuses before any connection.
pub fn compatibility_reason(backend: &Value, plan: &Plan) -> Result<Option<&'static str>> {
    let kind = kassiber_pytext::strip(&kassiber_envelope::py_str(
        &kassiber_pytext::loads(&backend["kind"].to_string())
            .unwrap_or(kassiber_pyjson::Value::Null),
    ))
    .to_lowercase();
    let kind = match kind.as_str() {
        "liquid-esplora" | "mempool" => "esplora".to_owned(),
        _ => kind,
    };
    let https = kassiber_pytext::strip(backend["url"].as_str().unwrap_or(""))
        .to_ascii_lowercase()
        .starts_with("https:");
    if kind == "esplora" && boolean(value(backend, "insecure"))? {
        return Ok(Some("insecure_tls"));
    }
    if kind == "esplora" && value(backend, "certificate").is_some() {
        return Ok(Some("custom_ca"));
    }
    if kind == "esplora"
        && https
        && std::env::var("KASSIBER_HOST_CA_BUNDLE")
            .is_ok_and(|v| kassiber_pytext::strip(&v).eq_ignore_ascii_case("explicit"))
    {
        return Ok(Some("system_ca_trust"));
    }
    if !matches!(kind.as_str(), "esplora" | "electrum") {
        return Ok(Some("backend_kind"));
    }
    if value(backend, "tor_proxy").is_some()
        || value(backend, "proxy").is_some()
        || onion(backend["url"].as_str().unwrap_or(""))
    {
        return Ok(Some("proxy_transport"));
    }
    if kind == "electrum" && value(backend, "certificate").is_some() {
        return Ok(Some("custom_ca"));
    }
    if kind == "electrum" && boolean(value(backend, "insecure"))? {
        return Ok(Some("insecure_tls"));
    }
    if kind == "electrum" {
        if let Some(timeout) = value(backend, "timeout") {
            if kassiber_pytext::parse_int(&timeout).map_or(true, |t| t != 30.into()) {
                return Ok(Some("custom_timeout"));
            }
        }
    }
    match plan.branches.len() {
        1 => {}
        2 => {
            let selectors: Vec<_> = plan.branches.iter().map(|b| b.selector).collect();
            if selectors != [Some(0), Some(1)] && canonical_multipath(plan).is_none() {
                return Ok(Some("separate_change_descriptor"));
            }
        }
        _ => return Ok(Some("noncanonical_multipath")),
    }
    if wollet_descriptor(plan).is_err() {
        return Ok(Some("descriptor_unsupported"));
    }
    if kind == "esplora" {
        return Ok(Some("http_route_policy"));
    }
    Ok(None)
}
