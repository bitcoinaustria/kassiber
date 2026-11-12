//! Pure BTCPay normalizers against `crates/chain/fixtures/python_btcpay.json`.
use super::normalize::{self, field, text};
use super::payment_methods as methods;
use super::permissions::{
    authorize_url, capability_permissions, permission_label, server_version, setup_guide,
    wallet_history_preset, KeyGrant,
};
use super::url::from_py;
use kassiber_envelope::{dict, get, s, truthy, AppError};
use kassiber_pyjson::{PyStr, Value};

fn fixture() -> Value {
    kassiber_pytext::loads(include_str!("../../fixtures/python_btcpay.json")).unwrap()
}

fn section(name: &str) -> Vec<Value> {
    match get(&fixture(), name).unwrap() {
        Value::List(items) => items.clone(),
        _ => panic!("{name}"),
    }
}

/// `str(value or "")` of a frozen input.
fn arg(value: &Value) -> String {
    if truthy(value) {
        from_py(&text(value))
    } else {
        String::new()
    }
}

fn outcome(result: Result<Value, AppError>) -> Value {
    match result {
        Ok(value) => dict([("ok", value)]),
        Err(e) if e.code == "internal_error" => dict([("exception", s("internal"))]),
        Err(e) => dict([(
            "error",
            dict([
                ("code", s(e.code.as_str())),
                ("message", s(e.message.to_string())),
                ("hint", e.hint.as_deref().map_or(Value::Null, s)),
                ("details", e.details.clone().unwrap_or(Value::Null)),
            ]),
        )]),
    }
}

/// Python's `exception` outcomes are compared by class only.
fn same(actual: Value, expected: &Value, name: &str) {
    if get(expected, "exception").is_some() {
        assert!(get(&actual, "exception").is_some(), "{name}: {actual:?}");
    } else {
        assert_eq!(&actual, expected, "{name}");
    }
}

fn opt(value: Option<String>) -> Value {
    value.map_or(Value::Null, s)
}

#[test]
fn payment_methods_match_python() {
    for case in section("payment_methods") {
        let input = field(&case, "input");
        let value = arg(input);
        let name = format!("{value:?}");
        same(
            outcome(Ok(s(methods::canonical(&value)))),
            field(&case, "canonical"),
            &name,
        );
        let (currency, handler) = methods::split(&value);
        same(
            outcome(Ok(Value::List(vec![s(currency), s(handler)]))),
            field(&case, "split"),
            &name,
        );
        same(
            outcome(Ok(methods::classify(&value).to_value())),
            field(&case, "classify"),
            &name,
        );
        same(
            outcome(Ok(Value::Bool(methods::is_wallet_history(&value)))),
            field(&case, "wallet_history"),
            &name,
        );
        let shown = match input {
            Value::Null => "None".to_owned(),
            other => from_py(&text(other)),
        };
        same(
            outcome(methods::require_wallet_history(&value, &shown).map(s)),
            field(&case, "require"),
            &name,
        );
        same(
            outcome(Ok(opt(methods::asset(&value)))),
            field(&case, "asset"),
            &name,
        );
    }
}

/// A frozen `(major, minor)` (a JSON list), saturated like `server_version`.
fn frozen_version(value: &Value) -> Option<(i64, i64)> {
    let Value::List(parts) = value else {
        assert_eq!(value, &Value::Null);
        return None;
    };
    let int = |v: &Value| match v {
        Value::Int(n) => i64::try_from(n).unwrap_or(i64::MAX),
        other => panic!("{other:?}"),
    };
    Some((int(&parts[0]), int(&parts[1])))
}

#[test]
fn server_versions_match_python() {
    for case in section("versions") {
        assert_eq!(
            server_version(field(&case, "input")),
            frozen_version(field(&case, "version")),
            "{case:?}"
        );
    }
    for case in section("wallet_versions") {
        let version = frozen_version(field(&case, "version"));
        let tables = Value::List(
            capability_permissions(version)
                .into_iter()
                .map(|table| {
                    Value::List(
                        table
                            .iter()
                            .map(|(c, p)| Value::List(vec![s(*c), s(*p)]))
                            .collect(),
                    )
                })
                .collect(),
        );
        assert_eq!(&tables, field(&case, "tables"), "{case:?}");
        assert_eq!(
            s(wallet_history_preset(version)),
            *field(&case, "history_preset"),
            "{case:?}"
        );
    }
}

