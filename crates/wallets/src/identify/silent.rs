//! `silent_payments.receive_keys`: the public keys behind a silent-payment
//! wallet's addresses, for matching a pasted `sp1…` address. The scan secret
//! only ever derives public keys here and is never returned.
use super::codec::bech32m_payload;
use crate::descriptors;
use bitcoin::bip32::{DerivationPath, Xpriv, Xpub};
use bitcoin::secp256k1::{PublicKey, Scalar, Secp256k1, SecretKey};
use kassiber_envelope::AppError;
use sha2::{Digest, Sha256};
use std::str::FromStr;

/// Labels 0 (change) through 50 are checked; later labels stay "unknown".
pub const LABEL_CHECK_LIMIT: u32 = 50;

pub struct ReceiveKeys {
    pub network: String,
    pub scan_pubkey: Vec<u8>,
    /// `(None, B_spend)` for the unlabeled address, `(Some(m), B_m)` per label.
    pub spend_pubkeys: Vec<(Option<u32>, Vec<u8>)>,
}

fn strip_origin(value: &str) -> &str {
    let text = kassiber_pytext::strip(value);
    match text.strip_prefix('[').and_then(|rest| rest.find(']')) {
        Some(end) => &text[end + 2..],
        None => text,
    }
}

fn key_and_path(expr: &str) -> (&str, &str) {
    let text = strip_origin(expr);
    text.split_once('/').unwrap_or((text, ""))
}

/// `_descriptor_args(_compact_descriptor(material))`, `None` where it raises.
fn descriptor_args(material: &str) -> Option<Vec<String>> {
    let compact: String = material
        .chars()
        .filter(|c| !kassiber_pytext::is_space(*c as u32))
        .collect();
    let text = compact.split('#').next().unwrap_or("");
    if !text.to_lowercase().starts_with("sp(") || !text.ends_with(')') {
        return None;
    }
    let inner = text.get(3..text.len() - 1)?;
    let mut args = vec![];
    let (mut depth, mut start) = (0i64, 0);
    for (at, char) in inner.char_indices() {
        match char {
            '(' | '[' => depth += 1,
            ')' | ']' => {
                depth -= 1;
                if depth < 0 {
                    return None;
                }
            }
            ',' if depth == 0 => {
                args.push(&inner[start..at]);
                start = at + 1;
            }
            _ => {}
        }
    }
    if depth != 0 {
        return None;
    }
    args.push(&inner[start..]);
    let args: Vec<String> = args
        .into_iter()
        .map(|arg| kassiber_pytext::strip(arg).to_owned())
        .filter(|arg| !arg.is_empty())
        .collect();
    (!args.is_empty()).then_some(args)
}

fn regex(pattern: &'static str) -> regex::Regex {
    regex::Regex::new(pattern).expect("static pattern")
}

/// Decode an extended key of any SLIP132 family by re-tagging it with the
/// plain xpub/xprv version bytes, as embit's `HDKey.from_string` accepts.
fn extended(key: &str, private: bool) -> Option<Vec<u8>> {
    let mut raw = bitcoin::base58::decode_check(key).ok()?;
    if raw.len() != 78 {
        return None;
    }
    raw[..4].copy_from_slice(if private {
        &[0x04, 0x88, 0xad, 0xe4]
    } else {
        &[0x04, 0x88, 0xb2, 0x1e]
    });
    Some(raw)
}

fn path(text: &str) -> Option<DerivationPath> {
    if text.is_empty() {
        Some(DerivationPath::master())
    } else {
        DerivationPath::from_str(&format!("m/{text}")).ok()
    }
}

