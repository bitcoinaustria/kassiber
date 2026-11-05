//! `project_ai_result` (`kassiber/core/chain_analysis_ai.py:151`): a local
//! chain investigation as a remote provider may see it. Topology and exact
//! amounts stay; public chain identifiers become process- and book-scoped
//! `ca-ref:` handles; free text, descriptors and anything not on the
//! allowlist are dropped.

use std::sync::OnceLock;

use fancy_regex::Regex;
use kassiber_envelope::{get, s, set};
use kassiber_pyjson::{Key, PyStr, Value};

use crate::pyre;
use crate::ToolError;

const HANDLE_FIELDS: &[&str] = &[
    "snapshot_id",
    "expected_snapshot_id",
    "base_snapshot_id",
    "current_snapshot_id",
    "case_id",
    "base_id",
    "other_id",
    "next_cursor",
    "cursor",
    "result_digest",
    "status_commitment",
    "plan_id",
    "expected_plan_id",
    "watch_id",
    "job_id",
    "dataset_id",
    "source_token",
    "psbt_token",
    "expected_active_id",
    "content_sha256",
    "dataset_state_digest",
    "state_digest",
];

const REFERENCE_FIELDS: &[&str] = &[
    "node_id",
    "edge_id",
    "input_id",
    "output_id",
    "transaction_node_id",
    "wallet_id",
    "transaction_id",
    "relation_id",
    "component_id",
    "reference",
    "subject",
    "target",
    "subject_id",
    "original_subject",
];

const REFERENCE_LISTS: &[&str] = &[
    "node_ids",
    "edge_ids",
    "input_ids",
    "output_ids",
    "wallet_ids",
    "transaction_ids",
    "component_ids",
    "subjects",
    "inputs_added",
    "inputs_removed",
];

const CODE_FIELDS: &[&str] = &[
    "kind",
    "code",
    "rule",
    "rule_version",
    "model",
    "status",
    "reason",
    "evidence_level",
    "level",
    "observer",
    "direction",
    "direction_from_label",
    "chain",
    "network",
    "scope",
    "authority",
    "basis_state",
    "confidence",
    "category",
    "comparison",
    "mode",
    "egress",
    "filter",
    "depth_unit",
    "ai_reference_scope",
    "source_kind",
    "observer_knowledge",
    "amount_visibility",
    "public_liquid_value_policy",
    "severity",
    "protocol",
    "phase",
    "algorithm",
    "availability",
    "field",
    "unit",
    "receipt_scope",
    "error_code",
    "visibility",
    "dataset_status",
    "format",
    "adapter",
    "network_source",
    "utxo_evidence",
    "value",
    "by_index",
];

const CODE_LISTS: &[&str] = &[
    "rules",
    "premises",
    "assumptions",
    "limitations",
    "path_kinds",
    "stopped_reasons",
    "missing_tables",
    "feature_codes",
    "contradictions",
    "findings_added",
    "findings_removed",
];

const SOURCE_CODES: &[&str] = &[
    "stored_transaction",
    "wallet_inventory",
    "reference_cache",
    "local_label",
    "journal_custody_decisions",
    "journal_custody_economic_relations",
    "reviewed_or_imported_privacy_boundary",
    "invalid_privacy_metadata",
    "equal_output_coinjoin",
    "large_equal_output_coinjoin",
    "local_acquisition",
    "observer_exclusion",
    "psbt",
    "psbt_proposal",
    "psbt_supplied",
    "local_dataset",
];

