#![cfg(unix)]
//! Rust replacements for the pytest parity tests that held pytest-only
//! mutation kills in `crates/wallets/scripts/routing_mutations.json`
//! (plan 22, decision D1 option 3). Each test replays the `kassiber`
//! processes its pytest test started, from a golden recorded where Rust and
//! the Python reference agreed; see `replay.rs`. Test names follow the
//! pytest tests they replace (parameters appended).

mod replay;

use replay::{clock_numbers, replay, replay_pinned, replay_served, replay_with};

/// A Core Lightning sync stamps its bookkeeper balance snapshots with its
/// day, which keys the rows and their hashes. The goldens were recorded on
/// 2026-11-10; the test-build seam pins that day so they compare exactly.
const CLN_SNAPSHOT_DAY: &[(&str, &str)] = &[(
    "KASSIBER_RUST_TEST_CLN_SNAPSHOT_TIME",
    "2026-11-10T00:00:00Z",
)];

/// `tests/test_rust_backend_kind_aliases.py::test_liquid_esplora_kind_matches_python`
#[test]
fn test_liquid_esplora_kind_matches_python() {
    replay("backend_kind_aliases/test_liquid_esplora_kind_matches_python");
}

/// `tests/test_rust_backend_kind_aliases.py::test_mempool_kind_address_list_matches_python`
#[test]
fn test_mempool_kind_address_list_matches_python() {
    replay("backend_kind_aliases/test_mempool_kind_address_list_matches_python");
}

/// `tests/test_rust_backend_kind_aliases.py::test_mempool_kind_descriptor_matches_python[receive-change]`
#[test]
fn test_mempool_kind_descriptor_matches_python_receive_change() {
    replay("backend_kind_aliases/test_mempool_kind_descriptor_matches_python_receive_change");
}

/// `tests/test_rust_bip329_services.py::RustBip329ServicesTest::test_descriptor_wallets_match_through_the_derived_ownership_index`
#[test]
fn test_descriptor_wallets_match_through_the_derived_ownership_index() {
    replay("bip329_services/test_descriptor_wallets_match_through_the_derived_ownership_index");
}

/// `tests/test_rust_connections.py::test_btcpay_discover_daemon_bytes_requests_and_all_tables[btcpay24_read_only-sqlite]`
#[test]
fn test_btcpay_discover_daemon_bytes_requests_and_all_tables_btcpay24_read_only_sqlite() {
    replay_with("connections/test_btcpay_discover_daemon_bytes_requests_and_all_tables_btcpay24_read_only_sqlite", clock_numbers(&["age_seconds"]));
}

/// `tests/test_rust_connections.py::test_btcpay_discover_daemon_bytes_requests_and_all_tables[btcpay24_store_only_key-sqlite]`
#[test]
fn test_btcpay_discover_daemon_bytes_requests_and_all_tables_btcpay24_store_only_key_sqlite() {
    replay_with("connections/test_btcpay_discover_daemon_bytes_requests_and_all_tables_btcpay24_store_only_key_sqlite", clock_numbers(&["age_seconds"]));
}

/// `tests/test_rust_connections.py::test_btcpay_discover_daemon_bytes_requests_and_all_tables[wallet_history-sqlite]`
#[test]
fn test_btcpay_discover_daemon_bytes_requests_and_all_tables_wallet_history_sqlite() {
    replay_with("connections/test_btcpay_discover_daemon_bytes_requests_and_all_tables_wallet_history_sqlite", clock_numbers(&["age_seconds"]));
}

/// `tests/test_rust_core_rpc_sync.py::test_address_list_matches_python`
#[test]
fn test_address_list_matches_python() {
    replay("core_rpc_sync/test_address_list_matches_python");
}

/// `tests/test_rust_core_rpc_sync.py::test_collaborative_spend_and_foreign_parents_match_python`
#[test]
fn test_collaborative_spend_and_foreign_parents_match_python() {
    replay("core_rpc_sync/test_collaborative_spend_and_foreign_parents_match_python");
}

