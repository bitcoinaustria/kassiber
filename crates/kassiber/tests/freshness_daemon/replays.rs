//! Parity tests that become replays of their recorded daemon sessions:
//! each named parameter's sessions, as the pytest test drove them.

use crate::replay;

// tests/test_rust_freshness_product.py
pub fn test_product_argument_errors_match_reference_without_effects() {
    replay(
        "product",
        "test_product_argument_errors_match_reference_without_effects",
        &["ui.freshness.run-extra10"],
    );
}

pub fn test_product_automatic_graph_lookup_matches_reference() {
    replay(
        "product",
        "test_product_automatic_graph_lookup_matches_reference",
        &["ui.wallets.sync-esplora"],
    );
}

pub fn test_product_enqueue_only_and_run_limit() {
    replay(
        "product",
        "test_product_enqueue_only_and_run_limit",
        &["False-ui.freshness.run"],
    );
}

pub fn test_product_selection_refuses_whole_invocation_without_effects() {
    replay(
        "product",
        "test_product_selection_refuses_whole_invocation_without_effects",
        &["missing-id-ui.freshness.run", "unported-ui.freshness.run"],
    );
}

pub fn test_product_sync_envelopes_events_and_all_tables() {
    replay(
        "product",
        "test_product_sync_envelopes_events_and_all_tables",
        &["empty-ui.wallets.sync-esplora"],
    );
}

pub fn test_product_sync_restart_sequences() {
    replay(
        "product",
        "test_product_sync_restart_sequences",
        &[
            "force-full-ui.wallets.sync-esplora",
            "force-without-state-ui.workspace.freshness.run-electrum",
            "force-without-state-ui.workspace.freshness.run-esplora",
        ],
    );
}

pub fn test_product_wallet_selection_direct_sync_matches_python() {
    replay(
        "product",
        "test_product_wallet_selection_direct_sync_matches_python",
        &["no-source", "paused"],
    );
}

pub fn test_product_workspace_preserves_active_context_and_book_order() {
    replay(
        "product",
        "test_product_workspace_preserves_active_context_and_book_order",
        &["esplora"],
    );
}

// tests/test_rust_freshness_journals.py
pub fn test_default_refresh_current_after_repeat() {
    replay("journals", "test_default_refresh_current_after_repeat", &[]);
}

pub fn test_default_refresh_envelopes_events_and_all_tables() {
    replay(
        "journals",
        "test_default_refresh_envelopes_events_and_all_tables",
        &[
            "empty-ui.freshness.run-esplora",
            "empty-ui.wallets.sync-esplora",
        ],
    );
}

pub fn test_default_refresh_policy_and_coalesced_pairing() {
    replay(
        "journals",
        "test_default_refresh_policy_and_coalesced_pairing",
        &["disabled-ui.freshness.run"],
    );
}

pub fn test_journal_job_pairing_and_caller_authority() {
    replay(
        "journals",
        "test_journal_job_pairing_and_caller_authority",
        &["False-exact", "True-exact"],
    );
}

pub fn test_journal_refusal_census_and_hybrid_only_replay() {
    // The development journal replay is armed (it ran Python); after a first
    // committed build it must not run, so the stand-in that always fails is
    // never reached.
    replay(
        "journals",
        "test_journal_refusal_census_and_hybrid_only_replay",
        &["after-first-commit-True"],
    );
}

// tests/test_rust_freshness_native.py, daemon half
pub fn test_native_freshness_live_stream_matches_python() {
    replay(
        "native",
        "test_native_freshness_live_stream_matches_python",
        &[],
    );
}