#[test]
fn key_setup_guides_match_python() {
    for case in section("guides") {
        let server = match field(&case, "server") {
            Value::Null => None,
            other => Some(arg(other)),
        };
        let preset = arg(field(&case, "preset"));
        let scope = match field(&case, "scope") {
            Value::Null => None,
            other => Some(arg(other)),
        };
        same(
            outcome(setup_guide(server.as_deref(), &preset, scope.as_deref())),
            field(&case, "guide"),
            &format!("{case:?}"),
        );
        let stores = ["s1", " ", "s 2"].map(str::to_owned);
        same(
            outcome(
                authorize_url(
                    server.as_deref().unwrap_or("https://shop.example"),
                    &preset,
                    &stores,
                    scope.as_deref(),
                )
                .map(s),
            ),
            field(&case, "scoped_url"),
            &format!("{case:?}"),
        );
    }
}

#[test]
fn bark_pages_match_python() {
    for case in section("bark_pages") {
        let Value::List(page) = field(&case, "input").clone() else {
            panic!("page")
        };
        same(
            outcome(super::bark::page_ids(&page)),
            field(&case, "stable_ids"),
            &format!("{case:?}"),
        );
        same(
            outcome(super::bark::page_fingerprint(&page).map(s)),
            field(&case, "fingerprint"),
            &format!("{case:?}"),
        );
    }
}

#[test]
fn key_grants_match_python() {
    for case in section("grants") {
        let permissions = match field(&case, "input") {
            Value::List(items) => Some(items.iter().map(|v| from_py(&text(v))).collect::<Vec<_>>()),
            _ => None,
        };
        let grant = KeyGrant::parse(permissions.as_deref());
        assert_eq!(&grant.summary(None), field(&case, "summary"), "{case:?}");
        assert_eq!(
            &grant.capabilities(Some("store-a"), None),
            field(&case, "store_a"),
            "{case:?}"
        );
        let excess = Value::List(grant.excess(false).into_iter().map(s).collect());
        assert_eq!(&excess, field(&case, "excess"), "{case:?}");
        let Value::List(by_version) = field(&case, "by_version") else {
            panic!("by_version")
        };
        for entry in by_version {
            let version = frozen_version(field(entry, "version"));
            assert_eq!(
                &grant.summary(version),
                field(entry, "summary"),
                "{entry:?}"
            );
            assert_eq!(
                &grant.capabilities(Some("store-a"), version),
                field(entry, "store_a"),
                "{entry:?}"
            );
        }
    }
    let Value::Dict(labels) = get(&fixture(), "labels").unwrap().clone() else {
        panic!("labels");
    };
    for (key, expected) in labels {
        let kassiber_pyjson::Key::Str(key) = key else {
            panic!("key")
        };
        assert_eq!(s(permission_label(&from_py(&key))), expected);
    }
}

#[test]
fn invoice_origins_match_python() {
    let titles = |id: &PyStr| match from_py(id).as_str() {
        "req1" => Some(PyStr::from("Title one")),
        "req9" => Some(PyStr::from("Ninth")),
        _ => None,
    };
    for case in section("origins") {
        let actual =
            normalize::normalize_invoice(&PyStr::from("store"), field(&case, "input"), &titles);
        same(
            outcome(actual),
            field(&case, "normalized"),
            &format!("{case:?}"),
        );
    }
}