/// `tests/test_rust_core_rpc_sync.py::test_deficit_rescan_and_the_widened_range_match_python`
#[test]
fn test_deficit_rescan_and_the_widened_range_match_python() {
    replay("core_rpc_sync/test_deficit_rescan_and_the_widened_range_match_python");
}

/// `tests/test_rust_core_rpc_sync.py::test_descriptor_wallet_rounds_match_python`
#[test]
fn test_descriptor_wallet_rounds_match_python() {
    replay("core_rpc_sync/test_descriptor_wallet_rounds_match_python");
}

/// `tests/test_rust_core_rpc_sync.py::test_existing_node_wallet_is_loaded_like_python`
#[test]
fn test_existing_node_wallet_is_loaded_like_python() {
    replay("core_rpc_sync/test_existing_node_wallet_is_loaded_like_python");
}

/// `tests/test_rust_core_rpc_sync.py::test_htlc_claim_and_refund_match_python`
#[test]
fn test_htlc_claim_and_refund_match_python() {
    replay("core_rpc_sync/test_htlc_claim_and_refund_match_python");
}

/// `tests/test_rust_core_rpc_sync.py::test_pruned_node_without_a_birthday_refuses_like_python`
#[test]
fn test_pruned_node_without_a_birthday_refuses_like_python() {
    replay("core_rpc_sync/test_pruned_node_without_a_birthday_refuses_like_python");
}

/// `tests/test_rust_core_rpc_sync.py::test_recreated_node_wallet_replays_like_python`
#[test]
fn test_recreated_node_wallet_replays_like_python() {
    replay("core_rpc_sync/test_recreated_node_wallet_replays_like_python");
}

/// `tests/test_rust_core_rpc_sync.py::test_spent_receive_raises_the_highest_used_index_like_python`
#[test]
fn test_spent_receive_raises_the_highest_used_index_like_python() {
    replay("core_rpc_sync/test_spent_receive_raises_the_highest_used_index_like_python");
}

/// `tests/test_rust_electrum_compatibility.py::test_dependency_discovery_overlap_matches_python`
#[test]
fn test_dependency_discovery_overlap_matches_python() {
    replay("electrum_compatibility/test_dependency_discovery_overlap_matches_python");
}

/// `tests/test_rust_freshness_sources.py::test_btcpay_freshness_jobs_match_python[ledger-and-account-route-sqlite]`
#[test]
fn test_btcpay_freshness_jobs_match_python_ledger_and_account_route_sqlite() {
    replay(
        "freshness_sources/test_btcpay_freshness_jobs_match_python_ledger_and_account_route_sqlite",
    );
}

/// `tests/test_rust_freshness_sources.py::test_btcpay_freshness_jobs_match_python[source-and-wallet-route-sqlite]`
#[test]
fn test_btcpay_freshness_jobs_match_python_source_and_wallet_route_sqlite() {
    replay(
        "freshness_sources/test_btcpay_freshness_jobs_match_python_source_and_wallet_route_sqlite",
    );
}

/// `tests/test_rust_freshness_sources.py::test_file_sourced_wallet_selection_syncs_directly_like_python`
#[test]
fn test_file_sourced_wallet_selection_syncs_directly_like_python() {
    replay("freshness_sources/test_file_sourced_wallet_selection_syncs_directly_like_python");
}

/// `tests/test_rust_freshness_sources.py::test_market_rate_coverage_seeds_then_refuses_live_lookups_without_egress[coingecko]`
#[test]
fn test_market_rate_coverage_seeds_then_refuses_live_lookups_without_egress_coingecko() {
    replay("freshness_sources/test_market_rate_coverage_seeds_then_refuses_live_lookups_without_egress_coingecko");
}

/// `tests/test_rust_freshness_sources.py::test_wallets_without_freshness_sources_do_not_refuse_a_run[core-silent-payment]`
#[test]
fn test_wallets_without_freshness_sources_do_not_refuse_a_run_core_silent_payment() {
    replay("freshness_sources/test_wallets_without_freshness_sources_do_not_refuse_a_run_core_silent_payment");
}

