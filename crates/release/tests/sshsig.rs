//! Python's frozen `verify_signature_bytes` verdicts, produced by the system
//! `ssh-keygen -Y verify` (scripts/export_sshsig_vectors.py). The native check
//! must accept and refuse the same signatures with the same envelope fields.
use base64::Engine;
use kassiber_envelope::get;
use kassiber_pyjson::Value;
use kassiber_release::verify::verify_signature;

fn text(value: &Value) -> String {
    kassiber_envelope::py_str(value)
}

fn bytes(value: &Value) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(text(value))
        .unwrap()
}

#[test]
fn sshsig_verdicts_match_python_and_ssh_keygen() {
    let data = kassiber_pytext::loads(include_str!("data/sshsig_vectors.json")).unwrap();
    let message = bytes(get(&data, "message").unwrap());
    let Some(Value::List(cases)) = get(&data, "cases") else {
        panic!("cases")
    };
    let mut accepted = 0;
    for case in cases {
        let field = |key: &str| get(case, key).unwrap_or_else(|| panic!("{key}"));
        let name = text(field("name"));
        let public_key: Vec<u8> = text(field("public_key")).chars().map(|c| c as u8).collect();
        let outcome = verify_signature(
            &message,
            &bytes(field("signature")),
            &public_key,
            &text(field("fingerprint")),
        );
        let expected = field("result");
        match (outcome, get(expected, "signer"), get(expected, "error")) {
            (Ok(signer), Some(python), None) => {
                assert_eq!(signer, text(python), "{name}");
                accepted += 1;
            }
            (Err(error), None, Some(python)) => {
                assert_eq!(error.code, text(get(python, "code").unwrap()), "{name}");
                assert_eq!(
                    error.message,
                    text(get(python, "message").unwrap()),
                    "{name}"
                );
                let hint = get(python, "hint").filter(|v| !matches!(v, Value::Null));
                assert_eq!(error.hint.as_deref(), hint.map(text).as_deref(), "{name}");
                let details = get(python, "details").filter(|v| !matches!(v, Value::Null));
                assert_eq!(error.details.as_ref(), details, "{name}");
                assert!(!error.retryable, "{name}");
            }
            (outcome, _, _) => panic!("{name}: Rust {outcome:?}, Python {expected:?}"),
        }
    }
    // Guard against a corpus that silently stops exercising acceptance.
    assert_eq!((cases.len(), accepted), (106, 34));
}
