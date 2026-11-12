//! Frozen CPython stage outputs: every case is independently selectable.
use kassiber_custody::custody;
use serde_json::Value;
fn check(case: &str) {
    let vectors: Vec<Value> = serde_json::from_str(include_str!(
        "../../../tests/fixtures/custody_component_cases.json"
    ))
    .unwrap();
    let v = vectors
        .iter()
        .find(|v| v["case"] == case)
        .expect("named component case");
    let result: Value = serde_json::from_str(&custody(&v["input"].to_string())).unwrap();
    if let Some(code) = v.get("error_code") {
        assert_eq!(&result["error"]["code"], code, "{case}");
    } else {
        // Including main #679's activation `remediation` (S11).
        assert_eq!(
            result["stages"]["components"],
            v["components"],
            "{case}: {}",
            result.get("error").unwrap_or(&Value::Null)
        );
    }
}
#[test]
fn components_balanced() {
    check("balanced");
}
#[test]
fn components_no_legs() {
    check("no_legs");
}
#[test]
fn components_missing_source() {
    check("missing_source");
}
#[test]
fn components_missing_destination() {
    check("missing_destination");
}
#[test]
fn components_unknown_role() {
    check("unknown_role");
}
#[test]
fn components_unbalanced() {
    check("unbalanced");
}
#[test]
fn components_unit_mismatch() {
    check("unit_mismatch");
}
#[test]
fn components_missing_time() {
    check("missing_time");
}
#[test]
fn components_invalid_time() {
    check("invalid_time");
}
#[test]
fn components_chronology() {
    check("chronology");
}
#[test]
fn components_chronology_after_boundary() {
    check("chronology_after_boundary");
}
#[test]
fn components_location_continuity() {
    check("location_continuity");
}
#[test]
fn components_allocation_required() {
    check("allocation_required");
}
#[test]
fn components_multi_allocation() {
    check("multi_allocation");
}
#[test]
fn components_allocation_invalid() {
    check("allocation_invalid");
}
#[test]
fn components_allocation_quantity() {
    check("allocation_quantity");
}
#[test]
fn components_allocation_source_coverage() {
    check("allocation_source_coverage");
}
#[test]
fn components_allocation_sink_coverage() {
    check("allocation_sink_coverage");
}
#[test]
fn components_fee_asset() {
    check("fee_asset");
}
#[test]
fn components_fee_wallet() {
    check("fee_wallet");
}
#[test]
fn components_fee_scope() {
    check("fee_scope");
}
#[test]
fn components_fee_orphan() {
    check("fee_orphan");
}
#[test]
fn components_scope_invalid_chain() {
    check("scope_invalid_chain");
}
#[test]
fn components_scope_rail_chain() {
    check("scope_rail_chain");
}
#[test]
fn components_scope_invalid_network() {
    check("scope_invalid_network");
}
#[test]
fn components_scope_mismatch() {
    check("scope_mismatch");
}
#[test]
fn components_scope_laundering() {
    check("scope_laundering");
    // One connected domain must remain valid; otherwise an over-eager scope
    // blocker would appear to catch laundering while rejecting valid routes.
    check("balanced");
}
#[test]
fn components_cross_scope_laundering() {
    check("cross_scope_laundering");
}
#[test]
fn components_cross_location_continuity() {
    check("cross_location_continuity");
}
#[test]
fn components_conversion() {
    check("conversion");
}
#[test]
fn components_conversion_topology() {
    check("conversion_topology");
}
#[test]
fn components_conversion_retained() {
    check("conversion_retained");
}
#[test]
fn components_conversion_review() {
    check("conversion_review");
}
#[test]
fn components_conversion_policy() {
    check("conversion_policy");
}
#[test]
fn components_conversion_missing_value() {
    check("conversion_missing_value");
}
#[test]
fn components_conversion_no_values() {
    check("conversion_no_values");
}
#[test]
fn components_conversion_unbalanced() {
    check("conversion_unbalanced");
}
#[test]
fn components_conversion_fee_quantity() {
    check("conversion_fee_quantity");
}
#[test]
fn components_conversion_fee_value() {
    check("conversion_fee_value");
}
#[test]
fn components_value_only_loss() {
    check("value_only_loss");
}
#[test]
fn components_suspense() {
    check("suspense");
}
#[test]
fn components_suspense_mode() {
    check("suspense_mode");
}
#[test]
fn components_suspense_review() {
    check("suspense_review");
}
#[test]
fn components_suspense_no_allocations() {
    check("suspense_no_allocations");
}
#[test]
fn components_suspense_anchor() {
    check("suspense_anchor");
}
#[test]
fn components_suspense_missing_time() {
    check("suspense_missing_time");
}
#[test]
fn components_suspense_source() {
    check("suspense_source");
}
#[test]
fn components_suspense_asset() {
    check("suspense_asset");
}
#[test]
fn components_suspense_quantity() {
    check("suspense_quantity");
}
#[test]
fn components_suspense_network() {
    check("suspense_network");
}
#[test]
fn components_suspense_time() {
    check("suspense_time");
}
#[test]
fn components_suspense_fee() {
    check("suspense_fee");
}
#[test]
fn components_content_missing() {
    check("content_missing");
}
#[test]
fn components_leg_count() {
    check("leg_count");
}
#[test]
fn components_allocation_count() {
    check("allocation_count");
}
#[test]
fn components_term_count() {
    check("term_count");
}
#[test]
fn components_commitment_header() {
    check("commitment_header");
}
#[test]
fn components_commitment_count() {
    check("commitment_count");
}
#[test]
fn components_commitment_ordinals() {
    check("commitment_ordinals");
}
#[test]
fn components_commitment_zero() {
    check("commitment_zero");
}
#[test]
fn components_commitment_no_anchors() {
    check("commitment_no_anchors");
}
#[test]
fn components_commitment_hash() {
    check("commitment_hash");
}
#[test]
fn components_commitment_volatile() {
    check("commitment_volatile");
}
#[test]
fn components_commitment_rejected() {
    check("commitment_rejected");
}
#[test]
fn components_anchor_missing() {
    check("anchor_missing");
}
#[test]
fn components_anchor_retracted() {
    check("anchor_retracted");
}
#[test]
fn components_anchor_identity() {
    check("anchor_identity");
}
#[test]
fn components_anchor_excluded() {
    check("anchor_excluded");
}
#[test]
fn components_anchor_source_direction() {
    check("anchor_source_direction");
}
#[test]
fn components_anchor_destination_direction() {
    check("anchor_destination_direction");
}
#[test]
fn components_anchor_loss_direction() {
    check("anchor_loss_direction");
}
#[test]
fn components_anchor_asset() {
    check("anchor_asset");
}
#[test]
fn components_asset_upper_semantics() {
    check("asset_upper_semantics");
}
#[test]
fn components_anchor_wallet() {
    check("anchor_wallet");
}
#[test]
fn components_anchor_time_invalid() {
    check("anchor_time_invalid");
}
#[test]
fn components_anchor_time_mismatch() {
    check("anchor_time_mismatch");
}
#[test]
fn components_anchor_time_warning() {
    check("anchor_time_warning");
}
#[test]
fn components_anchor_coverage() {
    check("anchor_coverage");
}
#[test]
fn components_anchor_partial() {
    check("anchor_partial");
}
#[test]
fn components_anchor_partial_zero() {
    check("anchor_partial_zero");
}
#[test]
fn components_anchor_partial_excess() {
    check("anchor_partial_excess");
}
#[test]
fn components_anchor_rail() {
    check("anchor_rail");
}
#[test]
fn components_anchor_chain() {
    check("anchor_chain");
}
#[test]
fn components_anchor_network() {
    check("anchor_network");
}
#[test]
fn components_owned_wallet_missing() {
    check("owned_wallet_missing");
}
#[test]
fn components_retained_location() {
    check("retained_location");
}
#[test]
fn components_transaction_anchor_missing() {
    check("transaction_anchor_missing");
}
#[test]
fn components_lineage_conflict() {
    check("lineage_conflict");
}
#[test]
fn components_revision_missing() {
    check("revision_missing");
}
#[test]
fn components_revision_scope() {
    check("revision_scope");
}
#[test]
fn components_revision_lineage() {
    check("revision_lineage");
}
#[test]
fn components_revision_order() {
    check("revision_order");
}
#[test]
fn components_revision_forward() {
    check("revision_forward");
}
#[test]
fn components_revision_valid() {
    check("revision_valid");
}
#[test]
fn components_membership_conflict() {
    check("membership_conflict");
}
#[test]
fn components_membership_empty_anchor() {
    check("membership_empty_anchor");
}
#[test]
fn components_component_order() {
    check("component_order");
}
#[test]
fn components_children_order() {
    check("children_order");
}
#[test]
fn components_native_supported() {
    check("native_supported");
}
#[test]
fn components_native_partial() {
    check("native_partial");
}
#[test]
fn components_native_contradicted() {
    check("native_contradicted");
}
#[test]
fn components_native_exact_sink() {
    check("native_exact_sink");
}
#[test]
fn components_native_other_asset() {
    check("native_other_asset");
}
#[test]
fn components_hash_kind() {
    check("hash_kind");
}
#[test]
fn components_hash_payment() {
    check("hash_payment");
}
#[test]
fn components_hash_payment_source() {
    check("hash_payment_source");
}
#[test]
fn components_hash_refund_txid() {
    check("hash_refund_txid");
}
#[test]
fn components_hash_refund_vout() {
    check("hash_refund_vout");
}
#[test]
fn components_hash_wallet() {
    check("hash_wallet");
}
#[test]
fn components_hash_direction() {
    check("hash_direction");
}
#[test]
fn components_hash_asset() {
    check("hash_asset");
}
#[test]
fn components_hash_amount() {
    check("hash_amount");
}
#[test]
fn components_hash_fee() {
    check("hash_fee");
}
#[test]
fn components_hash_included() {
    check("hash_included");
}
#[test]
fn components_hash_event() {
    check("hash_event");
}
#[test]
fn components_integer_semantics() {
    check("integer_semantics");
}
#[test]
fn components_time_semantics() {
    check("time_semantics");
}
#[test]
fn components_refusal_mode() {
    check("refusal_mode");
}
#[test]
fn components_refusal_mode_token() {
    check("refusal_mode_token");
}
#[test]
fn components_refusal_mode_missing() {
    check("refusal_mode_missing");
}
#[test]
fn components_refusal_integer() {
    check("refusal_integer");
}
#[test]
fn components_refusal_null_integer() {
    check("refusal_null_integer");
}
#[test]
fn components_refusal_list_integer() {
    check("refusal_list_integer");
}
#[test]
fn components_refusal_dict_integer() {
    check("refusal_dict_integer");
}
#[test]
fn components_refusal_utc_underflow() {
    check("refusal_utc_underflow");
}
#[test]
fn components_refusal_utc_overflow() {
    check("refusal_utc_overflow");
}
#[test]
fn components_refusal_chronology_overflow() {
    check("refusal_chronology_overflow");
}
#[test]
fn components_refusal_missing_child_field() {
    check("refusal_missing_child_field");
}
#[test]
fn components_refusal_child_before_mode() {
    check("refusal_child_before_mode");
}
#[test]
fn components_refusal_time_before_count() {
    check("refusal_time_before_count");
}
#[test]
fn components_scope_enriched_mismatch() {
    check("scope_enriched_mismatch");
}
#[test]
fn components_lineage_ties() {
    check("lineage_ties");
}
#[test]
fn components_location_ties() {
    check("location_ties");
}
#[test]
fn components_transactionless_wallet_missing() {
    check("transactionless_wallet_missing");
}
#[test]
fn components_anchor_row_time_invalid() {
    check("anchor_row_time_invalid");
}
#[test]
fn components_revision_backward() {
    check("revision_backward");
}

