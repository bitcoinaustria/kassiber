//! The freshness queue, source admission, job execution, native sync and
//! graph follow-up, replayed from the parity tests that compared the
//! fixtures (`examples/freshness_fixture.rs`, `freshness_sync_fixture.rs`,
//! `graph_fixture.rs`) with the Python reference
//! (`tests/test_rust_freshness_{queue,admission,execution,native,graph}.py`).
//!
//! Each Rust test carries its pytest test's name and replays every recorded
//! parameter of it: the same input against the same starting book, through
//! the fixture's own code, must print the identical line and leave the
//! identical book. Fixtures that reached a loopback chain run in a child
//! process of this binary on the recorded network tape; tests that changed
//! the book while the chain held a request hold the taped reply and make the
//! same change. The recordings (`tests/data/freshness/`) were made once
//! where Rust and Python agreed; see `golden.rs`.

#[path = "../../examples/freshness_fixture.rs"]
#[allow(dead_code)]
mod freshness_fixture;
#[path = "../../examples/freshness_sync_fixture.rs"]
#[allow(dead_code)]
mod freshness_sync_fixture;
mod golden;
#[path = "../../examples/graph_fixture.rs"]
#[allow(dead_code)]
mod graph_fixture;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use golden::{
    assert_same_book, calls, fixed_elapsed_ms, label, restore, snapshot, string, value, Golden,
    Scratch,
};
use kassiber_envelope::{get, py_str};
use kassiber_pyjson::Value;

/// The fixture ran one invocation per process; admission's execution slot
/// is process-wide, so in-process replays take turns.
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Set in the process a taped replay runs its fixture in.
const CHILD: &str = "KASSIBER_FRESHNESS_REPLAY_CHILD";

/// A response the recording's loopback chain held while the test acted:
/// the first request containing `pattern` waits until `act` has run on the
/// book.
pub struct Hold {
    pub pattern: &'static str,
    pub act: Box<dyn Fn(&Path)>,
}

/// Not a test of its own: the process a taped replay runs its fixture in.
/// Each process reads one network tape (`kassiber-net`'s test tape), so a
/// replay that connects gets a fresh process of this test binary.
#[test]
fn replay_child() {
    let Some(dir) = std::env::var_os(CHILD).map(PathBuf::from) else {
        return;
    };
    let fixture = std::fs::read_to_string(dir.join("fixture")).unwrap();
    let input = std::fs::read_to_string(dir.join("input")).unwrap();
    let printed = match fixture.as_str() {
        "freshness_sync_fixture" => freshness_sync_fixture::run(&input),
        "graph_fixture" => graph_fixture::run(&input),
        other => panic!("no taped fixture {other}"),
    };
    std::fs::write(dir.join("stdout"), printed).unwrap();
}

