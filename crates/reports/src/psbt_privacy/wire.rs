use crate::{dict, value, Value};
use kassiber_envelope::set;

fn ordered(source: &Value, keys: &[&str]) -> Value {
    dict(
        keys.iter().filter_map(|key| {
            kassiber_envelope::get(source, key).map(|value| (*key, value.clone()))
        }),
    )
}

fn rows(source: &Value, keys: &[&str], evidence: &[&str]) -> Value {
    let Value::List(source) = source else {
        unreachable!("native PSBT projection rows")
    };
    Value::List(
        source
            .iter()
            .map(|source| {
                let mut row = ordered(source, keys);
                if kassiber_envelope::get(source, "evidence").is_some() {
                    set(
                        &mut row,
                        "evidence",
                        ordered(value(source, "evidence"), evidence),
                    );
                }
                row
            })
            .collect(),
    )
}

pub(super) fn project(source: &Value) -> Value {
    let mut payload = ordered(
        source,
        &[
            "payload_schema_version",
            "redaction",
            "local_only",
            "read_only",
            "advisory_only",
            "signing_supported",
            "broadcast_supported",
            "psbt_exposure",
            "summary",
            "cluster_merge",
            "change_evidence",
            "unknowns",
            "transaction_tells",
            "adversary_deltas",
            "what_if",
            "findings",
            "limitations",
            "scope",
        ],
    );
    for (section, keys) in [
        (
            "summary",
            &[
                "evidence_level",
                "decode_status",
                "input_count",
                "output_count",
                "known_owned_input_count",
                "unknown_input_count",
                "owned_input_cluster_count",
                "cluster_merge_delta",
                "receive_reuse_output_count",
                "receive_reuse_cluster_delta",
                "blast_radius_score",
                "transaction_tell_count",
                "adversary_delta_count",
                "finding_count",
                "limitation_count",
                "signature_material_present",
            ][..],
        ),
        (
            "cluster_merge",
            &[
                "kind",
                "evidence_level",
                "owned_input_count",
                "owned_input_node_count",
                "owned_input_cluster_count",
                "owned_input_wallet_count",
                "cluster_merge_delta",
                "affected_cluster_ids",
                "affected_node_ids",
                "unknown_input_count",
            ][..],
        ),
        (
            "change_evidence",
            &[
                "evidence_level",
                "owned_output_match_count",
                "receive_reuse_output_count",
                "change_like_output_count",
                "unknown_output_count",
                "receive_reuse_cluster_delta",
                "items",
            ][..],
        ),
        (
            "unknowns",
            &[
                "evidence_level",
                "input_count",
                "output_without_local_match_count",
                "coverage_complete",
            ][..],
        ),
        (
            "scope",
            &["workspace", "profile", "workspace_bound", "profile_bound"][..],
        ),
    ] {
        set(&mut payload, section, ordered(value(source, section), keys));
    }
    for (section, keys, evidence) in [
        (
            "transaction_tells",
            &[
                "tell_id",
                "kind",
                "attribution",
                "evidence_level",
                "source",
                "penalizes_wallet",
                "evidence",
            ][..],
            &[
                "input_count",
                "known_owned_input_count",
                "rbf_signaled",
                "op_return_present",
                "fee_known",
                "fee_value_redacted",
                "unknown_input_count",
            ][..],
        ),
        (
            "adversary_deltas",
            &[
                "tier",
                "label",
                "evidence_level",
                "source",
                "cluster_merge_delta",
                "owned_input_cluster_count",
                "newly_exposed_component_count",
                "evidence",
            ][..],
            &[
                "current_exposed_cluster_count",
                "unknown_input_count",
                "model_assumption_count",
            ][..],
        ),
        (
            "what_if",
            &[
                "scenario",
                "evidence_level",
                "support_status",
                "cluster_merge_delta",
                "evidence",
            ][..],
            &[
                "modeled_output_reuse",
                "assumption",
                "matched_existing_receive_output_count",
                "receive_reuse_cluster_delta",
                "owned_input_cluster_count",
                "cluster_merge_delta",
            ][..],
        ),
        (
            "findings",
            &[
                "id",
                "kind",
                "severity",
                "title",
                "detail",
                "evidence_level",
                "evidence",
            ][..],
            &[
                "cluster_merge_delta",
                "unknown_input_count",
                "receive_reuse_output_count",
                "receive_reuse_cluster_delta",
                "signature_material_present",
                "unsigned_tx_clean",
            ][..],
        ),
        (
            "limitations",
            &["code", "message", "evidence_level", "evidence"][..],
            &["input_count"][..],
        ),
    ] {
        set(
            &mut payload,
            section,
            rows(value(source, section), keys, evidence),
        );
    }
    let mut change = value(&payload, "change_evidence").clone();
    let items = rows(
        value(&change, "items"),
        &[
            "kind",
            "evidence_level",
            "matched_local_output_count",
            "change_evidence",
        ],
        &[],
    );
    set(&mut change, "items", items);
    set(&mut payload, "change_evidence", change);
    payload
}