/// `_FIELDS`, before the sets above are added: unknown keys are dropped
/// until their disclosure has been considered.
const FIELDS: &str = "
    definition baseline threshold enabled checked_at acknowledged_at event_count unread_count observation evaluated acknowledged sequence
    schema_version query summary nodes edges findings clusters patterns exposure
    entropy paths frontier coverage capabilities result items id created_at
    updated_at occurred_at confirmed_at observed_at confirmation_observed_at confirmations confirmed block_height deleted revision expected_revision claim
    hidden_private_node_count hidden_private_relation_count private_labels_withheld
    evidence boundary_evidence conditional_on_boundary_interpretation
    amount_msat target_amount_msat fee_msat min_amount_msat asset target_asset
    source_asset interpretation_count interpretation_count_lower_bound entropy_bits
    link_counts deterministic_links states_explored participant_group_counts
    examples examples_truncated conditional_on_model limits max_states
    max_duration_ms max_inputs max_outputs complete local_only missing stale
    budget_exhausted topology_complete amount_complete_transaction_count
    selected_node_count hypotheses_enabled hypothesis_count pattern_count label_count
    source_rows node_count edge_count transaction_count output_count record_count
    path_count visited_node_count inspected_edge_count invalid_observations
    cache_rejected custody_fresh missing_node_count conflicting_node_count reference_reconciling_count
    complete_transaction_count reference_node_count frontier_omitted_count
    pruning count analytics depth node_limit edge_limit include_relations
    include_hypotheses start end reversible accounting_authority taint_inference
    cluster_defining changed comparison base current added_nodes removed_nodes
    changed_nodes added_edges removed_edges changed_edges before after
    added_findings removed_findings changed_findings added_clusters removed_clusters changed_clusters
    added_patterns removed_patterns changed_patterns added_exposure removed_exposure changed_exposure
    added_transaction_features removed_transaction_features changed_transaction_features coverage_changed
    local_graph backward_forward_trace bounded_alternative_paths
    cross_rail_custody_relations network_acquisition taint_attribution
    global_chain_completeness labels error retryable details
    wallet_inventory stored_transaction transactions wallets wallet_utxos
    transaction_graph_cache journal_custody_decisions journal_custody_economic_relations
    chain_analysis_labels chain_analysis_observations
    transaction_features features extractor_version scenario computation cache_hits
    request progress cancel_requested elapsed_ms phase job_id error_code receipt_scope
    max_received_fee_msat max_paid_fee_msat completed input_count output_count
    dataset_id datasets manifest content_sha256 dataset_state_digest state_digest
    byte_count row_count record_number claims dataset_version visibility dataset_status
    match_count subject_count active_dataset_count inactive_by_date_count lookup_queries
    match_limit match_limit_per_visibility truncated complete_chain_coverage valid_from valid_until
    scripts_skipped subject_limit subjects_truncated
    transaction_facts inputs outputs totals psbt_version validation input_msat output_msat
    final_vsize final_fee_rate_sat_vb known_input_amounts previous_transaction_hashes_verified
    missing_input_indices network_source network_verified chain_membership_verified
    unspentness_verified signatures_verified input_metadata output_derivation_metadata_present
    global_xpubs_present transaction_modifiable_flags utxo_evidence input_index output_index
    delta inputs_added inputs_removed output_scripts_added output_scripts_removed
    features_changed findings_added findings_removed payjoin checks additional_input_count
    receiver_contribution_msat negotiation_verified safe_to_sign metadata
    payment_output_substitution_allowed
    feature_codes contradictions value values signals_bip125 relative_locks minimum
    relative_lock_interpretation_available enabled counts by_index observations sighash
    low_r low_s verified equal_groups round_1000_sat_count bip69_value_script_order
    p2pkh p2sh p2wpkh p2wsh p2tr op_return p2pk witness_unknown other unknown
";

const TIME_FIELDS: &[&str] = &[
    "created_at",
    "updated_at",
    "occurred_at",
    "confirmed_at",
    "observed_at",
    "checked_at",
    "acknowledged_at",
    "confirmation_observed_at",
    "start",
    "end",
    "valid_from",
    "valid_until",
];

/// Prefixes of public chain identifiers a code may not carry.
const PUBLIC_PREFIXES: &[&str] = &[
    "bc1", "tb1", "bcrt1", "ex1", "lq1", "tex1", "tlq1", "ert1", "el1", "xpub", "tpub", "ypub",
    "zpub",
];

struct Patterns {
    code: Regex,
    hex64: Regex,
    asset: Regex,
    integer: Regex,
    fee_rate: Regex,
    time: Regex,
}

fn full(pattern: &str) -> Regex {
    pyre::compile(&format!(r"\A(?:{pattern})\Z"))
}

fn patterns() -> &'static Patterns {
    static PATTERNS: OnceLock<Patterns> = OnceLock::new();
    PATTERNS.get_or_init(|| Patterns {
        code: full(r"[a-z][a-z0-9_]*(?:-[a-z0-9_]+)*"),
        hex64: full(r"[a-f0-9]{64}"),
        asset: full(r"[A-Z]{2,8}"),
        integer: full(r"-?(?:0|[1-9][0-9]*)"),
        fee_rate: full(r"[0-9]+(?:\.[0-9]+)?"),
        time: full(r"[0-9T:.+Z-]{10,40}"),
    })
}

fn matches(regex: &Regex, text: &PyStr) -> bool {
    pyre::search(regex, text)
}

/// `str.isdecimal()`.
fn is_decimal(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| kassiber_pytext::decimal_value(u32::from(c)).is_some())
}

fn known_field(key: &str) -> bool {
    [
        HANDLE_FIELDS,
        REFERENCE_FIELDS,
        REFERENCE_LISTS,
        CODE_FIELDS,
        CODE_LISTS,
    ]
    .iter()
    .any(|set| set.contains(&key))
        || key == "source"
        || FIELDS.split_whitespace().any(|field| field == key)
}

/// `_handle(value)`: a canonical UUID (any case), an authored API handle.
fn is_uuid(text: &str) -> bool {
    let bytes = text.as_bytes();
    // Python's `uuid.UUID` also accepts braces, `urn:uuid:` and no hyphens,
    // but its `str()` is the hyphenated form, so only that form compares
    // equal to the lowered input.
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => *byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
}

enum Projected {
    Keep(Value),
    Omitted,
}

struct Projector<'a> {
    issue: &'a mut dyn FnMut(&str) -> Result<String, ToolError>,
}