#[test]
fn components_asset_hash_upper() {
    check("asset_hash_upper_019b");
    check("asset_hash_upper_0264");
}

#[test]
fn components_refusal_revision_target() {
    check("refusal_revision_target");
}

#[test]
fn components_refusal_competing_revision() {
    check("refusal_competing_revision");
}

#[test]
fn components_refusal_excluded_amount() {
    check("refusal_excluded_amount");
}

#[test]
fn components_refusal_excluded_fee() {
    check("refusal_excluded_fee");
}

#[test]
fn components_refusal_global_leg_order() {
    check("refusal_global_leg_order");
}

#[test]
fn components_refusal_legs_before_allocations() {
    check("refusal_legs_before_allocations");
}

#[test]
fn components_refusal_evidence_count_before_ordinal() {
    check("refusal_evidence_count_before_ordinal");
}

#[test]
fn components_commitment_missing_ignores_ordinal() {
    check("commitment_missing_ignores_ordinal");
}

#[test]
fn components_peg_in() {
    check("peg_in");
}

#[test]
fn components_peg_out() {
    check("peg_out");
}

#[test]
fn components_lightning_receive_lnd() {
    check("lightning_receive_lnd");
}

#[test]
fn components_lightning_receive_nwc() {
    check("lightning_receive_nwc");
}

