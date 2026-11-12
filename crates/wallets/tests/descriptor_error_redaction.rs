//! Malformed multisig descriptors must fail without echoing key material.
//!
//! The keys are real-format extended public keys derived here from fixed test
//! seeds, so the parser sees genuine base58check keys; they belong to no one.
use std::str::FromStr;

use bitcoin::bip32::{DerivationPath, Xpriv, Xpub};
use bitcoin::secp256k1::Secp256k1;
use bitcoin::NetworkKind;
use kassiber_wallets::{descriptors, material};
use serde_json::{json, Value};

/// The longest run of input characters an error may repeat. Longer runs could
/// carry a recognisable piece of a key, fingerprint, or path.
const MAX_ECHO: usize = 8;

struct Cosigner {
    fingerprint: String,
    key: String,
}

fn cosigner(seed: u8, network: NetworkKind, account: u32) -> Cosigner {
    let secp = Secp256k1::new();
    let master = Xpriv::new_master(network, &[seed; 32]).expect("test seed");
    let coin = if network == NetworkKind::Main { 0 } else { 1 };
    let path = DerivationPath::from_str(&format!("m/48'/{coin}'/{account}'/2'")).expect("path");
    let xpub = Xpub::from_priv(&secp, &master.derive_priv(&secp, &path).expect("derive"));
    Cosigner {
        fingerprint: master.fingerprint(&secp).to_string(),
        key: xpub.to_string(),
    }
}

fn origin(cosigner: &Cosigner, coin: u32, account: u32) -> String {
    format!(
        "[{}/48h/{coin}h/{account}h/2h]{}/<0;1>/*",
        cosigner.fingerprint, cosigner.key
    )
}

fn multisig(first: &str, second: &str) -> String {
    format!("wsh(sortedmulti(2,{first},{second}))")
}

/// The descriptor checksum (BIP380) of `body`, so the valid descriptor carries
/// a real one and the corrupted case differs from it by one character.
fn with_checksum(body: &str) -> String {
    let parsed = miniscript::Descriptor::<miniscript::DescriptorPublicKey>::from_str(body)
        .expect("valid test descriptor");
    let checksum = parsed
        .to_string()
        .rsplit_once('#')
        .expect("checksum")
        .1
        .to_owned();
    format!("{body}#{checksum}")
}

/// Replace the character at `index` of the second cosigner's key.
fn corrupt_key(descriptor: &str, key: &str, index: usize, with: char) -> String {
    let mut corrupted = key.to_owned();
    corrupted.replace_range(index..index + 1, &with.to_string());
    assert_ne!(corrupted, key, "corruption must change the key");
    descriptor.replace(key, &corrupted)
}

fn assert_redacted(case: &str, input: &str, secrets: &[&str], error: &Value) {
    let text = error.to_string();
    let lower = text.to_ascii_lowercase();
    for marker in ["xpub", "tpub"] {
        assert!(
            !lower.contains(marker),
            "{case}: error names {marker}: {text}"
        );
    }
    for secret in secrets {
        assert!(
            !text.contains(secret),
            "{case}: error echoes {secret:?}: {text}"
        );
    }
    let chars: Vec<char> = input.chars().collect();
    for window in chars.windows(MAX_ECHO + 1) {
        let run: String = window.iter().collect();
        assert!(
            !text.contains(&run),
            "{case}: error echoes {} input characters ({run:?}): {text}",
            run.chars().count()
        );
    }
}

/// Every way the desktop daemon or CLI reads a pasted or stored descriptor:
/// the wallet config (`descriptors::load`, the receive and change slots) and
/// pasted wallet material (`material::normalize`). The daemon passes these
/// errors on through `Error::into_app`, so the projection here is what a
/// caller, a log line, or the AI tool surface can ever see.
fn errors_for(descriptor: &str, network: &str) -> Vec<(&'static str, Value)> {
    let mut errors = vec![];
    for (slot, config) in [
        (
            "load descriptor",
            json!({"chain": "bitcoin", "network": network, "descriptor": descriptor}),
        ),
        (
            "load change_descriptor",
            json!({"chain": "bitcoin", "network": network,
                   "descriptor": "wpkh(02c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5)",
                   "change_descriptor": descriptor}),
        ),
    ] {
        if let Err(error) = descriptors::load(&config) {
            errors.push((slot, error.projection()));
            errors.push((slot, app_projection(error.into_app())));
        }
    }
    if let Err(error) = material::normalize(descriptor, &json!({})) {
        errors.push(("material", error.projection()));
        errors.push(("material", app_projection(error.into_app())));
    }
    errors
}

