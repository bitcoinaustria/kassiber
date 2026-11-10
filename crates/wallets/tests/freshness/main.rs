//! The freshness queue, source admission and job execution, replayed from
//! the parity tests that compared `examples/freshness_fixture.rs` with the
//! Python reference (`tests/test_rust_freshness_{queue,admission,execution}.py`).
//!
//! Each Rust test carries its pytest test's name and replays every recorded
//! parameter of it: the same operations against the same starting book,
//! through the fixture's own code in process, must print the identical line
//! and leave the identical book. The recordings (`tests/data/freshness/`)
//! were made once where Rust and Python agreed; see `golden.rs`.

#[path = "../../examples/freshness_fixture.rs"]
#[allow(dead_code)]
mod freshness_fixture;
mod golden;

use golden::{assert_same_book, calls, label, restore, snapshot, string, value, Golden, Scratch};
use kassiber_envelope::{get, py_str};

/// The fixture ran one invocation per process; admission's execution slot
/// is process-wide, so in-process replays take turns.
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Replay every recorded case of `test` from the golden `module`.
fn replay(module: &str, test: &str) {
    let golden = Golden::load(module);
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    for case in golden.cases(test) {
        let name = label(case);
        let scratch = Scratch::new(&name);
        for (index, call) in calls(case).iter().enumerate() {
            assert_eq!(string(call, "fixture"), "freshness_fixture", "{name}");
            let input = scratch.place(&string(call, "input"));
            let parsed = kassiber_pytext::loads(&input).unwrap();
            let book = std::path::PathBuf::from(py_str(get(&parsed, "book").unwrap()));
            let _ = std::fs::remove_file(&book);
            restore(&golden, value(call, "before"), &book);
            let printed = freshness_fixture::run(&input);
            assert_eq!(
                printed,
                scratch.place(&string(call, "stdout")),
                "{name} call {index}: the fixture printed something else"
            );
            assert_same_book(
                &format!("{name} call {index}"),
                &snapshot(&book),
                value(call, "after"),
            );
        }
    }
}

// tests/test_rust_freshness_queue.py
#[test]
fn test_freshness_queue_singleflight_force_and_running_owner() {
    replay(
        "queue",
        "test_freshness_queue_singleflight_force_and_running_owner",
    );
}

#[test]
fn test_freshness_queue_journal_merge() {
    replay("queue", "test_freshness_queue_journal_merge");
}

#[test]
fn test_freshness_queue_paused_enqueue_keeps_private_checkpoint() {
    replay(
        "queue",
        "test_freshness_queue_paused_enqueue_keeps_private_checkpoint",
    );
}

#[test]
fn test_freshness_queue_recovery_excludes_graph_and_other_profiles() {
    replay(
        "queue",
        "test_freshness_queue_recovery_excludes_graph_and_other_profiles",
    );
}

#[test]
fn test_freshness_queue_cancel_statuses_and_graph_blocking() {
    replay(
        "queue",
        "test_freshness_queue_cancel_statuses_and_graph_blocking",
    );
}

#[test]
fn test_freshness_queue_dispatch_rechecks_pause_cooldown_profile_and_running() {
    replay(
        "queue",
        "test_freshness_queue_dispatch_rechecks_pause_cooldown_profile_and_running",
    );
}

#[test]
fn test_freshness_queue_due_order_and_timestamp_boundaries() {
    replay(
        "queue",
        "test_freshness_queue_due_order_and_timestamp_boundaries",
    );
}

#[test]
fn test_freshness_queue_state_precedence_and_preservation() {
    replay(
        "queue",
        "test_freshness_queue_state_precedence_and_preservation",
    );
}

#[test]
fn test_freshness_queue_policy_defaults_legacy_and_saved() {
    replay(
        "queue",
        "test_freshness_queue_policy_defaults_legacy_and_saved",
    );
}

#[test]
fn test_freshness_queue_redacts_results_and_retains_private_state() {
    replay(
        "queue",
        "test_freshness_queue_redacts_results_and_retains_private_state",
    );
}

