//! lib/samouraiSourceSet.ts: turn the four Samourai account fields (Deposit, Badbank, Premix,
//! Postmix) into the watch-only source set `ui.wallets.import_samourai` takes.

use serde_json::{Map, Value, json};

pub struct Fields<'a> {
    pub deposit: &'a str,
    pub badbank: &'a str,
    pub premix: &'a str,
    pub postmix: &'a str,
}

pub struct Built {
    /// `{network, children: [...], xpubs: [...]}`.
    pub source_set: Value,
    pub children: usize,
    pub xpubs: usize,
    /// (section, message) per populated field that could not be read.
    pub errors: Vec<(&'static str, String)>,
}

impl Built {
    #[cfg(test)]
    pub fn error(&self, section: &str) -> Option<&str> {
        self.errors
            .iter()
            .find(|(s, _)| *s == section)
            .map(|(_, m)| m.as_str())
    }
}

const DESCRIPTOR_PREFIXES: [&str; 5] = ["pkh(", "sh(", "wpkh(", "wsh(", "tr("];
const XPUB_PREFIXES: [&str; 6] = ["xpub", "tpub", "ypub", "upub", "zpub", "vpub"];

fn coin_type(network: &str) -> u32 {
    let n = network.to_lowercase();
    if n == "main" || n == "mainnet" { 0 } else { 1 }
}

fn root_path(section: &str, script_type: &str, network: &str) -> String {
    let c = coin_type(network);
    if section == "deposit" {
        return match script_type {
            "p2pkh" => format!("m/44'/{c}'/0'"),
            "p2sh-p2wpkh" => format!("m/49'/{c}'/0'"),
            _ => format!("m/84'/{c}'/0'"),
        };
    }
    let root = match section {
        "badbank" => 2_147_483_644u32,
        "premix" => 2_147_483_645,
        _ => 2_147_483_646,
    };
    format!("m/84'/{c}'/{root}'")
}

fn script_type_from_public_key(material: &str, section: &str) -> Option<&'static str> {
    let prefix: String = material.chars().take(4).collect::<String>().to_lowercase();
    if section != "deposit" {
        return Some("p2wpkh");
    }
    match prefix.as_str() {
        "ypub" | "upub" => Some("p2sh-p2wpkh"),
        "zpub" | "vpub" => Some("p2wpkh"),
        _ => None,
    }
}

fn script_type_from_descriptor(d: &str) -> &'static str {
    let n: String = d
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_lowercase();
    if n.starts_with("pkh(") {
        "p2pkh"
    } else if n.starts_with("sh(wpkh(") {
        "p2sh-p2wpkh"
    } else {
        "p2wpkh"
    }
}

fn looks_like_descriptor(v: &str) -> bool {
    let l = v.to_lowercase();
    DESCRIPTOR_PREFIXES.iter().any(|p| l.starts_with(p))
}

fn looks_like_xpub(v: &str) -> bool {
    let l = v.to_lowercase();
    XPUB_PREFIXES.iter().any(|p| l.starts_with(p))
}

enum Material {
    Xpub(String),
    Descriptors(String, Option<String>),
}

fn first_string(obj: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| {
        obj.get(*k)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    })
}

fn from_json(material: &str) -> Option<Material> {
    let v: Value = serde_json::from_str(material).ok()?;
    let obj = v.as_object()?;
    let mut receive = first_string(
        obj,
        &[
            "descriptor",
            "receive_descriptor",
            "external_descriptor",
            "receive",
            "external",
        ],
    );
    let mut change = first_string(
        obj,
        &[
            "change_descriptor",
            "internal_descriptor",
            "change",
            "internal",
        ],
    );
    if let Some(items) = obj.get("descriptors").and_then(Value::as_array) {
        for item in items {
            let Some(item) = item.as_object() else {
                continue;
            };
            if item.get("active") == Some(&Value::Bool(false)) {
                continue;
            }
            let Some(d) = first_string(item, &["desc", "descriptor"]) else {
                continue;
            };
            let internal = item.get("internal").is_some_and(truthy);
            if internal && change.is_none() {
                change = Some(d);
            } else if !internal && receive.is_none() {
                receive = Some(d);
            }
        }
    }
    receive.map(|r| Material::Descriptors(r, change))
}

/// JavaScript truthiness for a JSON value.
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

fn from_text(material: &str) -> Option<Material> {
    let text = material.replace('\r', "\n");
    let mut receive: Option<String> = None;
    let mut change: Option<String> = None;
    let mut descriptors: Vec<String> = Vec::new();
    for line in text
        .split('\n')
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let delim = if line.contains('=') {
            Some('=')
        } else if line.contains(':') {
            Some(':')
        } else {
            None
        };
        if let Some(d) = delim {
            let (raw_key, value) = line.split_once(d).unwrap_or((line, ""));
            let key = raw_key.trim().to_lowercase().replace('-', "_");
            let value = value.trim().to_owned();
            if [
                "descriptor",
                "receive",
                "receive_descriptor",
                "external",
                "external_descriptor",
            ]
            .contains(&key.as_str())
            {
                receive = Some(value);
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
                change = Some(value);
                continue;
            }
        }
        if looks_like_descriptor(line) {
            descriptors.push(line.to_owned());
        }
    }
    // JS `!receive` treats an empty string as missing too.
    if receive.as_deref().is_none_or(str::is_empty) && !descriptors.is_empty() {
        receive = Some(descriptors[0].clone());
    }
    if change.as_deref().is_none_or(str::is_empty) && descriptors.len() > 1 {
        change = Some(descriptors[1].clone());
    }
    receive
        .filter(|r| !r.is_empty())
        .map(|r| Material::Descriptors(r, change.filter(|c| !c.is_empty())))
}