/// `tests/test_rust_liquid_compatibility.py::test_liquid_compatibility_books_match_python[electrum-extra0-custom_timeout-fresh]`
#[test]
fn test_liquid_compatibility_books_match_python_electrum_extra0_custom_timeout_fresh() {
    replay("liquid_compatibility/test_liquid_compatibility_books_match_python_electrum_extra0_custom_timeout_fresh");
}

/// `tests/test_rust_liquid_compatibility.py::test_liquid_compatibility_books_match_python[esplora-extra1-http_route_policy-extend]`
#[test]
fn test_liquid_compatibility_books_match_python_esplora_extra1_http_route_policy_extend() {
    replay("liquid_compatibility/test_liquid_compatibility_books_match_python_esplora_extra1_http_route_policy_extend");
}

/// `tests/test_rust_liquid_compatibility.py::test_liquid_compatibility_books_match_python[esplora-extra1-http_route_policy-fresh]`
#[test]
fn test_liquid_compatibility_books_match_python_esplora_extra1_http_route_policy_fresh() {
    replay("liquid_compatibility/test_liquid_compatibility_books_match_python_esplora_extra1_http_route_policy_fresh");
}

/// `tests/test_rust_liquid_compatibility.py::test_liquid_dependency_discovery_overlap_matches_python`
#[test]
fn test_liquid_dependency_discovery_overlap_matches_python() {
    replay_served("liquid_compatibility/test_liquid_dependency_discovery_overlap_matches_python");
}

/// `tests/test_rust_liquid_view_keys.py::test_legacy_set_view_key_matches_python[esplora]`
#[test]
fn test_legacy_set_view_key_matches_python_esplora() {
    replay("liquid_view_keys/test_legacy_set_view_key_matches_python_esplora");
}

/// `tests/test_rust_liquid_view_keys.py::test_view_key_books_match_python[electrum-fresh]`
#[test]
fn test_view_key_books_match_python_electrum_fresh() {
    replay("liquid_view_keys/test_view_key_books_match_python_electrum_fresh");
}

/// `tests/test_rust_report_read_unconfigured.py::RustReportReadWithoutBackendsTest::test_disabled_source_class_queues_nothing_like_python`
#[test]
fn test_disabled_source_class_queues_nothing_like_python() {
    replay("report_read_unconfigured/test_disabled_source_class_queues_nothing_like_python");
}

/// `tests/test_rust_report_read_unconfigured.py::RustReportReadWithoutBackendsTest::test_report_read_runs_the_wallet_sources_like_python`
#[test]
fn test_report_read_runs_the_wallet_sources_like_python() {
    replay_with(
        "report_read_unconfigured/test_report_read_runs_the_wallet_sources_like_python",
        clock_numbers(&["retry_after_seconds"]),
    );
}

/// `tests/test_rust_transaction_graph_public.py::RustTransactionGraphPublicLookupTest::test_backend_choice_matches_python`
#[test]
fn test_backend_choice_matches_python() {
    replay("transaction_graph_public/test_backend_choice_matches_python");
}

/// `tests/test_rust_transaction_graph_public.py::RustTransactionGraphPublicLookupTest::test_core_failures_warn_like_python`
#[test]
fn test_core_failures_warn_like_python() {
    replay("transaction_graph_public/test_core_failures_warn_like_python");
}

/// `tests/test_rust_transaction_graph_public.py::RustTransactionGraphPublicLookupTest::test_core_inline_prevouts_need_no_parent_like_python`
#[test]
fn test_core_inline_prevouts_need_no_parent_like_python() {
    replay("transaction_graph_public/test_core_inline_prevouts_need_no_parent_like_python");
}

/// `tests/test_rust_transaction_graph_public.py::RustTransactionGraphPublicLookupTest::test_core_lookup_attaches_parents_like_python`
#[test]
fn test_core_lookup_attaches_parents_like_python() {
    replay("transaction_graph_public/test_core_lookup_attaches_parents_like_python");
}

