"""Hand-reviewed dispositions for tests/rust_port/dispositions.json."""

# Samples include setup/helper paths and the first, middle and last tests;
# large mixed classes were inspected by method. Notes describe what the
# heuristic alone cannot decide. Sampling is not a claim of exhaustive review.
SAMPLES = {
    "tests/test_rust_store_migrations.py": "The opt-in Rust storage probe is compared with Python on deterministic disposable books, including complete typed/internal-object dumps, historical fixtures, journal coexistence and real killed-child recovery. Direct SQL/helper setup is explicit scenario input, not a portable CLI recording; retain the differential oracle and its dump-sensitivity check.",
    "tests/test_review_regressions.py": "CLI subprocesses, direct handlers, SQL mutation and embedded ledger snapshots coexist; keep per-method splits and retain snapshot comparisons as oracles.",
    "tests/test_daemon_smoke.py": "Subprocess JSONL sessions coexist with thread/consent unit tests and patched handle_request calls; do not label the whole module portable.",
    "tests/test_sync_backends.py": "Adapter parsing, SOCKS fakes, retraction SQL and atomic sync use direct services; preserve Rust transport and store tests rather than record fake Python sockets.",
    "tests/test_cli_smoke.py": "CLI helpers run real children; several methods directly mutate SQLite and need scenarios, while ordinary CSV imports can be boundary fixtures.",
    "tests/test_transaction_graph.py": "Graph cache SQL, backend permission and local-observation preference are service tests; public lookup requires transport fixtures in the owning crate.",
    "tests/test_source_funds_cli.py": "Real CLI helper, but _db and _tx_id helpers include direct SQLite edits; split those methods from CLI-only evidence sessions.",
    "tests/test_operator_service.py": "Mocked connection/runner and real worker threads exercise queue expiry and grants in the service; CLI argument parsing is separately portable Rust logic.",
    "tests/test_rp2_ownership_transfers.py": "Expected engine assertions are production tests, not comparisons with RP2; direct journal-handler persistence belongs in Rust service/store tests.",
    "tests/test_freshness.py": "Synthetic jobs, patched rate providers and worker ownership are service tests; source docstring assertion disappears, event-envelope behavior remains.",
    "tests/test_ai_core.py": "SSE parsing, request shape and provider CRUD are service units; patched daemon entry cases need subprocess scenarios with provider tapes.",
    "tests/test_custody_quantity_runtime.py": "Pure exact-quantity construction and raw in-memory SQL projection checks remain Rust tests; Python signature inspection becomes a typed API check.",
    "tests/test_custody_components.py": "Migration schema cases belong to store; activation/conservation and rollback cases belong to custody, even when one assertion launches the CLI.",
    "tests/test_ownership_transfers.py": "Synthetic row/index derivation, ambiguity and conservation properties port directly; no actual CLI or RP2 oracle is involved.",
    "tests/test_transfer_matching.py": "Pure fee, scope and evidence matchers port to Rust; optimized-window versus independent naive matcher comparison stays an oracle.",
    "tests/test_rates_kraken_csv.py": "CLI acquisition is mixed with direct SQL and patched pricing/ledger functions; bulk artifact/resource tests are service units, not all portable CLI recordings.",
    "tests/test_ai_core_http_transport.py": "Scripted HTTP server is a wire fixture, but Python clients run in process; rewrite provider scenarios against daemon chat, retain parser units.",
    "tests/test_tax_ledger_prepare.py": "Production stages are compared with the frozen oracle over goldens/random/NaN/Infinity edge books; retain all as oracle coverage.",
    "tests/test_custody_gaps.py": "Pure bounded search and seeded snapshot pagination coexist with schema-removal checks; store owns the latter, custody owns matcher capacity properties.",
    "tests/test_channel_lifecycle.py": "Synthetic channel histories exercise ownership and fee conservation in the journal builder; production expectations alone do not make an RP2 oracle.",
    "tests/test_cli_chat.py": "Real CLI and fake HTTP wire sessions coexist with mocked TTY/render/client tests; private provider seeding needs scenarios, rendering is a Rust CLI unit.",
    "tests/test_audit_package.py": "Direct summary/export services seeded with attachments and filed history; readiness and redaction are report crate tests, not ready-made wire sessions.",
    "tests/test_custody_component_surfaces.py": "dispatch/_ui_swap_matching_payload and daemon children all use Python _fixture SQL setup; explicit scenario replacement is required.",
    "tests/test_privacy_linkage.py": "In-memory graph/PSBT scoring and adversarial missing evidence are chain service properties; no graph edge establishes tax authority.",
    "tests/test_secrets_smoke.py": "SQLCipher, tar-member safety, credential migration and archive round trips are service units; preserve plaintext/encrypted database equivalence and rollback.",
    "tests/test_custody_gap_lifecycle.py": "Seeded SQL plus fingerprint/revision/rollback service tests; dispatch and daemon preview cases are scenarios, not portable fixtures.",
    "tests/test_tax_events.py": "Synthetic finalized projections exercise gates and descriptions; keep production assertions in Rust, not automatically oracle just because of tax naming.",
    "tests/test_ownership.py": "Classification, csv recovery, script scope and privacy redaction are pure/service units; injected verification fetchers must remain explicit test transports.",
    "tests/test_schema_swap_matching.py": "Historical schema downgrades, FK/uniqueness checks and reopens must port to store migration tests; fresh reviewed component behavior remains covered.",
    "tests/test_ai_core_api_format.py": "Provider storage and HTTP client fallback are units; daemon test_connection is in process; two real CLI classes use only external scripted provider setup.",
    "tests/test_custody_authored_migration.py": "Legacy SQL triggers/backfills and partial authored components test store compatibility; Rust must open historical fixtures rather than discard Python seeding intent.",
}