#[test]
fn components_lightning_receive_cln() {
    check("lightning_receive_cln");
}

#[test]
fn components_lightning_receive_core_ln() {
    check("lightning_receive_core_ln");
}

#[test]
fn components_swap_terms() {
    check("swap_terms");
}

#[test]
fn components_untracked_route() {
    check("untracked_route");
}

#[test]
fn components_liquid_wallet_omitted_scope() {
    check("liquid_wallet_omitted_scope");
}

#[test]
fn components_wallet_scope_conflict() {
    check("wallet_scope_conflict");
}

#[test]
fn components_conversion_inferred() {
    check("conversion_inferred");
}

#[test]
fn components_conversion_inferred_chronology() {
    check("conversion_inferred_chronology");
}

#[test]
fn components_conversion_ambiguous() {
    check("conversion_ambiguous");
}

#[test]
fn components_native_namespace_gate() {
    check("native_namespace_gate");
}

#[test]
fn components_native_excluded_rows() {
    check("native_excluded_rows");
}

#[test]
fn components_lineage_other_profile() {
    check("lineage_other_profile");
}

#[test]
fn components_authored_source_default() {
    check("authored_source_default");
}

#[test]
fn components_retained_reviewed_notes() {
    check("retained_reviewed_notes");
}

#[test]
fn components_liquid_substring_kind() {
    check("liquid_substring_kind");
}