fn parse_material(material: &str) -> Option<Material> {
    let t = material.trim();
    if t.is_empty() {
        return None;
    }
    if looks_like_xpub(t) {
        return Some(Material::Xpub(t.to_owned()));
    }
    from_json(t).or_else(|| from_text(t))
}

/// `buildSamouraiSourceSet` (`network` defaults to "main").
pub fn build_source_set(fields: Fields, network: Option<&str>) -> Built {
    let network = network.unwrap_or("main");
    let mut children: Vec<Value> = Vec::new();
    let mut xpubs: Vec<Value> = Vec::new();
    let mut errors: Vec<(&'static str, String)> = Vec::new();
    for (section, raw) in [
        ("deposit", fields.deposit),
        ("badbank", fields.badbank),
        ("premix", fields.premix),
        ("postmix", fields.postmix),
    ] {
        let material = raw.trim();
        if material.is_empty() {
            continue;
        }
        match parse_material(material) {
            None => errors.push((section, "Paste an output descriptor, two receive/change descriptors, or an xpub/ypub/zpub.".to_owned())),
            Some(Material::Xpub(x)) => match script_type_from_public_key(&x, section) {
                None => errors.push((section, "Bare Deposit xpub is ambiguous; paste a descriptor, ypub, or zpub instead.".to_owned())),
                Some(st) => xpubs.push(json!({"section": section, "script_type": st, "root_path": root_path(section, st, network), "xpub": x})),
            },
            Some(Material::Descriptors(d, change)) => {
                let st = script_type_from_descriptor(&d);
                let mut c = json!({"section": section, "script_type": st, "root_path": root_path(section, st, network), "descriptor": d});
                if let Some(ch) = change {
                    c["change_descriptor"] = json!(ch);
                }
                children.push(c);
            }
        }
    }
    Built {
        children: children.len(),
        xpubs: xpubs.len(),
        source_set: json!({"network": network, "children": children, "xpubs": xpubs}),
        errors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMPTY: Fields<'static> = Fields {
        deposit: "",
        badbank: "",
        premix: "",
        postmix: "",
    };

    #[test]
    fn four_fields_become_xpub_sources() {
        let b = build_source_set(
            Fields {
                deposit: "zpub6deposit",
                badbank: "xpub6badbank",
                premix: "xpub6premix",
                postmix: "xpub6postmix",
            },
            None,
        );
        assert!(b.errors.is_empty());
        assert_eq!(
            b.source_set["xpubs"],
            json!([
                {"section": "deposit", "script_type": "p2wpkh", "root_path": "m/84'/0'/0'", "xpub": "zpub6deposit"},
                {"section": "badbank", "script_type": "p2wpkh", "root_path": "m/84'/0'/2147483644'", "xpub": "xpub6badbank"},
                {"section": "premix", "script_type": "p2wpkh", "root_path": "m/84'/0'/2147483645'", "xpub": "xpub6premix"},
                {"section": "postmix", "script_type": "p2wpkh", "root_path": "m/84'/0'/2147483646'", "xpub": "xpub6postmix"},
            ])
        );
        assert_eq!(b.source_set["children"], json!([]));
        assert_eq!((b.children, b.xpubs), (0, 4));
    }

    #[test]
    fn receive_change_descriptor_text() {
        let text = "descriptor=wpkh([00000000/84'/0'/2147483646']xpub/0/*)\nchange_descriptor=wpkh([00000000/84'/0'/2147483646']xpub/1/*)";
        let b = build_source_set(
            Fields {
                postmix: text,
                ..EMPTY
            },
            None,
        );
        assert!(b.errors.is_empty());
        assert_eq!(
            b.source_set["children"],
            json!([{
                "section": "postmix", "script_type": "p2wpkh", "root_path": "m/84'/0'/2147483646'",
                "descriptor": "wpkh([00000000/84'/0'/2147483646']xpub/0/*)",
                "change_descriptor": "wpkh([00000000/84'/0'/2147483646']xpub/1/*)",
            }])
        );
    }

    #[test]
    fn reports_unknown_fields() {
        let b = build_source_set(
            Fields {
                premix: "not a descriptor",
                ..EMPTY
            },
            None,
        );
        assert_eq!((b.children, b.xpubs), (0, 0));
        assert!(
            b.error("premix")
                .unwrap()
                .contains("Paste an output descriptor")
        );
    }

    #[test]
    fn bare_deposit_xpub_is_ambiguous() {
        let b = build_source_set(
            Fields {
                deposit: "xpub6deposit",
                ..EMPTY
            },
            None,
        );
        assert_eq!((b.children, b.xpubs), (0, 0));
        assert!(
            b.error("deposit")
                .unwrap()
                .contains("Bare Deposit xpub is ambiguous")
        );
    }

    #[test]
    fn testnet_coin_type_and_json_descriptors() {
        let j = r#"{"descriptors":[{"desc":"sh(wpkh(tpubX/0/*))","internal":false},{"desc":"sh(wpkh(tpubX/1/*))","internal":true}]}"#;
        let b = build_source_set(
            Fields {
                deposit: j,
                ..EMPTY
            },
            Some("regtest"),
        );
        assert_eq!(
            b.source_set["children"][0]["root_path"],
            json!("m/49'/1'/0'")
        );
        assert_eq!(
            b.source_set["children"][0]["change_descriptor"],
            json!("sh(wpkh(tpubX/1/*))")
        );
    }
}