REVIEWS = {
    "tests/test_custody_channel_partial.py": {"*": {"disposition": "oracle", "group": "custody", "reason": "compares raw channel lifecycle and persisted loan roles with Python, freezes helper branches, and pins exact per-scenario engine counts plus authoring direction rules"}},
    "tests/test_custody_safety_partial.py": {"*": {"disposition": "oracle", "group": "custody", "reason": "compares raw holds, privacy boundaries and dependency closure against Python with frozen helper witnesses, full-interpreter controls and per-scenario producer counts"}},
    "tests/test_backend_configuration.py": {
        "test_malformed_backend_locations_never_echo_embedded_credentials": {"disposition": "rust-test", "group": "chain-sync", "reason": "calls Python backend redactors directly; Rust replacement is malformed_backend_locations_never_echo_embedded_credentials in crates/setup/tests/backends.rs"},
        "test_bare_backend_hosts_stay_visible_when_they_cannot_hold_credentials": {"disposition": "rust-test", "group": "chain-sync", "reason": "pins the Python bare-host disclosure rule; Rust replacement is bare_backend_location_fullmatch_preserves_only_the_python_allowlist in crates/setup/src/backend_output.rs"},
    },
    "tests/test_transaction_desktop_validation.py": {"*": {"disposition": "rust-test", "group": "transactions", "reason": "pins typed sort/order container refusals; replace with the desktop service validation regressions"}},
    "tests/test_rust_store_migrations.py": {"*": {"disposition": "oracle", "group": "store", "reason": "retains Python/Rust differential storage migration, exact logical-dump and interruption-recovery comparisons"}},
    "tests/test_rust_store_partition_reference.py": {"*": {"disposition": "oracle", "group": "store", "reason": "pins legacy-project reference behavior and compares both runtimes across publication and killed-child recovery"}},
    "tests/test_rust_store_interop.py": {"*": {"group": "store", "reason": "native storage admission, codec, ownership, path and unlock interoperability belongs to the shared storage boundary"}},
    "tests/test_custody_generated_books.py": {"*": {"disposition": "oracle", "group": "custody", "reason": "compares seeded raw snapshots and order permutations across Python and Rust while retaining the independent rate-order behavior pin"}},
    "tests/test_custody_interpretation_partial.py": {"*": {"disposition": "oracle", "group": "custody", "reason": "compares isolated raw-fact interpreter slices with Python, freezes active/blocked scenarios and text semantics, and proves caught partial differences still fail the comparison session"}},
    "tests/test_custody_engine_snapshot.py": {"*": {"disposition": "oracle", "group": "custody", "reason": "pins the raw snapshot privacy boundary and compares connection-free replay with the frozen SQL flow, including excluded/retracted anchors"}},
    "tests/test_custody_shadow_harness.py": {"*": {"disposition": "oracle", "group": "custody", "reason": "proves custody discrepancies fail subprocesses and sessions that catch the error"}},

    "tests/test_tax_ledger_prepare.py": {"*": {"disposition": "oracle", "reason": "compares production preparation stages with the frozen oracle over goldens and seeded edge books"}},
    "tests/test_tax_ledger_contract.py": {"*": {"disposition": "oracle", "reason": "pins the retained ledger comparison and fixture contract"}},
    "tests/test_tax_shadow_harness.py": {"*": {"disposition": "oracle", "reason": "proves implementation comparison fails on injected accounting differences"}},
    "tests/test_tax_shadow_capabilities.py": {"*": {"disposition": "oracle", "reason": "pins the oracle comparison capability contract"}},
    "tests/test_tax_oracle_imports.py": {"*": {"disposition": "python-only", "reason": "Python import isolation machinery disappears with the interpreter", "replacement": "Cargo dependency boundaries and tests proving the shipping binary has no oracle dependency"}},
    "tests/test_pyinstaller_entry.py": {"*": {"disposition": "python-only", "group": "packaging-ci", "reason": "PyInstaller CA bootstrap disappears with the Python launcher", "replacement": "Rust TLS trust-selection and packaged host/explicit/bundled CA tests"}},
    "tests/test_no_egress_guard.py": {"*": {"disposition": "python-only", "group": "network-egress", "reason": "Python socket monkeypatch and sitecustomize guard installation disappear", "replacement": "Rust no-egress guard with child-process discrepancy injection and transport coverage"}},
    "tests/test_rust_shell_parity.py": {"*": {"disposition": "oracle", "group": "cli-shell", "reason": "compares the Rust binary's CLI and daemon protocol with the Python reference while Python remains"}},
    "tests/test_rust_rates_candidate.py": {"*": {"disposition": "oracle", "group": "chain-sync", "reason": "runs rates CLI leaves through the hybrid candidate with served-by assertions and compares native daemon JSONL bytes against Python"}},
    "tests/test_rust_setup_desktop.py": {"*": {"disposition": "oracle", "group": "cli-shell", "reason": "compares hybrid Rust setup daemon responses byte for byte with Python on disposable plaintext/encrypted books and Core configuration"}},
    "tests/test_ci_shards.py": {"*": {"disposition": "python-only", "group": "packaging-ci", "reason": "pytest lane and Python tax-shadow inventory retire with Python", "replacement": "Cargo/native scenario inventory with disjoint CI lanes and oracle overlay validation"}},
    "tests/test_dependency_drift.py": {"*": {"disposition": "python-only", "group": "packaging-ci", "reason": "Python dependency/uv metadata checks retire with Python", "replacement": "Cargo.lock, license and desktop dependency drift validation"}},
    "tests/test_desktop_packaging.py": {"DesktopPackagingTest::test_cli_release_and_desktop_sidecar_share_one_argument_set": {"disposition": "python-only", "reason": "pins shared PyInstaller arguments and onefile/onedir packaging", "replacement": "one Rust binary/artifact identity across CLI and desktop packaging"}},
    "tests/test_tax_engine_backend.py": {
        "ProductionIsolationTest::test_runs_journals_without_rp2_installed": {"disposition": "python-only", "reason": "Python import finder blocks RP2 during an interpreter child run", "replacement": "Cargo dependency isolation and native binary smoke without Python/RP2"},
        "BindingRoundTripTest::test_the_binding_exposes_ledger_and_custody_operations": {"disposition": "python-only", "reason": "checks the retiring PyO3 module export surface", "replacement": "native crate/binary schema API contract checks"},
        "BindingRoundTripTest::test_rust_panic_surfaces_as_runtime_error": {"disposition": "python-only", "reason": "checks PyO3 panic translation to a Python RuntimeError", "replacement": "native panic containment and fail-closed error-envelope tests"},
        "ErrorClassTest": {"disposition": "python-only", "reason": "pins Python exception-class translation", "replacement": "Rust error variants and stable boundary codes/retry flags"},
    },
    "tests/test_custody_components.py": {"CustodySchemaTests": {"group": "store", "reason": "historical SQLite compatibility and immutable-trigger migrations port to store tests"}},
    "tests/test_custody_gaps.py": {"CustodyGapLegacySchemaMigrationTests": {"group": "store", "reason": "checks historical database migration rather than candidate matching"}},
    "tests/test_custody_authored_migration.py": {"*": {"group": "store", "reason": "legacy SQL backfill and trigger invariants port to Rust store migration tests"}},
    "tests/test_custody_quantity_runtime.py": {"CustodyQuantityRuntimeTests::test_gap_search_projection_is_a_required_runtime_input": {"disposition": "python-only", "reason": "inspects a Python keyword-only signature", "replacement": "Rust API requires the gap-search input in its type signature"}},
    "tests/test_freshness.py": {"FreshnessTest::test_module_docstring_is_visible_to_ast": {"disposition": "python-only", "reason": "checks Python module docstring placement for AST tooling", "replacement": "Rust module documentation and native inventory checks"}},
    "tests/test_transfer_matching.py": {"HeuristicMatchTests::test_window_slice_matches_naive_all_pairs_reference": {"disposition": "oracle", "reason": "compares optimized window matching with an independent naive all-pairs reference"}},
    "tests/rust_port/test_inventory.py": {"*": {"disposition": "oracle", "group": "packaging-ci", "reason": "pins the migration disposition inventory and its completeness barriers", "flags": []}},
    "tests/rust_port/test_contracts.py": {"*": {"disposition": "oracle", "group": "packaging-ci", "reason": "pins generated Python boundary/source contracts for the Rust replacement"}},
    "tests/test_cli_session_replay.py": {"*": {"disposition": "oracle", "group": "cli-shell", "reason": "records Python sessions and proves replay against a candidate fails on injected differences while Python remains the reference"}},
    "tests/test_cli_runner.py": {"*": {"disposition": "oracle", "group": "cli-shell", "reason": "pins the one switch that points every product subprocess at the Rust candidate during the port"}},
    "tests/test_rust_bip329_services.py": {"*": {"disposition": "oracle", "group": "transactions", "reason": "compares Rust CLI/daemon envelopes, persisted label/audit state and exact BIP329 file bytes against Python on disposable books"}},
    "tests/test_rust_document_services.py": {"*": {"disposition": "oracle", "group": "transactions", "reason": "compares document CLI and daemon refusals, evidence storage and prior-report archives, stored matching and commercial context against Python on disposable books"}},
    "tests/test_rust_transaction_desktop.py": {"*": {"disposition": "oracle", "group": "transactions", "reason": "compares desktop list/search/extremes raw daemon JSONL, all filters, grouped totals, queue scope, cursor pages and typed refusals with Python on disposable books"}},
    "tests/test_rust_transaction_listing.py": {"*": {"disposition": "oracle", "group": "transactions", "reason": "compares observed Rust CLI pages, all filters, sort and cursor bytes, exact msat values and fail-closed export/desktop barriers with Python on disposable books"}},
    "tests/test_rust_grouped_metadata_services.py": {"*": {"disposition": "oracle", "group": "transactions", "reason": "compares grouped Rust desktop saves, typed validation, exact price strings, append-only audit fields and version transitions against Python on disposable books"}},
    "tests/test_rust_hybrid_candidate.py": {"*": {"disposition": "oracle", "group": "cli-shell", "reason": "pins the hybrid candidate that routes each CLI call to Rust or the Python reference during the port"}},
}