impl Projector<'_> {
    fn reference(&mut self, text: &PyStr) -> Result<Projected, ToolError> {
        let value = text.as_str().ok_or(ToolError::Unexpected)?;
        Ok(Projected::Keep(s((self.issue)(value)?)))
    }

    fn project(
        &mut self,
        item: &Value,
        field: &str,
        parent: Option<&Value>,
    ) -> Result<Projected, ToolError> {
        match item {
            Value::Dict(pairs) => {
                let mut result = Vec::new();
                for (key, child) in pairs {
                    let Key::Str(name) = key else { continue };
                    let Some(name_text) = name.as_str() else {
                        continue;
                    };
                    if !known_field(name_text) && !is_decimal(name_text) {
                        continue;
                    }
                    if let Projected::Keep(value) = self.project(child, name_text, Some(item))? {
                        result.push((key.clone(), value));
                    }
                }
                Ok(Projected::Keep(Value::Dict(result)))
            }
            Value::List(items) => {
                let mut result = Vec::new();
                for child in items {
                    if let Projected::Keep(value) = self.project(child, field, parent)? {
                        result.push(value);
                    }
                }
                Ok(Projected::Keep(Value::List(result)))
            }
            Value::Null | Value::Int(_) | Value::Float(_) | Value::Bool(_) => {
                Ok(Projected::Keep(item.clone()))
            }
            Value::Str(text) => self.text(text, field, parent),
            _ => Ok(Projected::Omitted),
        }
    }

    fn text(
        &mut self,
        text: &PyStr,
        field: &str,
        parent: Option<&Value>,
    ) -> Result<Projected, ToolError> {
        let p = patterns();
        let keep = || Ok(Projected::Keep(Value::Str(text.clone())));
        if REFERENCE_FIELDS.contains(&field) || REFERENCE_LISTS.contains(&field) {
            return self.reference(text);
        }
        if field == "id" {
            return if text.as_str().is_some_and(is_uuid) {
                keep()
            } else {
                self.reference(text)
            };
        }
        if HANDLE_FIELDS.contains(&field) {
            return keep();
        }
        if field == "source" {
            let edge = parent.is_some_and(|parent| {
                matches!(get(parent, "kind"), Some(Value::Str(kind))
                    if ["creates", "spends", "custody", "hypothesis"].iter().any(|edge| kind.is(edge)))
                    && get(parent, "target").is_some()
            });
            if edge {
                return self.reference(text);
            }
            return if SOURCE_CODES.iter().any(|code| text.is(code)) {
                keep()
            } else {
                Ok(Projected::Omitted)
            };
        }
        if CODE_FIELDS.contains(&field) || CODE_LISTS.contains(&field) {
            let plain = pyre::lossy(text);
            let safe = pyre::char_len(text) <= 100
                && matches(&p.code, text)
                && !matches(&p.hex64, text)
                && !PUBLIC_PREFIXES
                    .iter()
                    .any(|prefix| plain.starts_with(prefix));
            return if safe { keep() } else { Ok(Projected::Omitted) };
        }
        if ["asset", "target_asset", "source_asset"].contains(&field) {
            return if matches(&p.asset, text) {
                keep()
            } else {
                Ok(Projected::Keep(s("unknown_asset")))
            };
        }
        if field.ends_with("_msat")
            || ["interpretation_count", "interpretation_count_lower_bound"].contains(&field)
            || is_decimal(field)
        {
            return if matches(&p.integer, text) {
                keep()
            } else {
                Ok(Projected::Omitted)
            };
        }
        if field == "final_fee_rate_sat_vb" {
            return if matches(&p.fee_rate, text) {
                keep()
            } else {
                Ok(Projected::Omitted)
            };
        }
        if TIME_FIELDS.contains(&field) {
            return if matches(&p.time, text) {
                keep()
            } else {
                Ok(Projected::Omitted)
            };
        }
        Ok(Projected::Omitted)
    }
}

/// `project_ai_result(conn, profile_id, value)`, with `issue` minting each
/// handle (`_reference`) in the order Python mints them.
pub fn project_ai_result(
    value: &Value,
    issue: &mut dyn FnMut(&str) -> Result<String, ToolError>,
) -> Result<Value, ToolError> {
    let mut projector = Projector { issue };
    let mut result = match projector.project(value, "", None)? {
        Projected::Keep(result) => result,
        Projected::Omitted => return Ok(Value::Null),
    };
    if matches!(result, Value::Dict(_)) {
        set(
            &mut result,
            "ai_reference_scope",
            s("current_process_and_book"),
        );
        set(
            &mut result,
            "ai_projection",
            s("opaque_graph_references_no_public_chain_identifiers"),
        );
    }
    Ok(result)
}

/// Kinds only an on-device provider may run (`_chain_analysis_ai_payload`).
pub fn on_device_kind(kind: &str) -> bool {
    kind.starts_with("ui.chain_analysis.sources.")
        || [
            "ui.chain_analysis.acquire.plan",
            "ui.chain_analysis.acquire.apply",
            "ui.chain_analysis.datasets.preview",
            "ui.chain_analysis.datasets.import",
            "ui.chain_analysis.datasets.preview.start",
            "ui.chain_analysis.datasets.import.start",
        ]
        .contains(&kind)
}