#[test]
fn test_freshness_queue_removed_progress_cannot_recreate_source() {
    replay(
        "queue",
        "test_freshness_queue_removed_progress_cannot_recreate_source",
    );
}

#[test]
fn test_freshness_queue_requested_jobs_do_not_borrow_other_work() {
    replay(
        "queue",
        "test_freshness_queue_requested_jobs_do_not_borrow_other_work",
    );
}

#[test]
fn test_freshness_queue_policy_updates_and_invalid_class() {
    replay(
        "queue",
        "test_freshness_queue_policy_updates_and_invalid_class",
    );
}

#[test]
fn test_freshness_queue_reset_and_delete_scope() {
    replay("queue", "test_freshness_queue_reset_and_delete_scope");
}

#[test]
fn test_freshness_queue_redaction_key_collisions_and_surrogates() {
    replay(
        "queue",
        "test_freshness_queue_redaction_key_collisions_and_surrogates",
    );
}

#[test]
fn test_freshness_queue_dispatch_order_prioritizes_exact_terminal_results() {
    replay(
        "queue",
        "test_freshness_queue_dispatch_order_prioritizes_exact_terminal_results",
    );
}

#[test]
fn test_freshness_queue_selected_batches_and_duplicates() {
    replay(
        "queue",
        "test_freshness_queue_selected_batches_and_duplicates",
    );
}

#[test]
fn test_freshness_workspace_order_preserves_active_context() {
    replay(
        "queue",
        "test_freshness_workspace_order_preserves_active_context",
    );
}

// tests/test_rust_freshness_admission.py
#[test]
fn test_freshness_source_specs_match_python() {
    replay("admission", "test_freshness_source_specs_match_python");
}

#[test]
fn test_freshness_source_validation_matches_python() {
    replay(
        "admission",
        "test_freshness_source_validation_matches_python",
    );
}

#[test]
fn test_freshness_source_account_routes_order_and_wallet_scope() {
    replay(
        "admission",
        "test_freshness_source_account_routes_order_and_wallet_scope",
    );
}

#[test]
fn test_freshness_source_policy_filter() {
    replay("admission", "test_freshness_source_policy_filter");
}

#[test]
fn test_freshness_admission_is_inert_and_matches_source_plan() {
    replay(
        "admission",
        "test_freshness_admission_is_inert_and_matches_source_plan",
    );
}

#[test]
fn test_freshness_admission_workspace_order_is_inert() {
    replay(
        "admission",
        "test_freshness_admission_workspace_order_is_inert",
    );
}

#[test]
fn test_freshness_admission_honors_disabled_journal_policy() {
    replay(
        "admission",
        "test_freshness_admission_honors_disabled_journal_policy",
    );
}

#[test]
fn test_freshness_admission_no_active_profile_and_local_only_wallets() {
    replay(
        "admission",
        "test_freshness_admission_no_active_profile_and_local_only_wallets",
    );
}

#[test]
fn test_freshness_admission_plans_kinds_without_a_freshness_source() {
    replay(
        "admission",
        "test_freshness_admission_plans_kinds_without_a_freshness_source",
    );
}

#[test]
fn test_freshness_admission_plans_every_wallet_an_id_or_label_selects() {
    replay(
        "admission",
        "test_freshness_admission_plans_every_wallet_an_id_or_label_selects",
    );
}

#[test]
fn test_freshness_admission_validation_is_not_capability_fallback() {
    replay(
        "admission",
        "test_freshness_admission_validation_is_not_capability_fallback",
    );
}

#[test]
fn test_freshness_admission_plans_btcpay_rates_and_file_sources() {
    replay(
        "admission",
        "test_freshness_admission_plans_btcpay_rates_and_file_sources",
    );
}

#[test]
fn test_freshness_admission_refuses_unsupported_whole_shapes() {
    replay(
        "admission",
        "test_freshness_admission_refuses_unsupported_whole_shapes",
    );
}

#[test]
fn test_freshness_admission_plans_an_address_list() {
    replay(
        "admission",
        "test_freshness_admission_plans_an_address_list",
    );
}