fn scan_secret(expr: &str) -> Option<[u8; 32]> {
    let (key, steps) = key_and_path(expr);
    if steps.contains('*') {
        return None;
    }
    let secp = Secp256k1::new();
    if regex(r"^(?:xprv|tprv|yprv|zprv|uprv|vprv)[1-9A-HJ-NP-Za-km-z]{20,}\n?$").is_match(key) {
        let node = Xpriv::decode(&extended(key, true)?).ok()?;
        return Some(
            node.derive_priv(&secp, &path(steps)?)
                .ok()?
                .private_key
                .secret_bytes(),
        );
    }
    if regex(r"^[KLc][1-9A-HJ-NP-Za-km-z]{20,}\n?$").is_match(key) {
        return Some(
            bitcoin::PrivateKey::from_wif(key)
                .ok()?
                .inner
                .secret_bytes(),
        );
    }
    None
}

fn spend_pubkey(expr: &str) -> Option<Vec<u8>> {
    let (key, steps) = key_and_path(expr);
    if steps.contains('*') || steps.contains('\'') || steps.to_lowercase().contains('h') {
        return None;
    }
    if steps.is_empty() && regex(r"^(?:02|03)[0-9a-fA-F]{64}\n?$").is_match(key) {
        return (0..66)
            .step_by(2)
            .map(|at| u8::from_str_radix(&key[at..at + 2], 16).ok())
            .collect();
    }
    if regex(r"^(?:xpub|tpub|ypub|zpub|upub|vpub)[1-9A-HJ-NP-Za-km-z]{20,}\n?$").is_match(key) {
        let node = Xpub::decode(&extended(key, false)?).ok()?;
        let secp = Secp256k1::verification_only();
        return Some(
            node.derive_pub(&secp, &path(steps)?)
                .ok()?
                .public_key
                .serialize()
                .to_vec(),
        );
    }
    None
}

fn label_tweak(scan: &[u8; 32], label: u32) -> [u8; 32] {
    let tag = Sha256::digest(b"BIP0352/Label");
    let mut hash = Sha256::new();
    hash.update(tag);
    hash.update(tag);
    hash.update(scan);
    hash.update(label.to_be_bytes());
    hash.finalize().into()
}

/// `None` means "cannot check" (missing or underivable material), never
/// "not this wallet's". An unsupported network refuses like Python.
pub fn receive_keys(config: &serde_json::Value) -> Result<Option<ReceiveKeys>, AppError> {
    let material = kassiber_pytext::strip(&descriptors::field(config, "sp_descriptor")).to_owned();
    if material.is_empty() {
        return Ok(None);
    }
    let Some(args) = descriptor_args(&material) else {
        return Ok(None);
    };
    let (scan, spend) = match args.as_slice() {
        [single] => match bech32m_payload(strip_origin(single), &["spscan", "tspscan"]) {
            Some((_, payload)) if payload.len() == 65 => {
                let mut scan = [0u8; 32];
                scan.copy_from_slice(&payload[..32]);
                (Some(scan), Some(payload[32..].to_vec()))
            }
            _ => return Ok(None),
        },
        [scan, spend] => (scan_secret(scan), spend_pubkey(spend)),
        _ => return Ok(None),
    };
    let (Some(scan), Some(spend)) = (scan, spend) else {
        return Ok(None);
    };
    let secp = Secp256k1::new();
    let derived = (|| {
        let scan_pubkey = PublicKey::from_secret_key(&secp, &SecretKey::from_slice(&scan).ok()?);
        let spend_point = PublicKey::from_slice(&spend).ok()?;
        let mut spend_pubkeys = vec![(None, spend.clone())];
        for label in 0..=LABEL_CHECK_LIMIT {
            let tweak = Scalar::from_be_bytes(label_tweak(&scan, label)).ok()?;
            let labeled = spend_point.add_exp_tweak(&secp, &tweak).ok()?;
            spend_pubkeys.push((Some(label), labeled.serialize().to_vec()));
        }
        Some((scan_pubkey.serialize().to_vec(), spend_pubkeys))
    })();
    let Some((scan_pubkey, spend_pubkeys)) = derived else {
        return Ok(None);
    };
    let network = descriptors::normalize_network("bitcoin", &descriptors::field(config, "network"))
        .map_err(descriptors::Error::into_app)?;
    Ok(Some(ReceiveKeys {
        network,
        scan_pubkey,
        spend_pubkeys,
    }))
}