/// `tests/test_rust_transaction_graph_public.py::RustTransactionGraphPublicLookupTest::test_core_parent_failures_continue_or_stop_like_python`
#[test]
fn test_core_parent_failures_continue_or_stop_like_python() {
    replay("transaction_graph_public/test_core_parent_failures_continue_or_stop_like_python");
}

/// `tests/test_rust_transaction_graph_public.py::RustTransactionGraphPublicLookupTest::test_electrum_lookup_attaches_parents_like_python`
#[test]
fn test_electrum_lookup_attaches_parents_like_python() {
    replay("transaction_graph_public/test_electrum_lookup_attaches_parents_like_python");
}

/// `tests/test_rust_transaction_graph_public.py::RustTransactionGraphPublicLookupTest::test_electrum_missing_parent_and_wrong_bytes_warn_like_python`
#[test]
fn test_electrum_missing_parent_and_wrong_bytes_warn_like_python() {
    replay(
        "transaction_graph_public/test_electrum_missing_parent_and_wrong_bytes_warn_like_python",
    );
}

/// `tests/test_rust_transaction_graph_public.py::RustTransactionGraphPublicLookupTest::test_explorer_failures_warn_like_python`
#[test]
fn test_explorer_failures_warn_like_python() {
    replay("transaction_graph_public/test_explorer_failures_warn_like_python");
}

/// `tests/test_rust_transaction_graph_public.py::RustTransactionGraphPublicLookupTest::test_explorer_lookup_matches_python`
#[test]
fn test_explorer_lookup_matches_python() {
    replay("transaction_graph_public/test_explorer_lookup_matches_python");
}

/// `tests/test_rust_transaction_graph_public.py::RustTransactionGraphPublicLookupTest::test_liquid_electrum_lookup_matches_python`
#[test]
fn test_liquid_electrum_lookup_matches_python() {
    replay("transaction_graph_public/test_liquid_electrum_lookup_matches_python");
}

/// `tests/test_rust_wallet_sync_bark.py::test_a_bark_wallet_whose_plugin_stops_serving_fails_like_python`
#[test]
fn test_a_bark_wallet_whose_plugin_stops_serving_fails_like_python() {
    replay("wallet_sync_bark/test_a_bark_wallet_whose_plugin_stops_serving_fails_like_python");
}

/// `tests/test_rust_wallet_sync_bark.py::test_a_ledger_switches_in_its_freshness_job_like_python[sqlite]`
#[test]
fn test_a_ledger_switches_in_its_freshness_job_like_python_sqlite() {
    replay("wallet_sync_bark/test_a_ledger_switches_in_its_freshness_job_like_python_sqlite");
}

/// `tests/test_rust_wallet_sync_bark.py::test_a_ledger_switches_to_the_bark_wallet_like_python`
#[test]
fn test_a_ledger_switches_to_the_bark_wallet_like_python() {
    replay("wallet_sync_bark/test_a_ledger_switches_to_the_bark_wallet_like_python");
}

/// `tests/test_rust_wallet_sync_bark.py::test_a_ledger_that_cannot_switch_stays_like_python[amount]`
#[test]
fn test_a_ledger_that_cannot_switch_stays_like_python_amount() {
    replay("wallet_sync_bark/test_a_ledger_that_cannot_switch_stays_like_python_amount");
}

/// `tests/test_rust_wallet_sync_bark.py::test_a_ledger_that_cannot_switch_stays_like_python[provenance]`
#[test]
fn test_a_ledger_that_cannot_switch_stays_like_python_provenance() {
    replay("wallet_sync_bark/test_a_ledger_that_cannot_switch_stays_like_python_provenance");
}

/// `tests/test_rust_wallet_sync_bark.py::test_a_ledger_that_cannot_switch_stays_like_python[unmatched]`
#[test]
fn test_a_ledger_that_cannot_switch_stays_like_python_unmatched() {
    replay("wallet_sync_bark/test_a_ledger_that_cannot_switch_stays_like_python_unmatched");
}

/// `tests/test_rust_wallet_sync_bark.py::test_a_lightning_payment_that_cannot_move_refuses_like_python[evidence]`
#[test]
fn test_a_lightning_payment_that_cannot_move_refuses_like_python_evidence() {
    replay(
        "wallet_sync_bark/test_a_lightning_payment_that_cannot_move_refuses_like_python_evidence",
    );
}