#[test]
fn payouts_match_python() {
    let pulls = |id: &PyStr| -> Option<Value> {
        let all = kassiber_pytext::loads(r#"{"pp1": {"id": "pp1", "name": "Refund 5ZbLqJLTwm9ZbDtZzkB3Yp", "viewLink": "https://shop/pull-payments/pp1"}, "pp2": {"id": "pp2", "name": "Refund customers", "description": "refund 5ZbLqJLTwm9ZbDtZzkB3Yq"}}"#).unwrap();
        get(&all, &from_py(id)).cloned()
    };
    for case in section("payouts") {
        let actual =
            normalize::normalize_payout(&PyStr::from("store"), field(&case, "input"), &pulls)
                .map(|v| v.unwrap_or(Value::Null));
        same(
            outcome(actual),
            field(&case, "normalized"),
            &format!("{case:?}"),
        );
        let encoded = kassiber_pyjson::dumps(&case, &Default::default()).unwrap();
        let normalized =
            kassiber_pyjson::dumps(field(&case, "normalized"), &Default::default()).unwrap();
        assert!(!normalized.contains("DISCARDED_BTCPAY_SECRET"), "{encoded}");
    }
}

#[test]
fn inspection_helpers_match_python() {
    for case in section("addresses") {
        let (chain, network) = normalize::network_from_address(&arg(field(&case, "input")));
        let actual = Value::List(vec![
            chain.map_or(Value::Null, s),
            network.map_or(Value::Null, s),
        ]);
        assert_eq!(&actual, field(&case, "network"), "{case:?}");
    }
    let inputs: [&[&str]; 3] = [&[], &["b", "a", "a", " "], &["bc1qa", "bc1qb", "bc1qc"]];
    for (input, expected) in inputs.iter().zip(section("fingerprints")) {
        let addresses: Vec<String> = input.iter().map(|a| (*a).to_owned()).collect();
        assert_eq!(opt(normalize::wallet_fingerprint(&addresses)), expected);
    }
    for case in section("payment_method_rows") {
        let normalized: Vec<Value> = normalize::payment_method_rows(field(&case, "input"))
            .iter()
            .map(|row| {
                normalize::normalize_payment_method(&PyStr::from("store"), row)
                    .unwrap_or(Value::Null)
            })
            .collect();
        assert_eq!(
            &Value::List(normalized),
            field(&case, "normalized"),
            "{case:?}"
        );
    }
}

#[test]
fn page_checkpoints_and_payments_match_python() {
    for case in section("wallet_pages") {
        let Value::List(page) = field(&case, "input") else {
            panic!("page")
        };
        same(
            outcome(normalize::wallet_page_ids(page)),
            field(&case, "stable_ids"),
            "wallet ids",
        );
        same(
            outcome(normalize::wallet_page_fingerprint(page).map(s)),
            field(&case, "fingerprint"),
            "wallet fp",
        );
    }
    for case in section("invoice_pages") {
        let Value::List(page) = field(&case, "input") else {
            panic!("page")
        };
        same(
            outcome(normalize::invoice_page_ids(page)),
            field(&case, "stable_ids"),
            "invoice ids",
        );
        same(
            outcome(normalize::invoice_page_fingerprint(page).map(s)),
            field(&case, "fingerprint"),
            "invoice fp",
        );
        same(
            outcome(Ok(Value::Bool(normalize::invoice_page_has_open(page)))),
            field(&case, "open"),
            "open",
        );
    }
    let invoice = dict([("currency", s("EUR")), ("amount", s("10"))]);
    for case in section("payments") {
        let payment = field(&case, "input");
        same(
            outcome(normalize::normalize_payment(&invoice, payment)),
            field(&case, "normalized"),
            &format!("{payment:?}"),
        );
        let methods_list = vec![
            dict([
                ("paymentMethodId", s("btc")),
                ("rate", s("1")),
                ("payments", Value::List(vec![payment.clone(), s("skip")])),
            ]),
            s("skip"),
            dict([("paymentMethod", s("BTC-LN")), ("payments", s("bad"))]),
        ];
        same(
            outcome(Ok(normalize::payments_from_methods(&methods_list))),
            field(&case, "from_methods"),
            "from methods",
        );
    }
}

/// Greenfield error bodies are the server's text.
mod greenfield_errors {
    use super::super::*;

    const KEY: &str = "0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn a_greenfield_error_body_withholds_the_api_key() {
        let config = json!({"name":"shop","url":"https://pay.example","token":KEY,
            "tor_proxy":"socks5h://u:proxy-pw-1@127.0.0.1:9050"});
        let withheld = config::credential_forms_of(&config, &[KEY]);
        let body = json!({
            "code": format!("bad-{KEY}"),
            "message": format!("key {} via proxy-pw-1", KEY.to_uppercase()),
            "missingPermission": format!("btcpay.store.{KEY}"),
        });
        let error = status_error(500, body.to_string().as_bytes(), &withheld);
        assert_eq!(
            error.message,
            "BTCPay returned HTTP 500 for the store list: key <redacted> via <redacted>"
        );
        assert_eq!(
            error.details.as_ref().map(kassiber_envelope::py_str),
            Some("{'btcpay_code': 'bad-<redacted>', 'http_status': 500}".to_owned())
        );
        let error = status_error(403, body.to_string().as_bytes(), &withheld);
        assert_eq!(
            error.message,
            "BTCPay API key is missing the 'btcpay.store.<redacted>' permission for the store list"
        );
    }

    #[test]
    fn a_store_list_403_names_the_permission_by_its_label() {
        let withheld = config::credential_forms_of(&json!({"name": "shop"}), &[]);
        for (missing, label) in [
            ("btcpay.store.canviewstoresettings", "View your stores"),
            ("btcpay.store.canviewwallet:S1", "View wallets"),
            ("custom.permission:S1", "custom.permission"),
        ] {
            let body = json!({"missingPermission": missing});
            let error = status_error(403, body.to_string().as_bytes(), &withheld);
            assert_eq!(
                error.message,
                format!("BTCPay API key is missing the '{label}' permission for the store list")
            );
        }
    }
}
