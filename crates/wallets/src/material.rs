//! Wallet-export intake. Normalization never grants ownership or network consent.
use crate::descriptors::{self as d, Error, Result};
use kassiber_envelope::AppError;
use serde_json::{json, Value};

fn validation(message: impl Into<String>) -> Error {
    AppError::new("validation", message).into()
}
fn invalid_script(kind: &str) -> Error {
    AppError::new("validation", format!("Unsupported script type '{kind}'"))
        .with_hint("Supported script types are p2pkh, p2sh-p2wpkh, p2wpkh, and p2tr.")
        .into()
}

pub fn script_types(values: &Value) -> Result<Vec<String>> {
    let values = match values {
        Value::Null => return Ok(vec![]),
        Value::Array(values) => values.clone(),
        _ => vec![values.clone()],
    };
    let mut result = std::collections::BTreeSet::new();
    for value in values {
        let kind = if d::truthy(&value) {
            kassiber_pytext::strip(&d::text(&value)).to_lowercase()
        } else {
            String::new()
        };
        if kind.is_empty() {
            continue;
        }
        if !d::SCRIPT_TYPES.contains(&kind.as_str()) {
            return Err(invalid_script(&kind));
        }
        result.insert(kind);
    }
    Ok(result.into_iter().collect())
}
fn lines(text: &str) -> Vec<String> {
    text.replace('\r', "\n")
        .split('\n')
        .map(|s| kassiber_pytext::strip(s).to_owned())
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
        .collect()
}
fn looks_like(text: &str) -> bool {
    [
        "pkh(", "wpkh(", "sh(", "wsh(", "tr(", "combo(", "addr(", "raw(", "ct(", "elwpkh(",
        "elwsh(", "elsh(", "eltr(",
    ]
    .iter()
    .any(|p| text.to_lowercase().starts_with(p))
}
pub fn bsms(text: &str) -> Result<Option<Value>> {
    let lines = lines(text);
    if !lines
        .first()
        .is_some_and(|s| s.to_lowercase() == "bsms 1.0")
    {
        return Ok(None);
    }
    if lines.len() < 4 {
        return Err(AppError::new("validation","Incomplete BSMS descriptor record").with_hint("Paste the plaintext coordinator descriptor record, not an encrypted .dat payload.").into());
    }
    let descriptor = &lines[1];
    if !looks_like(descriptor) {
        return Err(
            AppError::new("validation", "BSMS key records are not wallet descriptors")
                .with_hint("Paste the coordinator descriptor record, not a signer key record.")
                .into(),
        );
    }
    if lines[2].to_lowercase() == "no path restrictions" {
        if descriptor.contains("/**") {
            return Err(validation(
                "BSMS descriptor template requires path restrictions",
            ));
        }
        return Ok(Some(
            json!({"descriptor":descriptor,"descriptor_source":"bsms","synthesize_change":false}),
        ));
    }
    let restrictions: Vec<_> = lines[2]
        .split(',')
        .map(kassiber_pytext::strip)
        .filter(|s| !s.is_empty())
        .collect();
    if restrictions.is_empty() {
        return Err(validation(
            "BSMS descriptor record has no path restrictions",
        ));
    }
    let regex = regex::Regex::new(r"^/(?:\d+/)*(\*|\d+)$").expect("constant regex");
    for restriction in &restrictions {
        if !regex.is_match(restriction) {
            return Err(validation(
                "BSMS path restrictions must be non-hardened paths like /0/*",
            ));
        }
    }
    if !descriptor.contains("/**") {
        return Err(validation(
            "BSMS path restrictions require a descriptor template containing /**",
        ));
    }
    if restrictions.len() > 2 {
        return Err(validation(
            "BSMS records with more than receive/change path restrictions are not supported",
        ));
    }
    let mut result = json!({"descriptor":descriptor.replace("/**",restrictions[0]),"descriptor_source":"bsms","synthesize_change":false});
    if restrictions.len() > 1 {
        result["change_descriptor"] = json!(descriptor.replace("/**", restrictions[1]));
    }
    Ok(Some(result))
}
fn first(payload: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        payload
            .get(key)
            .and_then(Value::as_str)
            .map(kassiber_pytext::strip)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    })
}
fn from_json(payload: &Value) -> Option<Value> {
    let mut receive = first(
        payload,
        &[
            "descriptor",
            "receive_descriptor",
            "external_descriptor",
            "receive",
            "external",
        ],
    );
    let mut change = first(
        payload,
        &[
            "change_descriptor",
            "internal_descriptor",
            "change",
            "internal",
        ],
    );
    if let Some(descriptors) = payload.get("descriptors").and_then(Value::as_array) {
        for item in descriptors {
            if item.get("active") == Some(&Value::Bool(false)) {
                continue;
            }
            let descriptor = first(item, &["desc", "descriptor"]);
            if item.get("internal").is_some_and(d::truthy) {
                if change.is_none() {
                    change = descriptor;
                }
            } else if receive.is_none() {
                receive = descriptor;
            }
        }
    }
    receive.map(|receive| pair(receive, change))
}
fn pair(receive: String, change: Option<String>) -> Value {
    let mut result = json!({"descriptor":receive});
    if let Some(change) = change {
        result["change_descriptor"] = json!(change);
    }
    result
}
fn from_text(text: &str) -> Option<Value> {
    let mut receive = None;
    let mut change = None;
    let mut descriptors = vec![];
    for line in lines(text) {
        if let Some((key, value)) = line.split_once('=').or_else(|| line.split_once(':')) {
            let key = kassiber_pytext::strip(key).to_lowercase().replace('-', "_");
            if [
                "descriptor",
                "receive",
                "receive_descriptor",
                "external",
                "external_descriptor",
            ]
            .contains(&key.as_str())
            {
                receive = Some(kassiber_pytext::strip(value).to_owned());
                continue;
            }
            if [
                "change",
                "change_descriptor",
                "internal",
                "internal_descriptor",
            ]
            .contains(&key.as_str())
            {
                change = Some(kassiber_pytext::strip(value).to_owned());
                continue;
            }
        }
        if looks_like(&line) {
            descriptors.push(line);
        }
    }
    receive
        .or_else(|| descriptors.first().cloned())
        .map(|receive| pair(receive, change.or_else(|| descriptors.get(1).cloned())))
}
fn decode(key: &str) -> Result<Vec<u8>> {
    if key
        .chars()
        .any(|c| !"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz".contains(c))
    {
        return Err(validation("Invalid base58 extended public key"));
    }
    match bitcoin::base58::decode_check(key) {
        Ok(raw) => Ok(raw),
        Err(_) => {
            let raw = bitcoin::base58::decode(key).unwrap_or_default();
            Err(validation(if raw.len() < 5 {
                "Invalid base58check payload"
            } else {
                "Invalid extended public key checksum"
            }))
        }
    }
}
fn validate(value: Value) -> Result<Value> {
    d::assert_standalone_watch_only(&d::field(&value, "xpub"))?;
    d::assert_watch_only(&d::field(&value, "descriptor"))?;
    d::assert_watch_only(&d::field(&value, "change_descriptor"))?;
    Ok(value)
}

