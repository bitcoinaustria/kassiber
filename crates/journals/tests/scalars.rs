use kassiber_journals::scalar;
use serde_json::Value;

#[test]
fn byte_scalars_keep_python_identity_and_serialization_refusal() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("data/render-scalar-bytes.json")).unwrap();
    for case in cases {
        let hex = case["hex"].as_str().unwrap();
        let native = scalar::Value::Bytes(
            hex.as_bytes()
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
                .collect(),
        );
        assert_eq!(scalar::kind(&native), case["kind"]);
        assert_eq!(scalar::truth(&native), case["truth"].as_bool().unwrap());
        assert_eq!(scalar::repr(&native), case["representation"]);
        assert_eq!(scalar::text(&native).as_str().unwrap(), case["text"]);
        assert!(scalar::to_json(&native).is_none());
        assert_eq!(
            scalar::encode(&native).unwrap_err().to_string(),
            case["encoding_error"]
        );
        assert_eq!(
            scalar::encode(&scalar::snapshot(&native)).unwrap(),
            case["snapshot"]
        );
    }
}

#[test]
fn lossless_scalars_match_python_bytes_and_types() {
    let cases: Vec<Value> = serde_json::from_str(include_str!("data/render-scalars.json")).unwrap();
    for case in cases {
        let native = scalar::decode(case["raw"].as_str().unwrap()).unwrap();
        assert_eq!(
            scalar::encode(&native).unwrap(),
            case["encoded"],
            "{}",
            case["name"]
        );
        assert_eq!(
            scalar::encode(&scalar::snapshot(&native)).unwrap(),
            case["snapshot"],
            "{}",
            case["name"]
        );
        assert_eq!(scalar::kind(&native), case["kind"], "{}", case["name"]);
        assert_eq!(
            scalar::truth(&native),
            case["truth"].as_bool().unwrap(),
            "{}",
            case["name"]
        );
        assert_eq!(
            scalar::repr(&native),
            case["representation"],
            "{}",
            case["name"]
        );
        assert_eq!(
            scalar::text(&native),
            match scalar::decode(case["text_json"].as_str().unwrap()).unwrap() {
                scalar::Value::Str(v) => v,
                _ => unreachable!(),
            },
            "{}",
            case["name"]
        );
        assert_eq!(
            scalar::to_json(&native).is_some(),
            case["serde"].as_bool().unwrap(),
            "{}",
            case["name"]
        );
    }
}

#[test]
fn lossless_scalar_decode_refusals_match_python() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("data/render-scalar-refusals.json")).unwrap();
    for case in cases {
        let refusal = scalar::decode(case["raw"].as_str().unwrap()).unwrap_err();
        assert_eq!(serde_json::to_value(refusal).unwrap(), case["expected"]);
    }
}