/// Run one taped call in a child process, acting on `hold` when it fires.
fn run_taped(
    golden: &Golden,
    scratch: &Scratch,
    index: usize,
    call: &Value,
    book: &Path,
    hold: Option<&Hold>,
) -> String {
    let dir = scratch.0.join(format!("call-{index}"));
    std::fs::create_dir_all(dir.join("hold")).unwrap();
    std::fs::write(dir.join("fixture"), string(call, "fixture")).unwrap();
    std::fs::write(dir.join("input"), scratch.place(&string(call, "input"))).unwrap();
    std::fs::write(
        dir.join("net.tape"),
        py_str(golden.resolve(value(call, "tape"))),
    )
    .unwrap();
    if let Some(hold) = hold {
        std::fs::write(dir.join("hold/arm"), hold.pattern).unwrap();
    }
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args(["--exact", "replay_child", "--test-threads=1", "--quiet"]);
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("KASSIBER_") {
            command.env_remove(key);
        }
    }
    command
        .env(CHILD, &dir)
        .env(
            "KASSIBER_TEST_NET_TAPE",
            format!("replay:{}", dir.join("net.tape").display()),
        )
        .env("KASSIBER_TEST_NET_TAPE_HOLD", dir.join("hold"))
        .env("KASSIBER_TEST_NET_TAPE_WRITES", dir.join("net.writes"))
        // Never the account's app-data root or its offline switch.
        .env("XDG_DATA_HOME", scratch.0.join("xdg-data"))
        .env(
            "KASSIBER_OFFLINE_PREFERENCE_FILE",
            scratch.0.join("offline.json"),
        )
        .env("TMPDIR", &scratch.0)
        .env("KASSIBER_FRESHNESS_FIXTURE_BUDGET_SECS", "900");
    for (key, item) in golden::entries(value(call, "env")) {
        command.env(key, scratch.place(&py_str(item)));
    }
    let mut child = command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::fs::File::create(dir.join("stderr")).unwrap())
        .spawn()
        .unwrap();
    if let Some(hold) = hold {
        let deadline = Instant::now() + Duration::from_secs(60);
        while !dir.join("hold/held").exists() {
            assert!(Instant::now() < deadline, "the held request never came");
            if let Some(status) = child.try_wait().unwrap() {
                panic!("the fixture exited ({status}) before the held request");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        (hold.act)(book);
        std::fs::write(dir.join("hold/release"), b"").unwrap();
    }
    let status = child.wait().unwrap();
    assert!(
        status.success(),
        "the taped fixture failed: {status}\n{}",
        std::fs::read_to_string(dir.join("stderr")).unwrap_or_default()
    );
    // No request the recording's chain did not receive (the pytest tests
    // counted its hits).
    let recorded = std::fs::read_to_string(dir.join("net.tape"))
        .unwrap()
        .lines()
        .filter(|line| line.starts_with("w\t"))
        .count();
    let replayed = std::fs::read_to_string(dir.join("net.writes"))
        .unwrap_or_default()
        .lines()
        .count();
    assert!(
        replayed <= recorded,
        "{replayed} requests, recorded {recorded}"
    );
    std::fs::read_to_string(dir.join("stdout")).unwrap()
}

/// Replay every recorded case of `test` from the golden `module`.
fn replay(module: &str, test: &str) {
    replay_held(module, test, |_| None);
}

/// `replay`, with a hold for the cases `hold` names (by parameter id).
fn replay_held(module: &str, test: &str, hold: impl Fn(&str) -> Option<Hold>) {
    let golden = Golden::load(module);
    for case in golden.cases(test) {
        let name = label(case);
        let scratch = Scratch::new(&name);
        let held = hold(&string(case, "param"));
        for (index, call) in calls(case).iter().enumerate() {
            let input = scratch.place(&string(call, "input"));
            let parsed = kassiber_pytext::loads(&input).unwrap();
            let book = PathBuf::from(py_str(get(&parsed, "book").unwrap()));
            let _ = std::fs::remove_file(&book);
            restore(&golden, value(call, "before"), &book);
            let printed = if matches!(value(call, "tape"), Value::Null) {
                assert_eq!(string(call, "fixture"), "freshness_fixture", "{name}");
                let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
                freshness_fixture::run(&input)
            } else {
                run_taped(
                    &golden,
                    &scratch,
                    index,
                    call,
                    &book,
                    if index == 0 { held.as_ref() } else { None },
                )
            };
            assert_eq!(
                fixed_elapsed_ms(&printed),
                fixed_elapsed_ms(&scratch.place(&string(call, "stdout"))),
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

// tests/test_rust_freshness_native.py: the native sync fixture on a taped chain
#[test]
fn test_native_freshness_full_book_execution_matches_python() {
    replay(
        "native",
        "test_native_freshness_full_book_execution_matches_python",
    );
}

#[test]
fn test_native_freshness_several_wallets_match_python() {
    replay(
        "native",
        "test_native_freshness_several_wallets_match_python",
    );
}

#[test]
fn test_native_freshness_negative_balance_repair_matches_python() {
    replay(
        "native",
        "test_native_freshness_negative_balance_repair_matches_python",
    );
}

#[test]
fn test_native_freshness_electrum_address_list_matches_python() {
    replay(
        "native",
        "test_native_freshness_electrum_address_list_matches_python",
    );
}

#[test]
fn test_native_freshness_rescan_after_apply_matches_python() {
    replay(
        "native",
        "test_native_freshness_rescan_after_apply_matches_python",
    );
}

#[test]
fn test_native_freshness_forced_reviewed_restart_matches_python() {
    replay(
        "native",
        "test_native_freshness_forced_reviewed_restart_matches_python",
    );
}

#[test]
fn test_native_freshness_pretransport_fences() {
    replay("native", "test_native_freshness_pretransport_fences");
}

#[test]
fn test_native_freshness_revalidates_before_queue_and_transport() {
    replay(
        "native",
        "test_native_freshness_revalidates_before_queue_and_transport",
    );
}

#[test]
fn test_native_freshness_progress_authority_fences() {
    replay("native", "test_native_freshness_progress_authority_fences");
}

/// The history fetch a recorded chain held, for a case's protocol.
fn fetch(param: &str) -> &'static str {
    if param.contains("electrum") {
        "blockchain.scripthash.get_history"
    } else {
        "/scripthash/"
    }
}

/// `sql` on the book, as the pytest chain ran it at the held request.
fn on_book(sql: &'static str) -> impl Fn(&Path) {
    move |book: &Path| {
        kassiber_store::rusqlite::Connection::open(book)
            .unwrap()
            .execute_batch(sql)
            .unwrap()
    }
}

#[test]
fn test_native_freshness_controls_between_fetch_batches() {
    // The chain changed the book at the first history fetch; no later batch
    // or publication may start (the recorded tape ends there).
    replay_held(
        "native",
        "test_native_freshness_controls_between_fetch_batches",
        |param| {
            let sql = if param.contains("-cancel-") {
                "UPDATE freshness_jobs SET cancel_requested=1 WHERE status IN ('queued','running')"
            } else if param.contains("-book-") {
                "UPDATE settings SET value='' WHERE key='context_profile'"
            } else if param.contains("-config-") {
                "UPDATE wallets SET config_json='{}'"
            } else {
                "UPDATE backends SET url='http://127.0.0.1:1' WHERE name='fixture'"
            };
            Some(Hold {
                pattern: fetch(param),
                act: Box::new(on_book(sql)),
            })
        },
    );
}

#[test]
fn test_reference_prefetch_cancellation_repro_and_native_terminal_parity() {
    replay_held(
        "native",
        "test_reference_prefetch_cancellation_repro_and_native_terminal_parity",
        |param| {
            Some(Hold {
                pattern: fetch(param),
                act: Box::new(on_book(
                    "UPDATE freshness_jobs SET cancel_requested=1 WHERE status='queued'",
                )),
            })
        },
    );
}

// tests/test_rust_freshness_graph.py: the graph follow-up fixture on a taped chain
#[test]
fn test_native_graph_reference_full_book_parity() {
    replay("graph", "test_native_graph_reference_full_book_parity");
}

#[test]
fn test_native_graph_core_rpc_parity() {
    replay("graph", "test_native_graph_core_rpc_parity");
}

#[test]
fn test_native_graph_transaction_limit_cursor_and_cache_parity() {
    replay(
        "graph",
        "test_native_graph_transaction_limit_cursor_and_cache_parity",
    );
}

#[test]
fn test_native_graph_sanitizes_legacy_reference_fields() {
    replay(
        "graph",
        "test_native_graph_sanitizes_legacy_reference_fields",
    );
}

#[test]
fn test_native_graph_shared_prevout_budget_leaves_refused_row_first() {
    replay(
        "graph",
        "test_native_graph_shared_prevout_budget_leaves_refused_row_first",
    );
}

#[test]
fn test_native_graph_refuses_transaction_over_prevout_limit() {
    replay(
        "graph",
        "test_native_graph_refuses_transaction_over_prevout_limit",
    );
}

#[test]
fn test_native_graph_row_limit_and_private_cursor_parity() {
    replay(
        "graph",
        "test_native_graph_row_limit_and_private_cursor_parity",
    );
}

#[test]
fn test_native_graph_local_inspection_limit_fails_closed() {
    replay(
        "graph",
        "test_native_graph_local_inspection_limit_fails_closed",
    );
}

#[test]
fn test_electrum_mismatched_transaction_is_refused_like_python() {
    replay(
        "graph",
        "test_electrum_mismatched_transaction_is_refused_like_python",
    );
}

#[test]
fn test_native_graph_failures_advance_cursor_without_backend_fallback() {
    replay(
        "graph",
        "test_native_graph_failures_advance_cursor_without_backend_fallback",
    );
}

#[test]
fn test_native_graph_local_scope_and_cache_guards() {
    replay("graph", "test_native_graph_local_scope_and_cache_guards");
}

#[test]
fn test_native_graph_pretransport_fences_are_inert() {
    replay("graph", "test_native_graph_pretransport_fences_are_inert");
}
