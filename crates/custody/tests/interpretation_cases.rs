//! Each frozen Python slice has a selectable mutation witness.
use kassiber_custody::{interpretation, snapshot::Snapshot};
use serde_json::Value;

fn vectors() -> &'static [Value] {
    static VECTORS: std::sync::OnceLock<Vec<Value>> = std::sync::OnceLock::new();
    VECTORS.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../../tests/fixtures/custody_interpretation_cases.json"
        ))
        .unwrap()
    })
}

fn check(case: &str) {
    let vectors = vectors();
    let vector = vectors.iter().find(|v| v["case"] == case).unwrap();
    let snapshot: Snapshot = serde_json::from_value(vector["input"].clone()).unwrap();
    snapshot.validate().unwrap();
    assert_eq!(
        interpretation::partial(&snapshot).unwrap(),
        vector["partial"],
        "{case}"
    );
}

#[test]
fn interpretation_shared_aliases() {
    check("shared_aliases");
}
#[test]
fn interpretation_shared_reservations() {
    check("shared_reservations");
}
#[test]
fn interpretation_shared_anchor_alias() {
    check("shared_anchor_alias");
}

#[test]
fn interpretation_samourai_tx0() {
    check("samourai_tx0");
}

#[test]
fn interpretation_samourai_tx0_fee() {
    check("samourai_tx0_fee");
}

#[test]
fn interpretation_samourai_mixing() {
    check("samourai_mixing");
}

#[test]
fn interpretation_samourai_remix() {
    check("samourai_remix");
}

#[test]
fn interpretation_samourai_fanout() {
    check("samourai_fanout");
}

#[test]
fn interpretation_samourai_badbank() {
    check("samourai_badbank");
}

#[test]
fn interpretation_samourai_untrusted_source() {
    check("samourai_untrusted_source");
}

#[test]
fn interpretation_samourai_untrusted_target() {
    check("samourai_untrusted_target");
}

#[test]
fn interpretation_samourai_unknown_fee() {
    check("samourai_unknown_fee");
}

#[test]
fn interpretation_samourai_implicit_delta() {
    check("samourai_implicit_delta");
}

#[test]
fn interpretation_samourai_oversize() {
    check("samourai_oversize");
}

#[test]
fn interpretation_samourai_wrong_asset() {
    check("samourai_wrong_asset");
}

#[test]
fn interpretation_samourai_group_mismatch() {
    check("samourai_group_mismatch");
}

#[test]
fn interpretation_samourai_multiple_sources() {
    check("samourai_multiple_sources");
}

#[test]
fn interpretation_samourai_invalid_role() {
    check("samourai_invalid_role");
}

#[test]
fn interpretation_samourai_empty_group() {
    check("samourai_empty_group");
}

#[test]
fn interpretation_samourai_empty_section() {
    check("samourai_empty_section");
}

#[test]
fn interpretation_samourai_bool_group() {
    check("samourai_bool_group");
}

#[test]
fn interpretation_samourai_dict_group() {
    check("samourai_dict_group");
}

#[test]
fn interpretation_samourai_list_group() {
    check("samourai_list_group");
}

#[test]
fn interpretation_samourai_nonprintable_group() {
    check("samourai_nonprintable_group");
}

#[test]
fn interpretation_samourai_numeric_group() {
    check("samourai_numeric_group");
}

#[test]
fn interpretation_samourai_false_presence_gate() {
    check("samourai_false_presence_gate");
}

#[test]
fn interpretation_samourai_decoded_config() {
    check("samourai_decoded_config");
}

#[test]
fn interpretation_samourai_mixing_residual() {
    check("samourai_mixing_residual");
}

#[test]
fn interpretation_samourai_fee_inclusive() {
    check("samourai_fee_inclusive");
}

#[test]
fn interpretation_samourai_tampered_hash() {
    check("samourai_tampered_hash");
}

#[test]
fn interpretation_samourai_sorted_targets() {
    check("samourai_sorted_targets");
}

#[test]
fn interpretation_samourai_duplicate_target() {
    check("samourai_duplicate_target");
}

#[test]
fn interpretation_samourai_reviewed_reservation() {
    check("samourai_reviewed_reservation");
}

#[test]
fn interpretation_samourai_multiple_groups() {
    check("samourai_multiple_groups");
}

#[test]
fn interpretation_samourai_post_unicode14_group() {
    check("samourai_post_unicode14_group");
}

#[test]
fn interpretation_samourai_implicit_mix_residual() {
    check("samourai_implicit_mix_residual");
}

#[test]
fn interpretation_samourai_exact_msat() {
    check("samourai_exact_msat");
}