/// `tests/test_rust_wallet_sync_bark.py::test_a_lightning_payment_that_cannot_move_refuses_like_python[node]`
#[test]
fn test_a_lightning_payment_that_cannot_move_refuses_like_python_node() {
    replay("wallet_sync_bark/test_a_lightning_payment_that_cannot_move_refuses_like_python_node");
}

/// `tests/test_rust_wallet_sync_bark.py::test_a_movement_paying_two_payments_links_each_like_python`
#[test]
fn test_a_movement_paying_two_payments_links_each_like_python() {
    replay("wallet_sync_bark/test_a_movement_paying_two_payments_links_each_like_python");
}

/// `tests/test_rust_wallet_sync_bark.py::test_a_recorded_store_meets_the_books_network_like_python[main]`
#[test]
fn test_a_recorded_store_meets_the_books_network_like_python_main() {
    replay("wallet_sync_bark/test_a_recorded_store_meets_the_books_network_like_python_main");
}

/// `tests/test_rust_wallet_sync_bark.py::test_bark_cli_routes_like_python[attach-refused]`
#[test]
fn test_bark_cli_routes_like_python_attach_refused() {
    replay("wallet_sync_bark/test_bark_cli_routes_like_python_attach_refused");
}

/// `tests/test_rust_wallet_sync_bark.py::test_bark_freshness_jobs_match_python[sqlite]`
#[test]
fn test_bark_freshness_jobs_match_python_sqlite() {
    replay("wallet_sync_bark/test_bark_freshness_jobs_match_python_sqlite");
}

/// `tests/test_rust_wallet_sync_bark.py::test_bark_rows_refuse_or_stay_unlinked_like_python[ambiguous]`
#[test]
fn test_bark_rows_refuse_or_stay_unlinked_like_python_ambiguous() {
    replay("wallet_sync_bark/test_bark_rows_refuse_or_stay_unlinked_like_python_ambiguous");
}

/// `tests/test_rust_wallet_sync_bark.py::test_bark_setup_and_inspection_like_python`
#[test]
fn test_bark_setup_and_inspection_like_python() {
    replay("wallet_sync_bark/test_bark_setup_and_inspection_like_python");
}

/// `tests/test_rust_wallet_sync_bark.py::test_lightning_settled_into_bark_is_booked_once_like_python[ledger-first]`
#[test]
fn test_lightning_settled_into_bark_is_booked_once_like_python_ledger_first() {
    replay(
        "wallet_sync_bark/test_lightning_settled_into_bark_is_booked_once_like_python_ledger_first",
    );
}

/// `tests/test_rust_wallet_sync_bark.py::test_the_bark_wallet_books_like_python`
#[test]
fn test_the_bark_wallet_books_like_python() {
    replay("wallet_sync_bark/test_the_bark_wallet_books_like_python");
}

/// `tests/test_rust_wallet_sync_btcpay.py::test_btcpay_provenance_enriches_a_descriptor_wallet_like_python[wallet]`
#[test]
fn test_btcpay_provenance_enriches_a_descriptor_wallet_like_python_wallet() {
    replay(
        "wallet_sync_btcpay/test_btcpay_provenance_enriches_a_descriptor_wallet_like_python_wallet",
    );
}

/// `tests/test_rust_wallet_sync_btcpay.py::test_btcpay_sources_and_enrichment_sync_like_python`
#[test]
fn test_btcpay_sources_and_enrichment_sync_like_python() {
    replay("wallet_sync_btcpay/test_btcpay_sources_and_enrichment_sync_like_python");
}

/// `tests/test_rust_wallet_sync_cli.py::test_sync_cli_capability_errors_are_not_handoff[False]`
#[test]
fn test_sync_cli_capability_errors_are_not_handoff_false() {
    replay("wallet_sync_cli/test_sync_cli_capability_errors_are_not_handoff_False");
}

