//! Python's frozen update-check decisions (scripts/export_vectors.py).
use kassiber_envelope::get;
use kassiber_pyjson::Value;
use kassiber_release::update;

fn vectors() -> Value {
    kassiber_pytext::loads(include_str!("data/vectors.json")).unwrap()
}
fn list<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    match get(value, key) {
        Some(Value::List(items)) => items,
        _ => panic!("{key}"),
    }
}
fn text(value: &Value) -> String {
    kassiber_envelope::py_str(value)
}

#[test]
fn version_parsing_and_ordering_match_python() {
    let data = vectors();
    for case in list(&data, "versions") {
        let Value::List(case) = case else { panic!() };
        assert_eq!(
            Value::Bool(update::parse_version(&text(&case[0])).is_some()),
            case[1],
            "{:?}",
            case[0]
        );
    }
    for case in list(&data, "newer") {
        let Value::List(case) = case else { panic!() };
        let (a, b) = (text(&case[0]), text(&case[1]));
        assert_eq!(Value::Bool(update::is_newer(&a, &b)), case[2], "{a} > {b}");
        assert_eq!(Value::Bool(update::is_newer(&b, &a)), case[3], "{b} > {a}");
    }
}

#[test]
fn release_selection_and_status_text_match_python() {
    let data = vectors();
    for case in list(&data, "selection") {
        let Value::List(case) = case else { panic!() };
        match update::select_release(&case[0]) {
            Ok(chosen) => assert_eq!(chosen, case[1], "{:?}", case[0]),
            Err(error) => {
                assert!(get(&case[1], "error").is_some(), "{:?}: {error:?}", case[0]);
                assert_eq!(error.code, "update_check_failed");
            }
        }
    }
    for case in list(&data, "render") {
        let Value::List(case) = case else { panic!() };
        assert_eq!(update::render(&case[0], false), text(&case[1]));
        assert_eq!(update::render(&case[0], true), text(&case[2]));
    }
}

#[test]
fn release_urls_quote_tags_like_python() {
    let data = vectors();
    for case in list(&data, "release_url") {
        let Value::List(case) = case else { panic!() };
        assert_eq!(update::release_url(&text(&case[0])), text(&case[1]));
    }
}