# R5 wallet lifecycle replacements. Sync and UI snapshot tests retain their
# existing dispositions until those services are ported.
REVIEWS["tests/test_wallet_config_multi_script.py"] = {
    "ParseWalletConfigMultiScriptTests": {
        "disposition": "rust-test", "group": "chain-sync",
        "reason": "CLI wallet-config intake has native xpub-set, descriptor, BSMS, stdin and exclusive-input tests; FD reading also uses the existing kassiber-secrets tests",
        "replacement": "crates/cli/src/wallets.rs::cli_config_ports_public_xpub_sets_bsms_and_safe_stdin; cli_config_requires_exclusive_stdin_and_onchain_btcpay_creation",
    },
    "ValidatedWalletConfigTests": {
        "disposition": "rust-test", "group": "chain-sync",
        "reason": "native frozen config vectors pin inferred network, required material, bounded scan depth and redacted output",
        "replacement": "crates/wallets/src/config.rs::python_wallet_config_vectors",
    },
    "WalletConfigFreshnessTests": {
        "disposition": "rust-test", "group": "chain-sync",
        "reason": "native lifecycle tests pin checkpoint invalidation, alias identity, observer retention and gap/scan-depth policy identity",
        "replacement": "crates/wallets/src/lifecycle/tests.rs::config_changes_reset_freshness_but_deprecation_and_scan_depth_preserve_checkpoint; alias_rewrites_preserve_observer_state_but_policy_changes_discard_it; scan_depth_gap_aliases_and_script_order_do_not_rotate_policy",
    },
    "WatchOnlyWalletConfigTests::test_create_and_update_reject_private_spending_material": {
        "disposition": "rust-test", "group": "chain-sync",
        "reason": "native lifecycle refuses spending keys before row or epoch writes",
        "replacement": "crates/wallets/src/lifecycle/tests.rs::spending_private_create_and_persisted_update_fail_without_writes",
    },
    "WatchOnlyWalletConfigTests::test_change_only_private_material_is_refused_before_storage": {
        "disposition": "rust-test", "group": "chain-sync",
        "reason": "native lifecycle pins the inactive-change reference regression on create and update",
        "replacement": "crates/wallets/src/lifecycle/tests.rs::change_only_private_and_unsafe_legacy_reads_fail_closed",
    },
}
REVIEWS["tests/test_ownership_policy_epochs.py"] = {
    "OwnershipPolicyEpochTest::test_comparison_identity_canonicalizes_aliases_without_rewriting_material": {
        "disposition": "rust-test", "group": "chain-sync",
        "reason": "native lifecycle preserves authored epoch material across equivalent chain/network/script-set rewrites",
        "replacement": "crates/wallets/src/lifecycle/tests.rs::alias_rewrites_preserve_observer_state_but_policy_changes_discard_it",
    },
    "OwnershipPolicyEpochTest::test_rollover_preserves_private_material_and_final_coverage": {
        "disposition": "rust-test", "group": "chain-sync",
        "reason": "native lifecycle retains retired policy material and leaves existing epoch coverage attached; observer coverage acquisition remains outside lifecycle",
        "replacement": "crates/wallets/src/lifecycle/tests.rs::policy_rotation_retains_material_and_is_atomic; policy_rotation_preserves_existing_coverage (UI technical-coverage snapshot remains unported)",
    },
}
