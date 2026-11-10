#[test]
fn liquid_confirmation_scope_python_vectors() {
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("data/liquid_confirmations.json")).unwrap();
    assert!(cases.len() > 2000);
    for case in cases {
        let scope = kassiber_custody::confirmation::physical_scope(&case["row"]).unwrap();
        let actual = scope.map(|(chain, network, txid)| vec![chain, network, txid]);
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            case["scope"],
            "{}",
            case["name"]
        );
    }
}