#[test]
fn test_freshness_admission_plans_a_profile_with_several_wallets() {
    replay(
        "admission",
        "test_freshness_admission_plans_a_profile_with_several_wallets",
    );
}

#[test]
fn test_freshness_admission_mixed_workspace_refuses_before_recovery() {
    replay(
        "admission",
        "test_freshness_admission_mixed_workspace_refuses_before_recovery",
    );
}

#[test]
fn test_freshness_admission_execution_slot_released_after_refusal_and_drop() {
    replay(
        "admission",
        "test_freshness_admission_execution_slot_released_after_refusal_and_drop",
    );
}

#[test]
fn test_freshness_admission_rechecks_scope_config_and_policy() {
    replay(
        "admission",
        "test_freshness_admission_rechecks_scope_config_and_policy",
    );
}

#[test]
fn test_freshness_admission_rejects_different_connection_and_backend() {
    replay(
        "admission",
        "test_freshness_admission_rejects_different_connection_and_backend",
    );
}

// tests/test_rust_freshness_execution.py
#[test]
fn test_freshness_execution_success_checkpoint_and_progress() {
    replay(
        "execution",
        "test_freshness_execution_success_checkpoint_and_progress",
    );
}

#[test]
fn test_freshness_execution_preserves_checkpoint_without_mapping() {
    replay(
        "execution",
        "test_freshness_execution_preserves_checkpoint_without_mapping",
    );
}

#[test]
fn test_freshness_execution_retry_after_matches_python() {
    replay(
        "execution",
        "test_freshness_execution_retry_after_matches_python",
    );
}

#[test]
fn test_freshness_execution_exponential_backoff() {
    replay("execution", "test_freshness_execution_exponential_backoff");
}

#[test]
fn test_freshness_execution_database_busy_rolls_back_and_retries() {
    replay(
        "execution",
        "test_freshness_execution_database_busy_rolls_back_and_retries",
    );
}

#[test]
fn test_freshness_execution_errors_and_ui_redaction() {
    replay(
        "execution",
        "test_freshness_execution_errors_and_ui_redaction",
    );
}

#[test]
fn test_freshness_execution_generic_failure_and_handler_not_found() {
    replay(
        "execution",
        "test_freshness_execution_generic_failure_and_handler_not_found",
    );
}

#[test]
fn test_freshness_execution_removed_job_never_resurrects_source() {
    replay(
        "execution",
        "test_freshness_execution_removed_job_never_resurrects_source",
    );
}

#[test]
fn test_freshness_execution_cancellation_checked_before_and_after_handler() {
    replay(
        "execution",
        "test_freshness_execution_cancellation_checked_before_and_after_handler",
    );
}

#[test]
fn test_freshness_execution_retry_success_retains_attempt_history() {
    replay(
        "execution",
        "test_freshness_execution_retry_success_retains_attempt_history",
    );
}

#[test]
fn test_freshness_execution_missing_job_and_handler_are_distinct() {
    replay(
        "execution",
        "test_freshness_execution_missing_job_and_handler_are_distinct",
    );
}

#[test]
fn test_freshness_execution_requested_dispatch_rechecks_controls() {
    replay(
        "execution",
        "test_freshness_execution_requested_dispatch_rechecks_controls",
    );
}

#[test]
fn test_freshness_execution_aggregation_matches_daemon() {
    replay(
        "execution",
        "test_freshness_execution_aggregation_matches_daemon",
    );
}

#[test]
fn test_freshness_execution_admitted_coordinator_matches_daemon() {
    replay(
        "execution",
        "test_freshness_execution_admitted_coordinator_matches_daemon",
    );
}

#[test]
fn test_freshness_execution_no_active_profile_matches_daemon() {
    replay(
        "execution",
        "test_freshness_execution_no_active_profile_matches_daemon",
    );
}

#[test]
fn test_freshness_execution_ui_text_matches_python() {
    replay(
        "execution",
        "test_freshness_execution_ui_text_matches_python",
    );
}