#[test]
fn interpretation_samourai_float_overflow() {
    check("samourai_float_overflow");
}

macro_rules! recorded_cases {
    ($($name:ident => $case:literal),* $(,)?) => { $(#[test] fn $name() { check(concat!("recorded_", $case)); })* };
}
recorded_cases! {
    interpretation_recorded_bitcoin => "bitcoin",
    interpretation_recorded_bitcoin_upper_txid => "bitcoin_upper_txid",
    interpretation_recorded_bitcoin_typed_external => "bitcoin_typed_external",
    interpretation_recorded_bitcoin_timestamp_skew => "bitcoin_timestamp_skew",
    interpretation_recorded_bitcoin_fee_inclusive => "bitcoin_fee_inclusive",
    interpretation_recorded_bitcoin_implicit_residual => "bitcoin_implicit_residual",
    interpretation_recorded_bitcoin_partial => "bitcoin_partial",
    interpretation_recorded_exact_msat => "exact_msat",
    interpretation_recorded_liquid_main => "liquid_main",
    interpretation_recorded_liquid_test => "liquid_test",
    interpretation_recorded_liquid_regtest => "liquid_regtest",
    interpretation_recorded_liquid_asset_id => "liquid_asset_id",
    interpretation_recorded_lightning_lnd => "lightning_lnd",
    interpretation_recorded_lightning_cln => "lightning_cln",
    interpretation_recorded_lightning_circular => "lightning_circular",
    interpretation_recorded_lightning_hash_case => "lightning_hash_case",
    interpretation_recorded_untrusted_source => "untrusted_source",
    interpretation_recorded_untrusted_target => "untrusted_target",
    interpretation_recorded_tampered_quantity => "tampered_quantity",
    interpretation_recorded_lightning_chronology => "lightning_chronology",
    interpretation_recorded_untrusted_both => "untrusted_both",
    interpretation_recorded_same_wallet => "same_wallet",
    interpretation_recorded_zero_inbound => "zero_inbound",
    interpretation_recorded_negative_outbound => "negative_outbound",
    interpretation_recorded_multiple_sources => "multiple_sources",
    interpretation_recorded_multiple_targets => "multiple_targets",
    interpretation_recorded_different_txid => "different_txid",
    interpretation_recorded_network_separation => "network_separation",
    interpretation_recorded_lightning_shortfall => "lightning_shortfall",
    interpretation_recorded_lightning_forged_source => "lightning_forged_source",
    interpretation_recorded_lightning_forged_lnd => "lightning_forged_lnd",
    interpretation_recorded_lightning_bad_hash => "lightning_bad_hash",
    interpretation_recorded_lightning_wrong_kind => "lightning_wrong_kind",
    interpretation_recorded_lightning_chain_hash => "lightning_chain_hash",
    interpretation_recorded_lightning_duplicate => "lightning_duplicate",
    interpretation_recorded_lightning_no_network => "lightning_no_network",
    interpretation_recorded_lightning_asset_case => "lightning_asset_case",
    interpretation_recorded_liquid_missing_identity => "liquid_missing_identity",
    interpretation_recorded_liquid_asset_separation => "liquid_asset_separation",
    interpretation_recorded_synthetic_id => "synthetic_id",
    interpretation_recorded_reviewed_reservation => "reviewed_reservation",
    interpretation_recorded_multiple_events => "multiple_events",
    interpretation_recorded_nonpositive_placeholder => "nonpositive_placeholder",
    interpretation_recorded_nonfinite_graph_identity => "nonfinite_graph_identity",
    interpretation_recorded_lightning_skew_boundary => "lightning_skew_boundary",
    interpretation_recorded_lightning_invalid_timestamp => "lightning_invalid_timestamp",
}

macro_rules! fanout_cases {
    ($($name:ident => $case:literal),* $(,)?) => { $(#[test] fn $name() { check(concat!("fanout_", $case)); })* };
}
fanout_cases! {
    interpretation_fanout_bitcoin => "bitcoin",
    interpretation_fanout_bitcoin_graph => "bitcoin_graph",
    interpretation_fanout_liquid_graph => "liquid_graph",
    interpretation_fanout_liquid_main => "liquid_main",
    interpretation_fanout_liquid_test => "liquid_test",
    interpretation_fanout_liquid_regtest => "liquid_regtest",
    interpretation_fanout_liquid_asset_id => "liquid_asset_id",
    interpretation_fanout_three_destinations => "three_destinations",
    interpretation_fanout_equal_amount_ties => "equal_amount_ties",
    interpretation_fanout_amount_order => "amount_order",
    interpretation_fanout_provider_ids => "provider_ids",
    interpretation_fanout_multiple_events => "multiple_events",
    interpretation_fanout_exact_msat => "exact_msat",
    interpretation_fanout_value_pricing => "value_pricing",
    interpretation_fanout_exact_value_pricing => "exact_value_pricing",
    interpretation_fanout_existing_rate => "existing_rate",
    interpretation_fanout_existing_exact_rate => "existing_exact_rate",
    interpretation_fanout_zero_value => "zero_value",
    interpretation_fanout_negative_value => "negative_value",
    interpretation_fanout_missing_value => "missing_value",
    interpretation_fanout_zero_exact_value_fallback => "zero_exact_value_fallback",
    interpretation_fanout_zero_rate => "zero_rate",
    interpretation_fanout_whitespace_value => "whitespace_value",
    interpretation_fanout_unicode_value => "unicode_value",
    interpretation_fanout_nan_value => "nan_value",
    interpretation_fanout_nan_payload => "nan_payload",
    interpretation_fanout_infinite_value => "infinite_value",
    interpretation_fanout_negative_infinite_value => "negative_infinite_value",
    interpretation_fanout_untrusted_source => "untrusted_source",
    interpretation_fanout_untrusted_target => "untrusted_target",
    interpretation_fanout_tampered_quantity => "tampered_quantity",
    interpretation_fanout_fee_inclusive => "fee_inclusive",
    interpretation_fanout_partial_total => "partial_total",
    interpretation_fanout_over_total => "over_total",
    interpretation_fanout_duplicate_wallet => "duplicate_wallet",
    interpretation_fanout_same_wallet => "same_wallet",
    interpretation_fanout_one_destination => "one_destination",
    interpretation_fanout_multiple_sources => "multiple_sources",
    interpretation_fanout_zero_source => "zero_source",
    interpretation_fanout_zero_destination => "zero_destination",
    interpretation_fanout_negative_destination => "negative_destination",
    interpretation_fanout_unknown_direction => "unknown_direction",
    interpretation_fanout_asset_separation => "asset_separation",
    interpretation_fanout_network_separation => "network_separation",
    interpretation_fanout_synthetic_source => "synthetic_source",
    interpretation_fanout_synthetic_destination => "synthetic_destination",
    interpretation_fanout_reviewed_reservation => "reviewed_reservation",
}

#[test]
fn interpretation_ownership_projection() {
    let vectors = vectors();
    for vector in vectors
        .iter()
        .filter(|v| v["case"].as_str().unwrap().starts_with("ownership_"))
    {
        check(vector["case"].as_str().unwrap());
    }
}

#[test]
fn interpretation_consolidation_projection() {
    let vectors = vectors();
    let cases: Vec<_> = vectors
        .iter()
        .filter(|v| v["case"].as_str().unwrap().starts_with("consolidation_"))
        .collect();
    assert_eq!(cases.len(), 31);
    for vector in cases {
        check(vector["case"].as_str().unwrap());
    }
}

#[test]
fn interpretation_pipeline_projection() {
    let vectors = vectors();
    assert_eq!(vectors.len(), 343);
    for vector in vectors {
        let snapshot: Snapshot = serde_json::from_value(vector["input"].clone()).unwrap();
        assert_eq!(
            interpretation::partial(&snapshot).unwrap(),
            vector["partial"],
            "{}",
            vector["case"]
        );
    }
}

#[test]
fn interpretation_safety_projection() {
    let vectors = vectors();
    for vector in vectors {
        let snapshot: Snapshot = serde_json::from_value(vector["input"].clone()).unwrap();
        assert_eq!(
            interpretation::partial(&snapshot).unwrap()["safety"],
            vector["partial"]["safety"],
            "{}",
            vector["case"]
        );
    }
}

#[test]
fn interpretation_channel_projection() {
    let vectors = vectors();
    let cases: Vec<_> = vectors
        .iter()
        .filter(|v| v["case"].as_str().unwrap().starts_with("channel_"))
        .collect();
    assert_eq!(cases.len(), 34);
    for vector in cases {
        check(vector["case"].as_str().unwrap());
    }
}

#[test]
fn interpretation_native_projection() {
    let vectors = vectors();
    let cases: Vec<_> = vectors
        .iter()
        .filter(|v| {
            v["case"].as_str().unwrap().starts_with("native_")
                || matches!(
                    v["case"].as_str().unwrap(),
                    "shared_reservations"
                        | "samourai_fanout"
                        | "recorded_lightning_lnd"
                        | "channel_role_reservation"
                )
        })
        .collect();
    assert_eq!(cases.len(), 59);
    for v in cases {
        check(v["case"].as_str().unwrap());
    }
}