/// `tests/test_rust_wallet_sync_cli.py::test_sync_cli_capability_errors_are_not_handoff[True]`
#[test]
fn test_sync_cli_capability_errors_are_not_handoff_true() {
    replay("wallet_sync_cli/test_sync_cli_capability_errors_are_not_handoff_True");
}

/// `tests/test_rust_wallet_sync_cli.py::test_sync_cli_explicit_unreadable_wallet_config_is_typed[bad]`
#[test]
fn test_sync_cli_explicit_unreadable_wallet_config_is_typed_bad() {
    replay("wallet_sync_cli/test_sync_cli_explicit_unreadable_wallet_config_is_typed_bad");
}

/// `tests/test_rust_wallet_sync_cli.py::test_sync_cli_fixture_books_match_python[receive-change-wallet-electrum]`
#[test]
fn test_sync_cli_fixture_books_match_python_receive_change_wallet_electrum() {
    replay(
        "wallet_sync_cli/test_sync_cli_fixture_books_match_python_receive_change_wallet_electrum",
    );
}

/// `tests/test_rust_wallet_sync_cli.py::test_sync_cli_handoff_requires_the_test_only_opt_in`
#[test]
fn test_sync_cli_handoff_requires_the_test_only_opt_in() {
    replay("wallet_sync_cli/test_sync_cli_handoff_requires_the_test_only_opt_in");
}

/// `tests/test_rust_wallet_sync_cli.py::test_sync_cli_missing_capability_implementation_fails_closed`
#[test]
fn test_sync_cli_missing_capability_implementation_fails_closed() {
    replay("wallet_sync_cli/test_sync_cli_missing_capability_implementation_fails_closed");
}

/// `tests/test_rust_wallet_sync_cli.py::test_sync_cli_native_failure_never_falls_back[no-egress-electrum]`
#[test]
fn test_sync_cli_native_failure_never_falls_back_no_egress_electrum() {
    replay("wallet_sync_cli/test_sync_cli_native_failure_never_falls_back_no_egress_electrum");
}

/// `tests/test_rust_wallet_sync_cli.py::test_sync_cli_native_failure_never_falls_back[no-egress-esplora]`
#[test]
fn test_sync_cli_native_failure_never_falls_back_no_egress_esplora() {
    replay("wallet_sync_cli/test_sync_cli_native_failure_never_falls_back_no_egress_esplora");
}

/// `tests/test_rust_wallet_sync_cli.py::test_sync_cli_native_failure_never_falls_back[offline-electrum]`
#[test]
fn test_sync_cli_native_failure_never_falls_back_offline_electrum() {
    replay("wallet_sync_cli/test_sync_cli_native_failure_never_falls_back_offline_electrum");
}

/// `tests/test_rust_wallet_sync_cli.py::test_sync_cli_native_failure_never_falls_back[offline-esplora]`
#[test]
fn test_sync_cli_native_failure_never_falls_back_offline_esplora() {
    replay("wallet_sync_cli/test_sync_cli_native_failure_never_falls_back_offline_esplora");
}

/// `tests/test_rust_wallet_sync_cli.py::test_sync_cli_probe_does_not_bootstrap`
#[test]
fn test_sync_cli_probe_does_not_bootstrap() {
    replay("wallet_sync_cli/test_sync_cli_probe_does_not_bootstrap");
}

/// `tests/test_rust_wallet_sync_cli.py::test_sync_cli_unsupported_capability_handoff_has_no_traffic[False]`
#[test]
fn test_sync_cli_unsupported_capability_handoff_has_no_traffic_false() {
    replay("wallet_sync_cli/test_sync_cli_unsupported_capability_handoff_has_no_traffic_False");
}

/// `tests/test_rust_wallet_sync_cli.py::test_sync_cli_unsupported_capability_handoff_has_no_traffic[True]`
#[test]
fn test_sync_cli_unsupported_capability_handoff_has_no_traffic_true() {
    replay("wallet_sync_cli/test_sync_cli_unsupported_capability_handoff_has_no_traffic_True");
}