#[test]
fn components_generic_elements_domain() {
    check("generic_elements_domain");
}

#[test]
fn components_fee_null_domain() {
    check("fee_null_domain");
}

#[test]
fn components_missing_time_empty() {
    check("missing_time_empty");
}

#[test]
fn components_transaction_id_empty() {
    check("transaction_id_empty");
}

#[test]
fn components_zero_amount_chronology() {
    check("zero_amount_chronology");
}

#[test]
fn components_authoritative_chronology() {
    check("authoritative_chronology");
}

#[test]
fn components_inactive_header() {
    check("inactive_header");
}

#[test]
fn components_asset_l_btc_alias() {
    check("asset_l_btc_alias");
}

#[test]
fn components_lightning_chain_default() {
    check("lightning_chain_default");
}

#[test]
fn components_lightning_kind_scope() {
    check("lightning_kind_scope");
}

#[test]
fn components_refusal_rate_exact() {
    check("refusal_rate_exact");
}

#[test]
fn components_refusal_rate_value() {
    check("refusal_rate_value");
}

#[test]
fn components_refusal_rate_legacy() {
    check("refusal_rate_legacy");
}

#[test]
fn components_rate_precedence_ignores_later_junk() {
    check("rate_precedence_ignores_later_junk");
}

#[test]
fn components_core_ln_kind_scope() {
    check("core_ln_kind_scope");
}

#[test]
fn components_liquid_kind_conflict() {
    check("liquid_kind_conflict");
}

#[test]
fn components_allocation_beyond_i128() {
    let cases: Vec<Value> = serde_json::from_str(include_str!(
        "../../../tests/fixtures/custody_component_review.json"
    ))
    .unwrap();
    for c in cases {
        let snapshot = serde_json::from_value(c["input"].clone()).unwrap();
        let actual = kassiber_custody::components::compute(&snapshot).unwrap();
        assert_eq!(actual, c["expected"]);
    }
}
#[test]
fn components_legacy_pair_fee_hold() {
    check("legacy_pair_fee_hold");
}
#[test]
fn components_legacy_pair_fee_other_note() {
    check("legacy_pair_fee_other_note");
}
#[test]
fn components_legacy_pair_fee_no_observed_fee() {
    check("legacy_pair_fee_no_observed_fee");
}
#[test]
fn components_legacy_pair_fee_partial_overallocated() {
    check("legacy_pair_fee_partial_overallocated");
}