fn app_projection(error: kassiber_envelope::AppError) -> Value {
    json!({
        "code": error.code,
        "message": error.message,
        "hint": error.hint,
        "details": error.details.map(|d| format!("{d:?}")),
    })
}

#[test]
fn malformed_multisig_descriptor_errors_do_not_echo_key_material() {
    let first = cosigner(0x11, NetworkKind::Main, 0);
    let second = cosigner(0x22, NetworkKind::Main, 0);
    let testnet = cosigner(0x33, NetworkKind::Test, 0);
    let body = multisig(&origin(&first, 0, 0), &origin(&second, 0, 0));
    let valid = with_checksum(&body);
    let checksum = valid.rsplit_once('#').unwrap().1;
    let mid = second.key.len() / 2;

    let cases: Vec<(&str, String)> = vec![
        // One checksum character changed.
        ("bad checksum", {
            let flipped = if checksum.starts_with('q') { 'p' } else { 'q' };
            format!("{body}#{flipped}{}", &checksum[1..])
        }),
        // A character base58 does not have, inside the second key.
        (
            "non-base58 character in second key",
            corrupt_key(&valid, &second.key, mid, '0'),
        ),
        // A valid base58 character that breaks the key's own checksum.
        (
            "base58check failure in second key",
            corrupt_key(
                &valid,
                &second.key,
                mid,
                if second.key[mid..].starts_with('z') {
                    'y'
                } else {
                    'z'
                },
            ),
        ),
        (
            "truncated second key",
            valid.replace(&second.key, &second.key[..second.key.len() - 9]),
        ),
        (
            "missing closing paren",
            format!("{}#{checksum}", &body[..body.len() - 1]),
        ),
        (
            "missing both closing parens",
            body[..body.len() - 2].to_owned(),
        ),
        // A testnet cosigner in a mainnet multisig.
        (
            "mixed network keys",
            multisig(&origin(&first, 0, 0), &origin(&testnet, 1, 0)),
        ),
        // An odd-length (malformed) origin fingerprint on the second key.
        (
            "odd-length fingerprint",
            valid.replace(
                &format!("[{}/", second.fingerprint),
                &format!("[{}a/", second.fingerprint),
            ),
        ),
    ];

    let secrets = [
        first.key.as_str(),
        second.key.as_str(),
        testnet.key.as_str(),
        first.fingerprint.as_str(),
        second.fingerprint.as_str(),
        testnet.fingerprint.as_str(),
    ];
    let mut failures = 0;
    for (case, input) in &cases {
        for (slot, error) in errors_for(input, "main") {
            failures += 1;
            assert_redacted(&format!("{case} via {slot}"), input, &secrets, &error);
        }
    }
    // The redaction checks above must have had errors to inspect.
    assert!(
        failures >= cases.len(),
        "only {failures} errors for {} cases",
        cases.len()
    );

    // A testnet multisig in a book configured for mainnet.
    let test_a = cosigner(0x44, NetworkKind::Test, 0);
    let test_b = cosigner(0x55, NetworkKind::Test, 1);
    let wrong_network = multisig(&origin(&test_a, 1, 0), &origin(&test_b, 1, 1));
    let errors = errors_for(&wrong_network, "main");
    assert!(
        errors.iter().any(|(slot, _)| slot.starts_with("load")),
        "a testnet descriptor must not load into a mainnet wallet"
    );
    for (slot, error) in errors {
        assert_redacted(
            &format!("wrong network via {slot}"),
            &wrong_network,
            &[
                &test_a.key,
                &test_b.key,
                &test_a.fingerprint,
                &test_b.fingerprint,
            ],
            &error,
        );
    }
}

#[test]
fn descriptor_checksum_is_not_validated() {
    // Pins current behaviour: the `#checksum` suffix is stripped before
    // parsing, so a wrong checksum on an otherwise valid descriptor loads.
    // If checksum validation is added, its error must pass the redaction test
    // above, and this pin should be replaced by a rejection test.
    let first = cosigner(0x11, NetworkKind::Main, 0);
    let second = cosigner(0x22, NetworkKind::Main, 0);
    let body = multisig(&origin(&first, 0, 0), &origin(&second, 0, 0));
    let config = json!({"chain": "bitcoin", "network": "main",
                        "descriptor": format!("{body}#qqqqqqqq")});
    assert!(descriptors::load(&config).is_ok_and(|plan| plan.is_some()));
}
