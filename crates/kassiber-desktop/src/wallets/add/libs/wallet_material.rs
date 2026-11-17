//! lib/walletMaterialFormat.ts: client-side detection of what was pasted into the "wallet
//! export" field (inline feedback only; the daemon validates for real).

use serde_json::Value;

use crate::i18n::{t, tr};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    DescriptorJson,
    Bsms,
    Descriptor,
    Slip132,
    BareXpub,
    Empty,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Detection {
    pub kind: Kind,
    pub label: String,
    pub hint: Option<String>,
}

/// Script types a bare xpub/tpub can be wrapped into, modern first (`BARE_XPUB_SCRIPT_TYPES`).
pub const BARE_XPUB_SCRIPT_TYPES: [&str; 4] = ["p2wpkh", "p2sh-p2wpkh", "p2pkh", "p2tr"];

const DESCRIPTOR_PREFIXES: [&str; 13] = [
    "pkh(", "wpkh(", "sh(", "wsh(", "tr(", "combo(", "addr(", "raw(", "ct(", "elwpkh(", "elwsh(",
    "elsh(", "eltr(",
];

fn slip132_label(head: &str) -> Option<String> {
    matches!(head, "ypub" | "zpub" | "upub" | "vpub")
        .then(|| t(&format!("connections:add.descriptor.material.{head}")))
}

/// `scriptTypesFromDetectionPayload`: Ok(types) after a real probe (native SegWit when the
/// probe found none), Err(reason) when the backend could not be probed.
pub fn script_types_from_detection_payload(
    payload: Option<&Value>,
) -> Result<Vec<String>, Option<String>> {
    let probed = payload
        .and_then(|p| p.get("probed"))
        .and_then(Value::as_bool)
        == Some(true);
    if !probed {
        let reason = payload
            .and_then(|p| p.get("reason"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .map(str::to_owned);
        return Err(reason);
    }
    let active: Vec<String> = payload
        .and_then(|p| p.get("active"))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .filter(|s| BARE_XPUB_SCRIPT_TYPES.contains(s))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    Ok(if active.is_empty() {
        vec![BARE_XPUB_SCRIPT_TYPES[0].to_owned()]
    } else {
        active
    })
}

/// `detectWalletMaterial`.
pub fn detect(value: &str) -> Detection {
    // Labels and hints are shown inline in the setup form, so they are translated here.
    let d = |kind, label: &str, hint: Option<&str>| Detection {
        kind,
        label: t(&format!("connections:add.descriptor.material.{label}")),
        hint: hint.map(|h| t(&format!("connections:add.descriptor.material.{h}"))),
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return d(Kind::Empty, "empty", None);
    }
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        return d(Kind::DescriptorJson, "descriptorJson", None);
    }
    let lower = trimmed.to_lowercase();
    if lower.starts_with("bsms 1.0") {
        return d(Kind::Bsms, "bsms", None);
    }
    if DESCRIPTOR_PREFIXES.iter().any(|p| lower.starts_with(p)) {
        return d(Kind::Descriptor, "outputDescriptor", None);
    }
    // `slice(0, 4)` counts UTF-16 units; for the ASCII heads that matter, chars are the same.
    let head: String = trimmed.chars().take(4).collect();
    if let Some(label) = slip132_label(&head) {
        return Detection {
            kind: Kind::Slip132,
            label: format!("{head} · {label}"),
            hint: None,
        };
    }
    if head == "xpub" || head == "tpub" {
        return Detection {
            kind: Kind::BareXpub,
            label: tr(
                "connections:add.descriptor.material.bareXpub",
                &[("head", &head)],
            ),
            hint: Some(t("connections:add.descriptor.material.bareXpubHint")),
        };
    }
    d(Kind::Unknown, "unrecognized", Some("unrecognizedHint"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn detects_formats() {
        assert_eq!(detect("   ").kind, Kind::Empty);
        assert_eq!(detect(r#"{"descriptors":[]}"#).kind, Kind::DescriptorJson);
        assert_eq!(detect("[]").kind, Kind::DescriptorJson);
        assert_eq!(detect("wpkh(xpub.../0/*)").kind, Kind::Descriptor);
        assert_eq!(detect("sh(wpkh(xpub.../0/*))").kind, Kind::Descriptor);
        assert_eq!(detect("ct(slip77(...),elwpkh(...))").kind, Kind::Descriptor);
        assert_eq!(
            detect("BSMS 1.0\nwsh(sortedmulti(2,xpubA/**,xpubB/**))\n/0/*,/1/*\nbc1q...").kind,
            Kind::Bsms
        );
        for s in ["zpub6r...", "ypub6W...", "vpub5T...", "upub5E..."] {
            assert_eq!(detect(s).kind, Kind::Slip132);
        }
        assert_eq!(detect("xpub6C...").kind, Kind::BareXpub);
        assert_eq!(detect("tpub5...").kind, Kind::BareXpub);
        assert!(detect("xpub6C...").hint.is_some());
        assert_eq!(detect("hello world").kind, Kind::Unknown);
    }

    #[test]
    fn failed_probe_blocks() {
        let p = json!({"probed": false, "active": ["p2wpkh"], "reason": "backend unavailable"});
        assert_eq!(
            script_types_from_detection_payload(Some(&p)),
            Err(Some("backend unavailable".to_owned()))
        );
        assert_eq!(script_types_from_detection_payload(None), Err(None));
    }

    #[test]
    fn empty_probe_falls_back_to_native_segwit() {
        let p = json!({"probed": true, "active": []});
        assert_eq!(
            script_types_from_detection_payload(Some(&p)),
            Ok(vec!["p2wpkh".to_owned()])
        );
    }

    #[test]
    fn keeps_supported_names_only() {
        let p = json!({"probed": true, "active": ["p2tr", "p2wsh", 1, "p2wpkh"]});
        assert_eq!(
            script_types_from_detection_payload(Some(&p)),
            Ok(vec!["p2tr".to_owned(), "p2wpkh".to_owned()])
        );
    }
}
