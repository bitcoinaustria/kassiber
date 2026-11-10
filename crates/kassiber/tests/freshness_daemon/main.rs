//! The freshness daemon tests that drove `kassiber daemon` against a
//! loopback chain (`tests/test_rust_freshness_{product,journals,native,graph,
//! lifecycle}.py`), in Rust.
//!
//! Parity tests become replays of their recorded sessions (`session.rs`):
//! the same book, the same lines at the same points, the network from the
//! recorded tape, and the rows and book the Rust daemon produced while it
//! agreed with Python. Tests that act while a request is in flight hold the
//! taped response (`KASSIBER_TEST_NET_TAPE_HOLD`) as the pytest chain held
//! its reply, and keep their own assertions. Each Rust test carries its
//! pytest test's name.

#![cfg(feature = "test-harness")]

#[path = "../../../wallets/tests/freshness/golden.rs"]
#[allow(dead_code)]
mod golden;
mod graph;
mod journals;
mod lifecycle;
mod live;
mod product;
mod replays;
mod session;

use golden::{calls, label, string, Golden, Scratch};

/// Replay every recorded case of `test` (all its parameters, or those
/// `params` names) from the golden `module`.
fn replay(module: &str, test: &str, params: &[&str]) {
    let golden = Golden::load(module);
    let mut seen = 0;
    for case in golden.cases(test) {
        let param = string(case, "param");
        if !params.is_empty() && !params.contains(&param.as_str()) {
            continue;
        }
        seen += 1;
        let name = label(case);
        let scratch = Scratch::new(&name);
        for (index, call) in calls(case).iter().enumerate() {
            assert!(
                kassiber_envelope::truthy(golden::value(call, "session")),
                "{name}: call {index} is not a daemon session"
            );
            session::replay_session(&golden, &scratch, &name, index, call);
        }
    }
    if params.is_empty() {
        assert!(seen > 0, "{test}: no recording");
    } else {
        assert_eq!(
            seen,
            params.len(),
            "{test}: a named parameter has no recording"
        );
    }
}

// Each test is named after its pytest test (the mutation catalogs name
// these); the bodies live in the modules.

#[test]
fn test_live_freshness_controls_remain_available_during_fetch() {
    lifecycle::test_live_freshness_controls_remain_available_during_fetch();
}

#[test]
fn test_live_freshness_closed_output_stops_and_joins_the_request() {
    lifecycle::test_live_freshness_closed_output_stops_and_joins_the_request();
}

#[test]
fn test_live_freshness_eof_joins_the_request_before_exit() {
    lifecycle::test_live_freshness_eof_joins_the_request_before_exit();
}

#[test]
fn test_live_freshness_private_graph_handoff_and_discard() {
    lifecycle::test_live_freshness_private_graph_handoff_and_discard();
}

#[test]
fn test_live_freshness_graph_handoff_deduplicates_and_never_recovers_authority() {
    lifecycle::test_live_freshness_graph_handoff_deduplicates_and_never_recovers_authority();
}

#[test]
fn test_live_freshness_preconsent_and_early_request_fences() {
    lifecycle::test_live_freshness_preconsent_and_early_request_fences();
}

#[test]
fn test_live_freshness_refuses_replay_and_foreign_request_cancellation() {
    lifecycle::test_live_freshness_refuses_replay_and_foreign_request_cancellation();
}

#[test]
fn test_live_freshness_quiesces_before_lifecycle_acknowledgement() {
    lifecycle::test_live_freshness_quiesces_before_lifecycle_acknowledgement();
}

#[test]
fn test_live_freshness_encrypted_graph_handoff_drops_on_lock() {
    lifecycle::test_live_freshness_encrypted_graph_handoff_drops_on_lock();
}

#[test]
fn test_reference_source_control_checkpoint_repro_and_native_parity() {
    lifecycle::test_reference_source_control_checkpoint_repro_and_native_parity();
}

#[test]
fn test_native_graph_worker_controls_and_owned_connection() {
    graph::test_native_graph_worker_controls_and_owned_connection();
}

#[test]
fn test_native_graph_request_identity_and_replay_are_fenced() {
    graph::test_native_graph_request_identity_and_replay_are_fenced();
}

#[test]
fn test_native_graph_closed_output_joins_worker_before_exit() {
    graph::test_native_graph_closed_output_joins_worker_before_exit();
}

#[test]
fn test_native_graph_enabled_worker_waits_for_terminal_handoff() {
    graph::test_native_graph_enabled_worker_waits_for_terminal_handoff();
}

#[test]
fn test_native_graph_cancels_between_electrum_prevout_batches() {
    graph::test_native_graph_cancels_between_electrum_prevout_batches();
}

#[test]
fn test_native_graph_encrypted_worker_handoff_and_cleanup() {
    graph::test_native_graph_encrypted_worker_handoff_and_cleanup();
}

#[test]
fn test_product_source_controls_remain_available_during_fetch() {
    product::test_product_source_controls_remain_available_during_fetch();
}

#[test]
fn test_encrypted_journal_job_and_hybrid_key_handoff() {
    journals::test_encrypted_journal_job_and_hybrid_key_handoff();
}

#[test]
fn test_product_argument_errors_match_reference_without_effects() {
    replays::test_product_argument_errors_match_reference_without_effects();
}

#[test]
fn test_product_automatic_graph_lookup_matches_reference() {
    replays::test_product_automatic_graph_lookup_matches_reference();
}

#[test]
fn test_product_enqueue_only_and_run_limit() {
    replays::test_product_enqueue_only_and_run_limit();
}

#[test]
fn test_product_selection_refuses_whole_invocation_without_effects() {
    replays::test_product_selection_refuses_whole_invocation_without_effects();
}

#[test]
fn test_product_sync_envelopes_events_and_all_tables() {
    replays::test_product_sync_envelopes_events_and_all_tables();
}

#[test]
fn test_product_sync_restart_sequences() {
    replays::test_product_sync_restart_sequences();
}

#[test]
fn test_product_wallet_selection_direct_sync_matches_python() {
    replays::test_product_wallet_selection_direct_sync_matches_python();
}

#[test]
fn test_product_workspace_preserves_active_context_and_book_order() {
    replays::test_product_workspace_preserves_active_context_and_book_order();
}

#[test]
fn test_default_refresh_current_after_repeat() {
    replays::test_default_refresh_current_after_repeat();
}

#[test]
fn test_default_refresh_envelopes_events_and_all_tables() {
    replays::test_default_refresh_envelopes_events_and_all_tables();
}

#[test]
fn test_default_refresh_policy_and_coalesced_pairing() {
    replays::test_default_refresh_policy_and_coalesced_pairing();
}

#[test]
fn test_journal_job_pairing_and_caller_authority() {
    replays::test_journal_job_pairing_and_caller_authority();
}

#[test]
fn test_journal_refusal_census_and_hybrid_only_replay() {
    replays::test_journal_refusal_census_and_hybrid_only_replay();
}

#[test]
fn test_native_freshness_live_stream_matches_python() {
    replays::test_native_freshness_live_stream_matches_python();
}