/// `tests/test_rust_wallet_sync_cli.py::test_sync_cli_unsupported_hybrid_selection_falls_back_before_traffic`
#[test]
fn test_sync_cli_unsupported_hybrid_selection_falls_back_before_traffic() {
    replay("wallet_sync_cli/test_sync_cli_unsupported_hybrid_selection_falls_back_before_traffic");
}

/// `tests/test_rust_wallet_sync_gaps.py::test_address_list_rescan_after_apply_matches_python[esplora]`
#[test]
fn test_address_list_rescan_after_apply_matches_python_esplora() {
    replay("wallet_sync_gaps/test_address_list_rescan_after_apply_matches_python_esplora");
}

/// `tests/test_rust_wallet_sync_gaps.py::test_address_list_wallet[esplora]`
#[test]
fn test_address_list_wallet_esplora() {
    replay("wallet_sync_gaps/test_address_list_wallet_esplora");
}

/// `tests/test_rust_wallet_sync_gaps.py::test_descriptor_rescan_after_apply_matches_python[esplora]`
#[test]
fn test_descriptor_rescan_after_apply_matches_python_esplora() {
    replay("wallet_sync_gaps/test_descriptor_rescan_after_apply_matches_python_esplora");
}

/// `tests/test_rust_wallet_sync_gaps.py::test_inventory_owner_trims_a_descriptor[esplora]`
#[test]
fn test_inventory_owner_trims_a_descriptor_esplora() {
    replay("wallet_sync_gaps/test_inventory_owner_trims_a_descriptor_esplora");
}

/// `tests/test_rust_wallet_sync_gaps.py::test_negative_balance_repair_matches_python[electrum]`
#[test]
fn test_negative_balance_repair_matches_python_electrum() {
    replay("wallet_sync_gaps/test_negative_balance_repair_matches_python_electrum");
}

/// `tests/test_rust_wallet_sync_gaps.py::test_node_kind_without_a_backend_syncs_its_chain_material[lnd]`
#[test]
fn test_node_kind_without_a_backend_syncs_its_chain_material_lnd() {
    replay("wallet_sync_gaps/test_node_kind_without_a_backend_syncs_its_chain_material_lnd");
}

/// `tests/test_rust_wallet_sync_gaps.py::test_rescan_starts_from_the_published_checkpoint`
#[test]
fn test_rescan_starts_from_the_published_checkpoint() {
    replay("wallet_sync_gaps/test_rescan_starts_from_the_published_checkpoint");
}

/// `tests/test_rust_wallet_sync_gaps.py::test_samourai_child_matches_python[wpkh-pair-electrum]`
#[test]
fn test_samourai_child_matches_python_wpkh_pair_electrum() {
    replay("wallet_sync_gaps/test_samourai_child_matches_python_wpkh_pair_electrum");
}

/// `tests/test_rust_wallet_sync_gaps.py::test_two_descriptor_wallets_match_python[all-electrum]`
#[test]
fn test_two_descriptor_wallets_match_python_all_electrum() {
    replay("wallet_sync_gaps/test_two_descriptor_wallets_match_python_all_electrum");
}

/// `tests/test_rust_wallet_sync_lightning.py::test_core_lightning_method_failures_match_python[listinvoices]`
#[test]
fn test_core_lightning_method_failures_match_python_listinvoices() {
    replay_pinned(
        "wallet_sync_lightning/test_core_lightning_method_failures_match_python_listinvoices",
        CLN_SNAPSHOT_DAY,
        |_, _| {},
    );
}

/// `tests/test_rust_wallet_sync_lightning.py::test_core_lightning_wallet_sync_matches_python[wallet]`
#[test]
fn test_core_lightning_wallet_sync_matches_python_wallet() {
    replay_pinned(
        "wallet_sync_lightning/test_core_lightning_wallet_sync_matches_python_wallet",
        CLN_SNAPSHOT_DAY,
        |_, _| {},
    );
}

/// `tests/test_rust_wallet_sync_lightning.py::test_lnd_wallet_sync_matches_python[wallet]`
#[test]
fn test_lnd_wallet_sync_matches_python_wallet() {
    replay("wallet_sync_lightning/test_lnd_wallet_sync_matches_python_wallet");
}

