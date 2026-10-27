//! Each frozen Python slice has a selectable mutation witness.
use kassiber_custody::{interpretation, snapshot::Snapshot};
use serde_json::Value;

fn check(case: &str) {
    let vectors: Vec<Value> = serde_json::from_str(include_str!(
        "../../../tests/fixtures/custody_interpretation_cases.json"
    ))
    .unwrap();
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