pub fn normalize(input: &str, options: &Value) -> Result<Value> {
    let material = kassiber_pytext::strip(input);
    if material.is_empty() {
        return Err(AppError::new("validation", "Wallet export is required")
            .with_hint("Paste a descriptor, descriptor export, or supported extended public key.")
            .into());
    }
    d::assert_standalone_watch_only(material)?;
    if let Ok(payload) = serde_json::from_str::<Value>(material) {
        if let Some(parsed) = from_json(&payload) {
            return validate(parsed);
        }
    }
    if let Some(parsed) = bsms(material)? {
        return validate(parsed);
    }
    if let Some(parsed) = from_text(material) {
        return validate(parsed);
    }
    let prefix = material.get(..4).unwrap_or(material);
    if ["ypub", "zpub", "upub", "vpub"].contains(&prefix) {
        let mut raw = decode(material)?;
        if raw.len() < 4 {
            return Err(validation("Invalid extended public key"));
        }
        let main = prefix == "ypub" || prefix == "zpub";
        raw[..4].copy_from_slice(if main {
            &[0x04, 0x88, 0xb2, 0x1e]
        } else {
            &[0x04, 0x35, 0x87, 0xcf]
        });
        let key = bitcoin::base58::encode_check(&raw);
        let kind = if prefix == "ypub" || prefix == "upub" {
            "p2sh-p2wpkh"
        } else {
            "p2wpkh"
        };
        let mut result = pair(
            d::template(kind, &key, "0"),
            Some(d::template(kind, &key, "1")),
        );
        result["network"] = json!(if main { "main" } else { "test" });
        return Ok(result);
    }
    if prefix == "xpub" || prefix == "tpub" {
        let types = script_types(&options["script_types"])?;
        let network = if prefix == "xpub" { "main" } else { "test" };
        if !types.is_empty() {
            decode(material)?;
            return validate(json!({"xpub":material,"script_types":types,"network":network}));
        }
        if let Some(kind) = options["script_type"].as_str().filter(|s| !s.is_empty()) {
            if !d::SCRIPT_TYPES.contains(&kind) {
                return Err(invalid_script(kind));
            }
            decode(material)?;
            let mut result = pair(
                d::template(kind, material, "0"),
                Some(d::template(kind, material, "1")),
            );
            result["network"] = json!(network);
            return validate(result);
        }
        return Err(AppError::new("validation", "Bare xpub/tpub is ambiguous")
            .with_hint(
                "Paste an output descriptor or a wallet export that includes the script type.",
            )
            .into());
    }
    Err(AppError::new("validation","Unsupported wallet export format").with_hint("Paste a descriptor, a BSMS descriptor record, Bitcoin Core descriptor JSON, Sparrow-style descriptor text, or a ypub/zpub/upub/vpub key.").into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn python_material_vectors() {
        let vectors: Value =
            serde_json::from_str(include_str!("../tests/vectors/python_descriptors.json")).unwrap();
        for case in vectors["materials"].as_array().unwrap() {
            let actual = match normalize(case["material"].as_str().unwrap(), &case["options"]) {
                Ok(v) => json!({"ok":v}),
                Err(e) => e.projection(),
            };
            assert_eq!(actual, case["expected"], "{}", case["name"]);
        }
    }
}