/// `tests/test_rust_wallet_sync_silent_payments.py::test_core_rpc_scans_are_authoritative_only_when_complete[incomplete]`
#[test]
fn test_core_rpc_scans_are_authoritative_only_when_complete_incomplete() {
    replay("wallet_sync_silent_payments/test_core_rpc_scans_are_authoritative_only_when_complete_incomplete");
}

/// `tests/test_rust_wallet_sync_silent_payments.py::test_core_rpc_scans_are_authoritative_only_when_complete[untimestamped]`
#[test]
fn test_core_rpc_scans_are_authoritative_only_when_complete_untimestamped() {
    replay("wallet_sync_silent_payments/test_core_rpc_scans_are_authoritative_only_when_complete_untimestamped");
}

/// `tests/test_rust_wallet_sync_silent_payments.py::test_direct_wallet_sync_events_match_python[custom]`
#[test]
fn test_direct_wallet_sync_events_match_python_custom() {
    replay("wallet_sync_silent_payments/test_direct_wallet_sync_events_match_python_custom");
}

/// `tests/test_rust_wallet_sync_silent_payments.py::test_local_scanner_receive_spend_and_partial_match_python[custom]`
#[test]
fn test_local_scanner_receive_spend_and_partial_match_python_custom() {
    replay("wallet_sync_silent_payments/test_local_scanner_receive_spend_and_partial_match_python_custom");
}

/// `tests/test_rust_wallet_sync_silent_payments.py::test_local_scanner_receive_spend_and_partial_match_python[esplora]`
#[test]
fn test_local_scanner_receive_spend_and_partial_match_python_esplora() {
    replay("wallet_sync_silent_payments/test_local_scanner_receive_spend_and_partial_match_python_esplora");
}

/// `tests/test_rust_wallet_sync_silent_payments.py::test_offline_mode_matches_python[esplora]`
#[test]
fn test_offline_mode_matches_python_esplora() {
    replay("wallet_sync_silent_payments/test_offline_mode_matches_python_esplora");
}

/// `tests/test_rust_wallet_sync_silent_payments.py::test_scan_payload_shapes_match_python[amount]`
#[test]
fn test_scan_payload_shapes_match_python_amount() {
    replay("wallet_sync_silent_payments/test_scan_payload_shapes_match_python_amount");
}

/// `tests/test_rust_wallet_sync_silent_payments.py::test_scan_payload_shapes_match_python[range]`
#[test]
fn test_scan_payload_shapes_match_python_range() {
    replay("wallet_sync_silent_payments/test_scan_payload_shapes_match_python_range");
}

/// `tests/test_rust_wallet_sync_silent_payments.py::test_scan_payload_shapes_match_python[spend-only]`
#[test]
fn test_scan_payload_shapes_match_python_spend_only() {
    replay("wallet_sync_silent_payments/test_scan_payload_shapes_match_python_spend_only");
}

/// `tests/test_rust_wallet_sync_silent_payments.py::test_scanner_failures_match_python[mismatch]`
#[test]
fn test_scanner_failures_match_python_mismatch() {
    replay("wallet_sync_silent_payments/test_scanner_failures_match_python_mismatch");
}

/// `tests/test_rust_wallet_sync_silent_payments.py::test_scanner_failures_match_python[mode]`
#[test]
fn test_scanner_failures_match_python_mode() {
    replay("wallet_sync_silent_payments/test_scanner_failures_match_python_mode");
}

/// `tests/test_rust_wallet_sync_silent_payments.py::test_scanner_failures_match_python[symlink]`
#[test]
fn test_scanner_failures_match_python_symlink() {
    replay("wallet_sync_silent_payments/test_scanner_failures_match_python_symlink");
}

/// `tests/test_rust_wallet_sync_silent_payments.py::test_server_assisted_scan_matches_python[complete]`
#[test]
fn test_server_assisted_scan_matches_python_complete() {
    replay("wallet_sync_silent_payments/test_server_assisted_scan_matches_python_complete");
}
