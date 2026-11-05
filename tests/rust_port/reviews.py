"""Hand-reviewed dispositions for tests/rust_port/dispositions.json."""

# Samples include setup/helper paths and the first, middle and last tests;
# large mixed classes were inspected by method. Notes describe what the
# heuristic alone cannot decide. Sampling is not a claim of exhaustive review.
SAMPLES = {
    "tests/test_rust_shell_parity.py": "Raw CLI bytes, complete command metadata and daemon records compare Python with Rust, including injected formatting discrepancies. Real-book setup, exact typed state, encryption, ownership exclusion and killed migration cases use disposable roots and explicitly seeded SQL/identities; retain this cross-runtime oracle rather than treating its helper setup as portable CLI fixtures.",
    "tests/test_backup_recovery.py": "Killed Python installer regression, every observed install I/O boundary, and bounded recovery-intent admission; port the storage transaction and cross-runtime killed-child matrix to Rust.",
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
    "tests/test_twentyonebitcoin_import.py": {
        "TwentyOneBitcoinLegacyReadinessTest": {
            "disposition": "rust-test",
            "group": "imports",
            "reason": "Direct coordinator regression against seeded legacy flags; native replacement is twentyone_legacy_reactivation_invalidates_ready_journals_once in crates/wallets/src/file_import/tests.rs, with full-book CLI parity in test_twentyone_unpriced_legacy_reactivation_matches_readiness_dump.",
        },
    },
    "tests/test_rust_freshness_product.py": {"*": {"disposition": "oracle", "group": "daemon-runtime", "reason": "compares registered no-refresh product envelopes, event sequences and every logical table with Python across disposable Electrum/Esplora fixture, restart and reviewed books; pins per-invocation hybrid decisions, zero served-shape fallbacks, argument validation and inert/busy selection"}},
    "tests/test_rust_freshness_journals.py": {"*": {"disposition": "oracle", "group": "daemon-runtime", "reason": "compares default journal-refresh product events, envelopes and every logical table with Python on disposable loopback books, including pairing authority, policy, workspace order and reviewed restarts; records native refusals and restricts development replay to proven rollback"}},
    "tests/test_rust_freshness_graph.py": {
        "*": {"disposition": "rust-test", "group": "daemon-runtime", "reason": "drives the guarded connection-owning graph worker on disposable loopback books to pin grant, request, lifecycle, output and SQLCipher fences and unsolicited redacted events; preserves subprocess witnesses without claiming automatic startup or product registration"},
        "test_reference_electrum_mismatched_transaction_repro_and_native_refusal": {"disposition": "oracle", "group": "daemon-runtime", "reason": "reproduces Python caching different Electrum transaction bytes under the requested ID and pins native computed-ID refusal; retains the reference bug without changing Python or claiming product parity"},
    },
    "tests/test_rust_freshness_lifecycle.py": {
        "*": {"disposition": "rust-test", "group": "daemon-runtime", "reason": "drives the guarded native request loop against blocked disposable backends to pin cancellation, source controls, quiescence, exact-request authority and private after-response graph-grant handoff/discard; retains subprocess and complete-book witnesses until product registration, without claiming graph-worker execution"},
        "test_reference_source_control_checkpoint_repro_and_native_parity": {"disposition": "oracle", "group": "daemon-runtime", "reason": "reproduces Python pause/resume checkpoint disclosure; compares complete source-control envelopes after explicitly removing that private result container, and every unchanged logical table"},
    },
    "tests/test_rust_analysis_features.py": {"*": {"disposition": "oracle", "group": "chain-sync", "reason": "compares unchanged Python public-feature extraction, signature categories, cache closure and evidence-bound rules with a native JSON-only driver; retains byte comparisons and complete 112-table read invariants without product routing"}},
    "tests/test_backend_configuration.py": {
        "test_malformed_backend_locations_never_echo_embedded_credentials": {"disposition": "rust-test", "group": "chain-sync", "reason": "calls Python backend redactors directly; Rust replacement is malformed_backend_locations_never_echo_embedded_credentials in crates/setup/tests/backends.rs"},
        "test_bare_backend_hosts_stay_visible_when_they_cannot_hold_credentials": {"disposition": "rust-test", "group": "chain-sync", "reason": "pins the Python bare-host disclosure rule; Rust replacement is bare_backend_location_fullmatch_preserves_only_the_python_allowlist in crates/setup/src/backend_output.rs"},
    },
    "tests/test_transaction_desktop_validation.py": {"*": {"disposition": "rust-test", "group": "transactions", "reason": "pins typed sort/order container refusals; replace with the desktop service validation regressions"}},

    "tests/test_rust_backup_install.py": {"*": {"disposition": "oracle", "group": "secrets-backup", "reason": "compares both codecs, complete restored and rollback bytes, fixed durable-step inventories, every Python/Rust starter and restarter, second interruptions during recovery, and ordinary CLI startup admission"}},
    "tests/test_backup_recovery.py": {"*": {"disposition": "rust-test", "group": "secrets-backup", "reason": "restore storage regression and fault injection; native replacement is the shared restore transaction and cross-runtime durability matrix"}},
    "tests/test_rust_backup_services.py": {"*": {"disposition": "oracle", "group": "secrets-backup", "reason": "compares native backup export and staging import with Python using exact logical SQLCipher/plaintext dumps, manifest and archive refusals; installation has its own killed-child oracle"}},
    "tests/test_rust_backup_desktop.py": {"*": {"disposition": "oracle", "group": "secrets-backup", "reason": "compares unmodified desktop backup JSONL bytes, redacted summaries, cancellation, stale generations, ownership refusals, locked restore outcomes and per-kind native routing on identical disposable database identities"}},
    "tests/test_rust_secrets_lifecycle.py": {"*": {"disposition": "oracle", "group": "secrets-backup", "reason": "compares CLI and daemon secrets wire bytes and complete encrypted states, kills lifecycle writers before either-runtime recovery, and gates native credential cases on a newly launched disposable Linux bus and keyring"}},
    "tests/test_rust_secrets_services.py": {"*": {"disposition": "oracle", "group": "secrets-backup", "reason": "compares encryption CLI bytes and SQLCipher contents with Python and kills publication children before recovery with either runtime; native keyrings are never constructed"}},

    "tests/test_rust_transfer_services.py": {"*": {"disposition": "oracle", "group": "custody", "reason": "Compares CLI/desktop bytes and normalized authoring outputs against Python; native vectors pin complete authoring rows and hybrid routing proves Rust execution"}},
    "tests/test_rust_review_workflow.py": {"*": {"disposition": "oracle", "group": "custody", "reason": "compares review cases, input requests, plans, applies and receipts over unchanged Python daemon and CLI processes on copies of one disposable book: identical bytes and all 112 tables, masking only generated UUIDs and sampled clocks, on SQLite and SQLCipher"}},
    "tests/test_rust_gap_services.py": {"*": {"disposition": "oracle", "group": "custody", "reason": "Compares gap CLI and daemon bytes and complete normalized rows against Python; native fixed-source sequences pin exact identities and hybrid logs require Rust execution"}},

    "tests/test_report_csv_atomic.py": {"*": {"disposition": "rust-test", "group": "reports", "reason": "native store report_csv tests retain interrupted-publication cleanup, umask creation, overwrite modes and final-symlink behavior"}},




    "tests/test_rust_wallet_publication.py": {"*": {"disposition": "oracle", "group": "chain-sync", "reason": "compares atomic Rust publication of Python-prepared Bitcoin fetches against unchanged Python full books across fixtures, restarts, reviewed journals and saved reports; retains rollback and refusal witnesses without claiming end-to-end native sync or product-route acceptance"}},
    "tests/test_rust_wallet_fetch.py": {"*": {"disposition": "oracle", "group": "chain-sync", "reason": "compares independent native Bitcoin preparation and atomic publication against unchanged Python serialized fetches and full logical books across fixtures, subprocess restarts, backend races, current journals and saved reports; product services and route registration remain unaccepted"}},
    "tests/test_rust_wallet_sync_service.py": {"*": {"disposition": "oracle", "group": "chain-sync", "reason": "compares the shared native wallet-sync service with Python's complete CLI sync handler, full envelopes and normalized books across both selection modes, fixtures, restarts and reviewed books; retains process consent and cancellation fences without claiming CLI/daemon registration or hybrid product parity"}},
    "tests/test_rust_wallet_sync_cli.py": {"*": {"disposition": "oracle", "group": "chain-sync", "reason": "compares registered native and Python CLI sync envelopes and every normalized table over both loopback protocols, restart sequences and reviewed books; asserts zero hybrid fallbacks for supported calls and inert unsupported selection"}},
    "tests/test_rust_freshness_queue.py": {"*": {"disposition": "oracle", "group": "daemon-runtime", "reason": "compares native pre-dispatch freshness queue operations, complete envelopes and every logical table against Python on disposable books; captures Python's selected-job and workspace ordering without claiming daemon execution, network, progress-stream or route-registration acceptance"}},
    "tests/test_rust_freshness_admission.py": {"*": {"disposition": "oracle", "group": "daemon-runtime", "reason": "compares inert source specifications and admitted-book plans with Python on disposable books; pins whole-shape refusal, execution exclusion and scope/configuration revalidation without claiming product execution or route registration"}},
    "tests/test_rust_freshness_execution.py": {"*": {"disposition": "oracle", "group": "daemon-runtime", "reason": "compares internal freshness execution and admitted-book aggregation against Python with scripted handlers, typed envelopes, progress payloads and every logical table; pins retry, rollback, checkpoint and UI privacy behavior without claiming native network handlers or live daemon registration"}},
    "tests/test_rust_freshness_native.py": {"*": {"disposition": "oracle", "group": "daemon-runtime", "reason": "compares admitted native Bitcoin freshness jobs, full progress sequences, terminal results and every logical table with Python on disposable loopback backends; verifies guarded daemon stream envelopes and batch cancellation/scope/configuration/consent fences without claiming product route registration or graph handoff"}},
    "tests/test_rust_sync_hybrid_routing.py": {"*": {"disposition": "python-only", "group": "cli-shell", "reason": "pins the test-only Python hybrid launcher handoff, native-result refusal and preservation of consumable unlock inputs", "replacement": "retain while the hybrid Python launcher is required"}},
    "tests/test_rust_bdk_execution.py": {"*": {"disposition": "oracle", "group": "chain-sync", "reason": "compares native BDK aggregates/canonical/reorg/retraction sequences and Electrum/Esplora tip, history, graph-reuse and absence checkpoints to Python over disposable loopback fixtures; does not claim product-route parity"}},
    "tests/test_rust_wallet_acquisition.py": {"test_acquisition_product_defaults_match_python_without_wallets": {"disposition": "oracle", "group": "daemon-runtime", "reason": "compares the raw answers of the three product refresh routes with Python on a book without wallets under no-egress; the journal-only plan runs natively"}, "*": {"disposition": "oracle", "group": "chain-sync", "reason": "compares Bitcoin Esplora/Electrum acquisition, graph decoding and complete normalized records against Python on owned loopback fixtures, including coinbase and changed-height/empty observations; no persisted book parity is claimed"}},
    "tests/test_rust_bitcoin_records.py": {"*": {"disposition": "oracle", "group": "chain-sync", "reason": "compares pure Bitcoin normalized records, shared-observer composition, retraction vetoes and private provenance mappings against Python with no consent or transport; no atomic apply or product sync is claimed"}},
    "tests/test_rust_observer_state.py": {"*": {"disposition": "oracle", "group": "chain-sync", "reason": "compares inert BDK aggregate JSON and private observer snapshot/reuse reads against unmodified Python; checks complete books remain unchanged, without claiming native sync or post-sync dump parity"}},
    "tests/test_rust_pairing_commands.py": {"*": {"disposition": "oracle", "group": "transactions", "reason": "Compares pairing CLI and daemon wire bytes with Python, checks complete unchanged read/dry-run rows and Python projections of Rust writes, and requires zero hybrid fallbacks"}},
    "tests/test_rust_transfer_reads.py": {"*": {"disposition": "oracle", "group": "transactions", "reason": "Compares CLI and daemon component/gap read bytes, complete unchanged database rows, private evidence projections and hybrid Rust routing"}},
    "tests/test_rust_store_migrations.py": {"*": {"disposition": "oracle", "group": "store", "reason": "retains Python/Rust differential storage migration, exact logical-dump and interruption-recovery comparisons"}},
    "tests/test_rust_store_partition_reference.py": {"*": {"disposition": "oracle", "group": "store", "reason": "pins legacy-project reference behavior and compares both runtimes across publication and killed-child recovery"}},
    "tests/test_rust_store_interop.py": {"*": {"group": "store", "reason": "native storage admission, codec, ownership, path and unlock interoperability belongs to the shared storage boundary"}},
    "tests/test_rust_journal_commands.py": {"*": {"disposition": "oracle", "group": "journals", "reason": "compares registered plain and automatic-pairing journal CLI envelopes and complete typed books against Python with bound lifecycle sources, verifies zero-fallback valid routes and scoped refusals, and pins staged-commit recovery and replay authority on both codecs"}},
    "tests/test_rust_journal_ai.py": {"*": {"disposition": "oracle", "group": "journals", "reason": "compares the assistant tool's no-pairing journal step with unchanged Python _journals_refresh_payload(auto_pair=False): payload bytes or typed refusal and every table on both codecs, with a pairing control"}},
    "tests/test_rust_journal_views.py": {"*": {"disposition": "oracle", "group": "journals", "reason": "compares registered desktop journal snapshot, events, transfers and explained quarantine pages with the unchanged Python daemon over every argument variant, plain, paired, generated and malformed books on both codecs; reads must not write"}},
    "tests/test_rust_journal_quarantine_resolve.py": {"*": {"disposition": "oracle", "group": "journals", "reason": "compares registered desktop quarantine resolution envelopes and complete books with the unchanged Python daemon, including every refusal without writes, price overrides, exclusions and optional rebuilds on both codecs"}},
    "tests/test_rust_journal_desktop.py": {"*": {"disposition": "oracle", "group": "journals", "reason": "compares registered desktop journal envelopes, ordered progress and every typed table with Python; pins argument and book authority, automatic pairing, staged commits, cancellation and connection-owner retirement on both codecs"}},
    "tests/test_rust_journal_render.py": {"*": {"disposition": "oracle", "group": "journals", "reason": "regenerates complete pre-persistence build-state vectors from unchanged Python and proves every rendered UUID has a declared bijection; native journal tests compare all state and refusal bytes"}},
    "tests/test_rust_journal_render_branches.py": {
        "test_renderer_branch_witnesses_rebuild_from_python": {"disposition": "oracle", "group": "journals", "reason": "regenerates exact branch states, refusals and observed intermediate audits from production Python; native tests compare those witnesses and exact UUID consumption"},
        "test_renderer_branch_mutations_apply_and_name_existing_tests": {"disposition": "rust-test", "group": "journals", "reason": "validates branch mutation applicability, distinct identities and named native kill tests"},
    },
    "tests/test_rust_journal_persistence.py": {"*": {"disposition": "oracle", "group": "journals", "reason": "regenerates exact storage encodings and replays complete frozen database snapshots through unchanged Python; native tests persist the same stage and ledger inputs and compare every stored scalar, id, timestamp and rowid"}},
    "tests/test_rust_review_builds.py": {"*": {"disposition": "oracle", "group": "journals", "reason": "replays complete review-build states through unchanged Python with declared identity tapes and canonical live-anchor filtering; native tests join loading, custody, fusion and rendering for the same synthetic snapshots"}},
    "tests/test_rust_custody_loader.py": {"*": {"disposition": "oracle", "group": "journals", "reason": "compares native custody snapshot bytes and exact refusals with port-branch Python over disposable SQLite books; retains the test-harness oracle and exact typed revision refusals across eight Python hash seeds"}},
    "tests/test_rust_custody_loader_blobs.py": {"*": {"disposition": "oracle", "group": "journals", "reason": "compares actual SQLite BLOB consumers and JSON serialization refusals with Python at complete CLI/daemon and finalized projection boundaries, recording typeof and zero writes on disposable books"}},
    "tests/test_journal_prebuild_campaign.py": {"*": {"disposition": "oracle", "group": "journals", "reason": "retains complete scenario-vector regeneration from unchanged Python and the rollback-only recorder transaction contract for native whole-stage comparison"}},
    "tests/test_rust_journal_refresh.py": {"*": {"disposition": "oracle", "group": "journals", "reason": "compares unregistered automatic-pairing refresh with Python across complete SQLite/SQLCipher books, ordered result bytes, exact refusals, progress and UUID/clock tapes; retains active pairing, skip-current and two-commit failure witnesses"}},
    "tests/test_rust_journal_process.py": {
        "test_native_prepared_pipeline_matches_live_payload_progress_and_complete_dump": {"disposition": "oracle", "group": "journals", "reason": "compares the unregistered native pipeline with live Python payloads, phases and scalar-exact complete SQLite/SQLCipher dumps, including real cached pricing; proves resolution and repairs leave each prepared fixture unchanged"},
        "test_real_pricing_pipeline_refusals_restore_complete_book": {"disposition": "oracle", "group": "journals", "reason": "compares pricing/build refusals with Python and verifies complete SQLite/SQLCipher rollback after native pricing writes or an injected persistence failure"},
        "test_killed_pipeline_writer_recovers_pricing_and_every_journal_table": {"disposition": "oracle", "group": "journals", "reason": "kills the native SQLite/SQLCipher writer after three named writes, reopens the database, checks integrity and compares every stored scalar with the starting book, including rollback of cached pricing"},
        "test_real_prebuild_process_vectors_rebuild_from_python": {"disposition": "oracle", "group": "journals", "reason": "regenerates full process payloads, refusals, progress and complete dumps from unchanged Python with real source repair, legacy refresh, pricing and separate durable UUID/lifecycle clock tapes"},
        "test_native_real_prebuild_replays_complete_books_and_clock_uuid_tapes": {"disposition": "oracle", "group": "journals", "reason": "replays all real-prebuild process vectors on physical SQLite/SQLCipher books and compares every scalar, payload/refusal, progress phase and consumed durable identity/clock without masking"},
        "test_native_real_prebuild_preserves_blob_consumers_without_v1_encoding": {"disposition": "oracle", "group": "journals", "reason": "compares actual BLOB notes and JSON consumers through full native processing on SQLite/SQLCipher with Python payload/refusal, complete dumps and callback tapes; a v1 serialization refusal does not replace processing behavior"},
        "test_native_real_prebuild_storage_failure_reverts_every_active_stage": {"disposition": "oracle", "group": "journals", "reason": "injects two storage failures after real source audits, legacy activation and cached pricing, proving complete SQLite/SQLCipher rollback of every active stage"},
    },
    "tests/test_rust_review_cases.py": {"*": {"disposition": "oracle", "group": "custody", "reason": "regenerates unregistered review-case, operation, economic/policy-effect and portable-artifact primitive vectors from current Python; pins complete digests, exact scalars, bounded previews and refusal order while native comparisons preserve books and caller transactions; includes ordered planning and recomputing validation fixtures; includes atomic apply fixtures and input-request fixtures; product routes are compared by test_rust_review_workflow"}},
    "tests/test_rust_custody_loader_blobs.py": {"*": {"disposition": "oracle", "group": "journals", "reason": "compares actual SQLite BLOB consumers and JSON serialization refusals with unchanged Python at complete CLI/daemon and finalized projection boundaries, recording typeof and zero writes on disposable books"}},
    "tests/test_journal_process_campaign.py": {"*": {"disposition": "oracle", "group": "journals", "reason": "counts every full-process generated operand and verifies durable shard resumption and build-fingerprint refusal through the unregistered native process probe"}},
    "tests/test_journal_prebuild_campaign.py": {"*": {"disposition": "oracle", "group": "journals", "reason": "retains complete scenario-vector regeneration from unchanged Python and the rollback-only recorder transaction contract for native whole-stage comparison"}},
    "tests/test_rust_custody_loader.py": {"*": {"disposition": "oracle", "group": "journals", "reason": "compares native custody snapshot bytes and exact refusals with unchanged Python over disposable SQLite books; retains the test-harness oracle and explicit BLOB corruption and Python message-determinism exceptions"}},
    "tests/test_custody_fusion_builder.py": {"*": {"disposition": "oracle", "group": "custody", "reason": "compares the Rust request builder with the original Python builder over independent finalized projections, raw generated books, competing refusals and exact ledger operands; fusion remains outside normal comparison admission"}},
    "tests/test_custody_fusion_reference.py": {"*": {"disposition": "oracle", "group": "custody", "reason": "pins sorted competing-asset first refusal across eight hash seeds after the R2g Python reference fix"}},
    "tests/test_custody_projection_quarantines_port.py": {"*": {"disposition": "oracle", "group": "custody", "reason": "compares the private finalized compiler with unchanged Python, including quarantine origins, issue priority, barrier receipts and refusal admission; counts distinct seed activity and rejects vacuity"}},
    "tests/test_custody_projection_conversions_port.py": {"*": {"disposition": "oracle", "group": "custody", "reason": "compares reviewed conversion metadata with unchanged Python AST; counts distinct seed activity and explicit refusals and rejects vacuous scenarios"}},
    "tests/test_custody_projection_composition_port.py": {"*": {"disposition": "oracle", "group": "custody", "reason": "compares selected projection rows, connected path blocking and scoped barriers with Python; counts distinct seed activity and refusals and rejects vacuous scenarios"}},
    "tests/test_custody_projection_stage.py": {"*": {"disposition": "oracle", "group": "custody", "reason": "compares the full finalized stage against unchanged Python over raw generators and permutations, including refusals; regenerates independent adapter-family and admission-order witnesses"}},
    "tests/test_custody_projection_port.py": {"*": {"disposition": "oracle", "group": "custody", "reason": "compares finalized-row primitives with independent CPython vectors and per-engine scenario counts; pins complete finalized projection as enabled"}},
    "tests/test_custody_quantity_stage.py": {"*": {"disposition": "oracle", "group": "custody", "reason": "compares complete quantity from raw-only v1 books, inherited scenarios and permutations; independently asserts positive per-engine scenario activity"}},
    "tests/test_custody_quantity_port.py": {"*": {"disposition": "oracle", "group": "custody", "reason": "compares frozen and generated exact quantity primitives with Python, asserts per-engine counts and permutations, with complete runtime stage composition and explicit decision-only finalization skips"}},
    "tests/test_custody_interpretation_stage.py": {"*": {"disposition": "oracle", "group": "custody", "reason": "compares complete interpretation through PyO3 over every generator with independent per-engine scenario counts and permutations, frozen migration/admission vectors and caught-discrepancy session checks"}},
    "tests/test_custody_native_partial.py": {"*": {"disposition": "oracle", "group": "custody", "reason": "retains frozen Python native HTLC helper witnesses after full-stage comparison consolidates generated and reservation controls"}},
    "tests/test_custody_channel_partial.py": {"*": {"disposition": "oracle", "group": "custody", "reason": "retains frozen channel helpers, loan authoring direction controls and complete interpretation comparison of persisted loan roles"}},
    "tests/test_custody_safety_partial.py": {"*": {"disposition": "oracle", "group": "custody", "reason": "retains frozen holds/privacy/dependency witnesses and pins quarantine reasons against both complete interpreter stages"}},
    "tests/test_custody_generated_books.py": {"*": {"disposition": "oracle", "group": "custody", "reason": "compares seeded raw snapshots and order permutations across Python and Rust while retaining the independent rate-order behavior pin"}},
    "tests/test_custody_interpretation_partial.py": {"*": {"disposition": "oracle", "group": "custody", "reason": "retains frozen CPython slice and text vectors after whole-stage comparison supersedes the partial executable seam"}},
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
    "tests/test_rust_project_desktop.py": {"*": {"disposition": "oracle", "group": "cli-shell", "reason": "compares native desktop project list/create/select sessions with Python byte for byte, including the project catalog and all 112 tables of every created or switched book in disposable XDG roots"}},
    "tests/test_rust_workspace_shell.py": {"*": {"disposition": "oracle", "group": "daemon-runtime", "reason": "compares native workspace overviews, health, next actions, badges, onboarding, secrets initialization and cancel refusal with Python daemon/CLI bytes and all 112 tables on disposable SQLite/SQLCipher books"}},
    "tests/test_rust_daemon_diagnostics.py": {"*": {"disposition": "oracle", "group": "daemon-runtime", "reason": "compares native ui.logs.snapshot and ui.egress.snapshot sessions with Python daemon bytes (wall-clock and source-location fields normalized) on plaintext and locked/unlocked SQLCipher books, with all tables unchanged"}},
    "tests/test_rust_release_cli.py": {"*": {"disposition": "oracle", "group": "cli-shell", "reason": "compares native update preference/refusal and verify-download outputs with Python in JSON, table and plain formats under the strict socket guard, including every signature, manifest and artifact refusal"}},
    "tests/test_rust_setup_desktop.py": {"*": {"disposition": "oracle", "group": "cli-shell", "reason": "compares hybrid Rust setup daemon responses byte for byte with Python on disposable plaintext/encrypted books and Core configuration"}},
    "tests/test_rust_chain_candidate.py": {"*": {"disposition": "oracle", "group": "chain-sync", "reason": "compares five native backend probe kinds against Python on identical loopback fixtures and asserts the hybrid daemon has no Python fallback"}},
    "tests/test_rust_reports.py": {"*": {"disposition": "oracle", "group": "reports", "reason": "compares stored-journal report JSON/table/plain and daemon bytes with Python on disposable FIFO/HIFO/Austrian books, including 3000 transactions, blockers and saved snapshots; asserts native report routing"}},
    "tests/test_rust_report_exports.py": {
        "*": {"disposition": "oracle", "group": "reports", "reason": "compares complete native CLI/daemon CSV export sequences, literal artifact bytes and normalized whole-database state with Python; declares generated identity mappings and proves Rust-only hybrid routing"},
        "test_commercial_subledger_read_csv_bytes_database_and_routing": {"disposition": "oracle", "group": "reports", "reason": "compares commercial CLI JSON/table/plain/CSV bytes, explicit CSV publication and path errors with Python; proves unchanged whole-database state and Rust-only hybrid routing"},
    },
    "tests/test_rust_wallet_discovery.py": {"*": {"disposition": "oracle", "group": "chain-sync", "reason": "compares compatibility descriptor discovery targets, progress and queries with Python on owned loopback fakes; this preparation helper does not establish product sync or book persistence parity"}},
    "tests/test_rust_austrian_xlsx.py": {"*": {"disposition": "oracle", "group": "reports", "reason": "compares native annual workbook content, complete normalized databases, actual saved hashes and explicit filing against Python; proves Rust-only CLI and daemon routing"}},
    "tests/test_rust_gain_reads.py": {"*": {"disposition": "oracle", "group": "reports", "reason": "compares native retained gain explanations and desktop gains with Python, including canonical response bytes, all database tables, compact and legacy calculations, custody bounds and zero explanation CLI/daemon fallbacks; the desktop gains core projection is verified while its journal-refresh-dependent daemon kind stays unregistered"}},
    "tests/test_rust_exit_tax.py": {"*": {"disposition": "oracle", "group": "reports", "reason": "compares native exit-tax projections, wire bytes, XLSX content and all database tables with Python, including refusal gates, estimate disclosures and the reference no-snapshot boundary; requires zero CLI and daemon fallbacks"}},
    "tests/test_journal_freshness_readers.py": {"*": {"disposition": "rust-test", "group": "journals", "reason": "canonical calculation-stamp freshness must gate retained graph, AI, diagnostic and SQL-list projections; native report/store regressions cover the shared stamp and affected readers retain their own contracts"}},
    "tests/test_rust_cross_check.py": {"*": {"disposition": "oracle", "group": "reports", "reason": "compares native diagnostic cross-check response bytes, openpyxl workbook content and all 112 database tables with Python over parser, matching, stale and source-blocked cases; requires zero CLI/daemon fallbacks and preserves the reference no-snapshot boundary"}},
    "tests/test_rust_source_funds_reads.py": {"*": {"disposition": "oracle", "group": "reports", "reason": "compares native stored source, link, case, recipient and evidence listings with unchanged Python, including structured service results, normal/AppError CLI/daemon bytes, output-file bytes, SQLCipher and all 112 tables; requires zero stored-list fallbacks and preserves frozen case metadata, editor disclosure and reference cursor/amount quirks; internal-error process witnesses compare honest native debug strings and all remaining response bytes; review/input handoffs and dry runs have separate oracles"}},
    "tests/test_rust_source_funds_mutations.py": {"*": {"disposition": "oracle", "group": "reports", "reason": "compares native source creation and managed-id evidence association against unchanged Python on SQLite and SQLCipher, including helper envelope bytes, all 112 tables, unchanged managed files and hashes, and CLI/daemon process envelopes with zero mutation fallbacks; AI tool execution and consent remain Python-owned under R6; the separate diagnostic matrix pins honest native debug strings and all non-debug bytes"}},
    "tests/test_rust_source_funds_recipients.py": {"*": {"disposition": "oracle", "group": "reports", "reason": "compares native recipient creation, update and soft deletion with unchanged Python on SQLite and SQLCipher using full helper envelopes, all 112 tables, frozen case history and unchanged file hashes; process checks require native CLI/daemon routing without fallbacks; no recipient AI tools are exposed; the separate diagnostic matrix pins honest native debug strings and non-debug wire parity"}},
    "tests/test_rust_source_funds_links.py": {"*": {"disposition": "oracle", "group": "reports", "reason": "compares native link creation, review and managed-id evidence association with unchanged Python on SQLite and SQLCipher using exact helper envelopes, every one of the 112 tables, attachment bytes/hashes and late rollback; real CLI/daemon calls require identical wire bytes and zero fallbacks; AI execution remains Python-owned; batch review, persisted passes and case production have separate oracles; the diagnostic matrix separately pins honest native debug strings and all non-debug bytes"}},
    "tests/test_rust_source_funds_saves.py": {"*": {"disposition": "oracle", "group": "reports", "reason": "compares native complete report/SVG case saving and fingerprint-bound inspection with unchanged Python on SQLite and SQLCipher, including literal helper/process envelopes, all 112 tables, frozen recipient/external fields, report digests, attachment bytes/hashes, stale/truncated refusals and zero CLI/daemon fallbacks; writer-lock and late rollback witnesses are native; AI tool execution, consent and once-only execution remain Python-owned under R6"}},
    "tests/test_rust_source_funds_batch.py": {"*": {"disposition": "oracle", "group": "reports", "reason": "compares deterministic native batch review with unchanged Python on SQLite and SQLCipher using exact helper and process envelopes, all 112 tables, managed attachment hashes, dry-run identity, partial rollback and zero CLI/daemon fallbacks; structural lineage independently pins current authority, wallet isolation, change chains and exact msat; provenance grants no ownership or tax facts"}},
    "tests/test_rust_source_funds_diagnostics.py": {"*": {"disposition": "oracle", "group": "reports", "reason": "compares unchanged Python and native source-funds CLI/daemon failures on SQLite and SQLCipher with identical non-debug bytes, pinned null/empty debug fields under the approved honest-native contract, daemon secret redaction, all 112 tables, unchanged evidence hashes, late attachment rollback and zero fallbacks; malformed-record post-commit Python failures have a separate reproduction and native rollback tests"}},
    "tests/test_rust_source_funds_case_writes.py": {"*": {"disposition": "oracle", "group": "reports", "reason": "compares the native frozen-case persistence helper with unchanged Python using exact receipt envelope encodings and all 112 tables on SQLite and SQLCipher, including late insert/commit refusals and caller rollback; this low-level writer oracle does not claim report composition or native AI admission; composed CLI/daemon saving has a separate native oracle"}},
    "tests/test_rust_source_funds_report.py": {"*": {"disposition": "oracle", "group": "reports", "reason": "compares complete native source-funds report service bytes with measured unchanged Python seed variants and CLI/daemon envelopes plus output-file bytes; preserves all 112 tables and requires native report routing with zero in-scope fallbacks; covers stored-current/stale and source-blocked books without refreshing journals or opening evidence files; PDF remains Python; case saving has a separate native oracle"}},
    "tests/test_rust_full_xlsx.py": {"*": {"disposition": "oracle", "group": "reports", "reason": "compares native full-report and verification workbook content, normalized wire and all database table bytes, independently verified saved digests and repeated explicit filing against Python; requires Rust-only CLI and daemon routing"}},
    "tests/test_rust_platform_csv.py": {"*": {"disposition": "oracle", "group": "reports", "reason": "compares native CoinTracking/Blockpit CSV bytes, metadata/error bytes and all 112 tables against unchanged Python with explicit Rust-only CLI/daemon routing"}},
    "tests/test_rust_year_handoff.py": {"*": {"disposition": "oracle", "group": "reports", "reason": "compares native year-handoff workbook content, normalized wire bytes and all 112 database tables, actual saved hashes and repeated filing against Python; requires Rust-only CLI and daemon routing"}},
    "tests/test_rust_xlsx.py": {"*": {"disposition": "oracle", "group": "reports", "reason": "compares independent Python XlsxWriter workbooks and native Rust renderer content, styles, cached formulas and deterministic metadata over report contract books and inherited export fixtures; retained until Python retirement"}},
    "tests/test_rust_xlsx_exports.py": {"*": {"disposition": "oracle", "group": "reports", "reason": "compares native transaction XLSX projections, metadata, all database tables and CLI/daemon routing with Python over inherited export fixtures and ready/stale books"}},
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
    "tests/test_schema_swap_matching.py": {
        "FreshSchemaTests::test_active_payout_component_freezes_and_retires_with_review": {"disposition": "rust-test", "group": "transactions", "reason": "Deletion and idempotence are pinned by payout_deletions_match_python_complete_database_rows_and_refusals; native payout creation remains open and the Python setup is retained"},
    },
    "tests/test_custody_authored_migration.py": {
        "*": {"group": "store", "reason": "legacy SQL backfill and trigger invariants port to Rust store migration tests"},
        "test_native_review_activation_failure_leaves_no_committed_draft": {"disposition": "rust-test", "group": "transactions", "reason": "crates/transfers/tests/pairs.rs pins complete-row refusal parity without retaining a draft"},
        "test_pair_invalidation_failure_keeps_complete_database_unchanged": {"disposition": "rust-test", "group": "transactions", "reason": "pair_final_invalidation_refusals_are_atomic pins rollback after create, update and unpair invalidation failures"},
    },
    "tests/test_custody_quantity_runtime.py": {"CustodyQuantityRuntimeTests::test_gap_search_projection_is_a_required_runtime_input": {"disposition": "python-only", "reason": "inspects a Python keyword-only signature", "replacement": "Rust API requires the gap-search input in its type signature"}},
    "tests/test_freshness.py": {"FreshnessTest::test_module_docstring_is_visible_to_ast": {"disposition": "python-only", "reason": "checks Python module docstring placement for AST tooling", "replacement": "Rust module documentation and native inventory checks"}},
    "tests/test_transfer_matching.py": {
        "PaymentHashExactMatchTests": {"disposition": "rust-test", "reason": "R4 pairing matcher byte oracle and hydrated full-database service oracle in crates/transfers/tests/pairing_matcher.rs and pairing_services.rs"},
        "ProviderEvidenceExactMatchTests": {"disposition": "rust-test", "reason": "R4 pairing matcher byte oracle and hydrated full-database service oracle in crates/transfers/tests/pairing_matcher.rs and pairing_services.rs"},
        "ProviderEvidenceAdversarialTests": {"disposition": "rust-test", "reason": "R4 pairing matcher byte oracle and hydrated full-database service oracle in crates/transfers/tests/pairing_matcher.rs and pairing_services.rs"},
        "HeuristicMatchTests": {"disposition": "rust-test", "reason": "R4 pairing matcher byte oracle and hydrated full-database service oracle in crates/transfers/tests/pairing_matcher.rs and pairing_services.rs"},
        "ConflictClusteringTests": {"disposition": "rust-test", "reason": "R4 pairing matcher byte oracle and hydrated full-database service oracle in crates/transfers/tests/pairing_matcher.rs and pairing_services.rs"},
        "PairAndDismissalSuppressionTests": {"disposition": "rust-test", "reason": "R4 pairing matcher byte oracle and hydrated full-database service oracle in crates/transfers/tests/pairing_matcher.rs and pairing_services.rs"},
        "ExcludedRowsTests": {"disposition": "rust-test", "reason": "R4 pairing matcher byte oracle and hydrated full-database service oracle in crates/transfers/tests/pairing_matcher.rs and pairing_services.rs"},
        "LightningPaymentHashSuppressionTests": {"disposition": "rust-test", "reason": "R4 pairing matcher byte oracle and hydrated full-database service oracle in crates/transfers/tests/pairing_matcher.rs and pairing_services.rs"},
        "RefundLinkMatchingTests": {"disposition": "rust-test", "reason": "R4 pairing matcher byte oracle and hydrated full-database service oracle in crates/transfers/tests/pairing_matcher.rs and pairing_services.rs"},
        "HeuristicMatchTests::test_window_slice_matches_naive_all_pairs_reference": {"disposition": "oracle", "reason": "compares optimized window matching with an independent naive all-pairs reference; captured calls also pin the Rust matcher"},
    },
    "tests/rust_port/test_inventory.py": {"*": {"disposition": "oracle", "group": "packaging-ci", "reason": "pins the migration disposition inventory and its completeness barriers", "flags": []}},
    "tests/rust_port/test_contracts.py": {"*": {"disposition": "oracle", "group": "packaging-ci", "reason": "pins generated Python boundary/source contracts for the Rust replacement"}},
    "tests/test_cli_session_replay.py": {"*": {"disposition": "oracle", "group": "cli-shell", "reason": "records Python sessions and proves replay against a candidate fails on injected differences while Python remains the reference"}},
    "tests/test_cli_runner.py": {"*": {"disposition": "oracle", "group": "cli-shell", "reason": "pins the one switch that points every product subprocess at the Rust candidate during the port"}},
    "tests/test_rust_bip329_services.py": {"*": {"disposition": "oracle", "group": "transactions", "reason": "compares Rust CLI/daemon envelopes, persisted label/audit state and exact BIP329 file bytes against Python on disposable books"}},
    "tests/test_rust_desktop_settings.py": {"*": {"disposition": "oracle", "group": "chain-sync", "reason": "compares native freshness status/configure and maintenance settings/configure raw JSONL and all 112 tables with Python, including policy shapes, refusals, commit order and SQLCipher books"}},
    "tests/test_rust_desktop_backends.py": {"*": {"disposition": "oracle", "group": "network-egress", "reason": "compares the native active-profile backend list raw JSONL and all 112 tables with Python, including redaction, wallet scope and default-backend summaries"}},
    "tests/test_rust_desktop_utxos.py": {"*": {"disposition": "oracle", "group": "chain-sync", "reason": "compares the native wallet UTXO inventory raw JSONL and all 112 tables with Python across every support branch, source filters, truncation and the reference surrogate crash"}},
    "tests/test_rust_desktop_lightning_test.py": {"*": {"disposition": "oracle", "group": "network-egress", "reason": "compares the native Lightning backend test raw JSONL, owned fake-node requests and all 112 tables with Python, including refusals, failures and the product no-egress switch"}},
    "tests/test_rust_desktop_settings_egress.py": {"*": {"disposition": "oracle", "group": "network-egress", "reason": "proves desktop status, settings, backend list and UTXO reads open no connection under the strict test guard while matching Python bytes and tables"}},
    "tests/test_rust_wallet_identify_onchain.py": {"*": {"disposition": "oracle", "group": "network-egress", "reason": "drives the Python and Rust daemons step by step against loopback Esplora/Electrum fixtures and compares streamed progress, terminal reports, mid-request and early cancellation, refusals, offline refusals and recorded backend traffic, including no lookup before the explicit action"}},
    "tests/test_rust_wallet_import_desktop.py": {"*": {"disposition": "oracle", "group": "imports", "reason": "compares native desktop file imports, inline Wasabi bundles, Samourai source sets and file analysis raw JSONL and all 112 tables with Python on SQLite and SQLCipher, including the local journal step, disabled journals and refusals, masking only generated identifiers and the clock"}},
    "tests/test_rust_wallet_document_import.py": {"*": {"disposition": "oracle", "group": "imports", "reason": "drives both daemons through staged document grants, local-OCR previews against a loopback provider and reviewed imports, comparing every record, provider request and all 112 tables on SQLite and SQLCipher, including renderer-field refusals, lock and book-switch withdrawal and the journal step"}},
    "tests/test_rust_wallet_reveal.py": {"*": {"disposition": "oracle", "group": "secrets-backup", "reason": "compares the native descriptor reveal raw JSONL and all 112 tables with Python, including target refusals, the plaintext acknowledgement, fresh SQLCipher authentication on an unlocked book and wallet selection, and checks no AI tool reaches the kind"}},
    "tests/test_rust_wallet_onboarding.py": {"*": {"disposition": "oracle", "group": "imports", "reason": "compares native desktop identification, uploaded-ledger previews and script-type detection raw JSONL and all 112 tables with Python on SQLite and SQLCipher, including refusals, retired policies, invoices, silent payments, CSV harvesting, offline/no-egress fallbacks and a loopback no-traffic regression"}},
    "tests/test_rust_document_services.py": {"*": {"disposition": "oracle", "group": "transactions", "reason": "compares document CLI and daemon refusals, evidence storage and prior-report archives, stored matching and commercial context against Python on disposable books"}},
    "tests/test_rust_transaction_dashboard.py": {"*": {"disposition": "oracle", "group": "transactions", "reason": "compares native desktop dashboard raw JSONL and all 112 tables with Python, including periods, zone folds, candidates, reviewed pairs and stale journals"}},
    "tests/test_rust_report_capital_gains.py": {"*": {"disposition": "oracle", "group": "reports", "reason": "compares native desktop capital gains raw JSONL and all tables with Python, including the stale-journal rebuild prelude and the report-read refresh before the read"}},
    "tests/test_rust_rates_rebuild.py": {"*": {"disposition": "oracle", "group": "network-egress", "reason": "compares native desktop rates rebuild with Python on disposable books, including live opt-in, the offline gate, local Kraken repricing and journal rebuilds"}},
    "tests/test_rust_ai_tools.py": {
        "AiCatalogDriftTest": {"disposition": "oracle", "group": "ai-mcp-operator", "reason": "regenerates the embedded AI catalog and the Python vectors the kassiber-ai tests replay, and checks the AI mutation catalog still applies"},
        "AiToolExecutorParityTest": {"disposition": "oracle", "group": "ai-mcp-operator", "reason": "compares the Rust AI read executor (test.ai_tool, test.ai_auto_read) with Python's executor through a reference wrapper, raw JSONL per read tool, argument case, executor state and fixture book, and repeats the Rust run in an empty network namespace"},
    },
    "tests/test_rust_audit_projections.py": {"*": {"disposition": "oracle", "group": "reports", "reason": "compares native audit change counts and the worklist direct-dispatch refusal raw JSONL and all 112 tables with Python"}},
    "tests/test_rust_custody_projections.py": {"*": {"disposition": "oracle", "group": "transactions", "reason": "compares native custody coverage and lineage raw JSONL, cursors and all 112 tables with Python"}},
    "tests/test_rust_transfer_review_context.py": {"*": {"disposition": "oracle", "group": "transactions", "reason": "compares the native transfer review context raw JSONL and all 112 tables with Python, including redaction, journal context and the stale-journal rebuild"}},
    "tests/test_rust_export_preview.py": {"*": {"disposition": "oracle", "group": "reports", "reason": "compares the native export preview raw JSONL and all 112 tables with Python for transaction, platform, year-handoff and label exports, including refusals and draft lists"}},
    "tests/test_rust_report_blockers.py": {"*": {"disposition": "oracle", "group": "reports", "reason": "compares native report blockers raw JSONL and all 112 tables with Python for every barrier, cached counts, missing context and the stale-journal rebuild"}},
    "tests/test_rust_maintenance_run.py": {"*": {"disposition": "oracle", "group": "reports", "reason": "compares the native maintenance run raw JSONL and all 112 tables with Python, including refusals, sync modes without refresh and journal processing"}},
    "tests/test_rust_report_read_sync.py": {"*": {"disposition": "oracle", "group": "network-egress", "reason": "compares native report-read wallet refresh with Python, including source-class filtering, failed refresh blockers, the rate limit and maintenance sync modes"}},
    "tests/test_rust_evidence_summary.py": {"*": {"disposition": "oracle", "group": "reports", "reason": "compares the native audit evidence summary raw JSONL and all 112 tables with Python for whole-book, bounded and case scopes, options and refusals"}},
    "tests/test_rust_transaction_graph.py": {"*": {"disposition": "oracle", "group": "transactions", "reason": "compares the native transaction graph raw JSONL and all 112 tables with Python, including ownership, Liquid legs, local links, custody annotations and the public-lookup refusal"}},
    "tests/test_rust_transaction_review_context.py": {"*": {"disposition": "oracle", "group": "transactions", "reason": "compares the native transaction review context raw JSONL and all 112 tables with Python, including redaction, sections, refusals and the absent read prelude"}},
    "tests/test_rust_desktop_projections_egress.py": {"*": {"disposition": "oracle", "group": "network-egress", "reason": "proves the desktop projection reads open no connection without consent with the strict socket guard armed and product egress enabled"}},
    "tests/test_rust_desktop_settings.py": {"*": {"disposition": "oracle", "group": "network-egress", "reason": "compares native freshness and maintenance settings reads and configure writes with Python, including refusals and all 112 tables"}},
    "tests/test_rust_transaction_resolve.py": {"*": {"disposition": "oracle", "group": "transactions", "reason": "compares native desktop resolution raw JSONL and all 112 tables with Python, including aliases, focused pair amounts, stale journals and reviewed fingerprints"}},
    "tests/test_rust_transaction_desktop.py": {"*": {"disposition": "oracle", "group": "transactions", "reason": "compares desktop list/search/extremes raw daemon JSONL, all filters, grouped totals, queue scope, cursor pages and typed refusals with Python on disposable books"}},
    "tests/test_rust_transaction_listing.py": {"*": {"disposition": "oracle", "group": "transactions", "reason": "compares observed Rust CLI pages, all filters, sort and cursor bytes, exact msat values and fail-closed export/desktop barriers with Python on disposable books"}},
    "tests/test_rust_grouped_metadata_services.py": {"*": {"disposition": "oracle", "group": "transactions", "reason": "compares grouped Rust desktop saves, typed validation, exact price strings, append-only audit fields and version transitions against Python on disposable books"}},
    "tests/test_rust_hybrid_candidate.py": {"*": {"disposition": "oracle", "group": "cli-shell", "reason": "pins the hybrid candidate that routes each CLI call to Rust or the Python reference during the port"}},
    "tests/test_rust_source_funds_review.py": {"*": {"disposition": "oracle", "group": "reports", "reason": "compares complete fingerprint-bound native review and input packets with unchanged Python bytes, recipe files and CLI/daemon envelopes; checks all 112 tables and requires zero in-scope fallbacks while retaining account HOME"}},
    "tests/test_rust_source_funds_coverage.py": {"*": {"disposition": "oracle", "group": "reports", "reason": "compares native historical inbound coverage, caps, exact totals and complete service/CLI/daemon bytes with unchanged Python; checks all 112 tables and zero fallbacks while preserving account HOME"}},
    "tests/test_rust_source_funds_suggest.py": {"*": {"disposition": "oracle", "group": "reports", "reason": "compares complete native suggestion previews with unchanged Python bytes and declared one-to-one generated UUID mapping; checks every table, rollback and zero fallbacks while retaining account HOME"}},
    "tests/test_rust_source_funds_passes.py": {"*": {"disposition": "oracle", "group": "reports", "reason": "compares persisted native suggestion and assembly passes with unchanged Python helper envelopes, call ordering, process bytes and per-pass commit boundaries on SQLite and SQLCipher; preserves frozen assembly inputs, all 112 tables and attachment hashes, requires zero CLI/daemon fallbacks and separately pins native late projection rollback"}},
    "tests/test_rust_source_funds_routes.py": {"*": {"disposition": "oracle", "group": "reports", "reason": "audits every source-funds CLI leaf and desktop daemon kind against the Python inventory, pins native registration, compares process refusals and reads on both bindings with all 112 tables and attachment hashes, and records PDF exports as explicitly Python-owned"}},
}

# R5 wallet lifecycle replacements. Mixed sync/observer obligations remain
# explicit below; the safe wallet-list snapshot has its own native vectors.
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

# R5 lifecycle remainder: pure descriptor/material computation stays native;
# scanner acquisition, sync classification and technical-coverage snapshots do not.
REVIEWS["tests/test_wallet_setup.py"] = {
    "*": {
        "disposition": "rust-test", "group": "chain-sync",
        "reason": "frozen synthetic material vectors and native watch-only preflight cover public/private SLIP132, script sets, checksums, JSON, BSMS and exported descriptor pairs",
        "replacement": "crates/wallets/src/material.rs::python_material_vectors; crates/wallets/src/descriptors.rs::python_descriptor_vectors; crates/wallets/src/lifecycle/tests.rs::spending_private_create_and_persisted_update_fail_without_writes",
    },
}
REVIEWS["tests/test_wallet_descriptors.py"] = {
    "*": {
        "disposition": "rust-test", "group": "chain-sync",
        "reason": "native Python vectors cover derivation, Liquid domain tags, fixed and multipath branches, network inference, private-key refusal and fixed multi-script indices",
        "replacement": "crates/wallets/src/descriptors.rs::python_descriptor_vectors; python_normalization_and_branch_vectors; crates/wallets/src/material.rs::python_material_vectors",
    },
}
for _graph_oracle in (
    "test_native_graph_reference_full_book_parity",
    "test_native_graph_transaction_limit_cursor_and_cache_parity",
    "test_native_graph_sanitizes_legacy_reference_fields",
    "test_native_graph_shared_prevout_budget_leaves_refused_row_first",
    "test_native_graph_refuses_transaction_over_prevout_limit",
    "test_native_graph_row_limit_and_private_cursor_parity",
    "test_native_graph_local_inspection_limit_fails_closed",
    "test_native_graph_failures_advance_cursor_without_backend_fallback",
    "test_native_graph_local_scope_and_cache_guards",
):
    REVIEWS["tests/test_rust_freshness_graph.py"][_graph_oracle] = {
        "disposition": "oracle", "group": "daemon-runtime",
        "reason": "compares bounded native graph completion payloads and every normalized table with Python over disposable Electrum/Esplora backends, including local conflicts, network scope, sanitized caches and private cursor budgets; freezes only admission time and does not claim product daemon registration",
    }

REVIEWS["tests/test_wallet_config_multi_script.py"].update({
    "XpubWalletIsSyncableTests::test_has_descriptor_sync_material": {
        "disposition": "rust-test", "group": "chain-sync",
        "reason": "native snapshot and config vectors recognize derivable public xpub/script sets and refuse absent derivation material",
        "replacement": "crates/wallets/src/snapshot.rs::python_wallet_snapshot_vectors; crates/wallets/src/config.rs::python_wallet_config_vectors",
    },
    "XpubWalletIsSyncableTests::test_backend_summary_is_descriptor_sync_mode": {
        "disposition": "rust-test", "group": "chain-sync",
        "reason": "native snapshot vectors retain backend-descriptor mode for xpub/script-set configurations",
        "replacement": "crates/wallets/src/snapshot.rs::python_wallet_snapshot_vectors",
    },
    "WatchOnlyWalletConfigTests::test_persisted_private_wallet_fails_closed_on_update_and_sync": {
        "disposition": "rust-test", "group": "chain-sync",
        "reason": "lifecycle update is covered natively; this mixed test's sync preflight remains a later chain-sync acceptance obligation",
        "replacement": "crates/wallets/src/lifecycle/tests.rs::spending_private_create_and_persisted_update_fail_without_writes (sync half remains unported)",
    },
    "WalletConfigFreshnessTests::test_script_type_migration_keeps_prior_scripts_in_owned_index": {
        "disposition": "rust-test", "group": "chain-sync",
        "reason": "lifecycle retains prior policy material; construction of the observer-owned script index remains in the unported sync slice",
        "replacement": "crates/wallets/src/lifecycle/tests.rs::policy_rotation_retains_material_and_is_atomic (observer-owned index remains unported)",
    },
})
REVIEWS["tests/test_silent_payments.py"] = {
    "SilentPaymentsTests::test_watch_only_wallet_creation_redacts_material": {
        "disposition": "rust-test", "group": "chain-sync",
        "reason": "native lifecycle persists private scan material but only safe presence metadata reaches ordinary wallet and desktop output",
        "replacement": "crates/wallets/src/lifecycle/tests.rs::birthdays_and_silent_payment_scan_keys_persist_without_spending_authority; crates/wallets/src/snapshot.rs::python_wallet_snapshot_vectors",
    },
    "SilentPaymentsTests::test_inactive_wallet_key_fields_cannot_store_spending_keys": {
        "disposition": "rust-test", "group": "chain-sync",
        "reason": "native configuration vectors reject private spending material in inactive descriptor, change and xpub fields on Silent Payments wallets",
        "replacement": "crates/wallets/src/config.rs::python_wallet_config_vectors",
    },
    "SilentPaymentsTests::test_rejects_spending_material_and_requires_scan_start": {
        "disposition": "rust-test", "group": "chain-sync",
        "reason": "native config vectors pin nested spend-key rejection, key-network families, explicit scan birthdays and independent full-history/server acknowledgements",
        "replacement": "crates/wallets/src/config.rs::python_wallet_config_vectors",
    },
}
REVIEWS.setdefault("tests/test_sync_backends.py", {})["SyncBackendsTest::test_wallet_birthday_validation_and_unix_conversion"] = {
    "disposition": "rust-test", "group": "chain-sync",
    "reason": "birthday configuration normalization is covered by native vectors; backend Unix conversion remains in the later sync slice",
    "replacement": "crates/wallets/src/config.rs::python_wallet_config_vectors (backend conversion remains unported)",
}

# R5 local file ingestion: sync/native-node stamping and historical backfill
# remain separate obligations; these replacements cover the enabled adapters.
REVIEWS["tests/test_payment_hash_promotion.py"] = {
    "PhoenixNormalizerTests": {
        "disposition": "rust-test", "group": "imports",
        "reason": "frozen Python Phoenix CSV vectors pin promoted, normalized, missing and malformed payment hashes",
        "replacement": "crates/wallets/src/file_import/tests.rs::python_file_import_vectors",
    },
    "InsertPersistsPaymentHashTests::test_phoenix_import_writes_column": {
        "disposition": "rust-test", "group": "imports",
        "reason": "native Phoenix ingestion persists the normalized hash and its importer source in the admitted store",
        "replacement": "crates/wallets/src/file_import/tests.rs::phoenix_metadata_is_idempotent_preserves_authored_notes_and_invalidates_once",
    },
    "InsertPersistsPaymentHashTests::test_unchanged_repeat_import_does_not_invalidate_journals": {
        "disposition": "rust-test", "group": "imports",
        "reason": "native generic CSV repeat imports create no run or journal invalidation",
        "replacement": "crates/wallets/src/file_import/tests.rs::csv_import_dedupe_batch_membership_and_journal_invalidation",
    },
}

REVIEWS["tests/test_exchange_importers.py"] = {
    "ExchangeImporterTest::test_bullbitcoin_parser_accepts_lbtc_amount_suffix": {
        "disposition": "rust-test", "group": "imports",
        "reason": "native Bull vectors pin LBTC/BTC suffixes and buy/sell amounts; remaining provider-file/API duties stay unported",
        "replacement": "crates/wallets/src/file_import/tests.rs::python_bull_import_vectors",
    },
    "ExchangeImporterTest::test_river_loader_normalizes_header_whitespace_like_row_parsing": {
        "disposition": "rust-test", "group": "imports",
        "reason": "native River vectors pin whitespace, case folding and ordered duplicate/colliding headers; other provider-file/API duties remain unported",
        "replacement": "crates/wallets/src/file_import/tests.rs::python_river_import_vectors",
    },
}

REVIEWS["tests/test_rust_diagnostics.py"] = {"*": {
    "disposition": "oracle", "group": "secrets-backup",
    "reason": "compares native public diagnostics/export and CLI failure bytes with Python; normalizes only explicitly allowed native runtime and stack values, with normalization-scope regressions",
}}
REVIEWS["tests/test_rust_lightning_reports.py"] = {"*": {
    "disposition": "oracle", "group": "reports",
    "reason": "regenerates the unchanged Python profitability/CSV, scoped-lookup and window-coercion corpus, reproduces retained reference quirks and validates the service mutation catalog; native tests and process comparisons cover the served CLN/LND report family",
}}
REVIEWS["tests/test_rust_lightning_acquisition.py"] = {"*": {
    "disposition": "oracle", "group": "reports",
    "reason": "regenerates LND report acquisition and ancillary-refusal projections from unchanged Python; compares real native/Python processes over disposable loopback HTTP, TLS and explicit proxies, pins error categories and privacy differences, and verifies consent/offline/no-egress zero-hit refusals",
}}
REVIEWS["tests/test_rust_cln_acquisition.py"] = {"*": {
    "disposition": "oracle", "group": "reports",
    "reason": "regenerates CLN report projections and ancillary refusals from unchanged Python, compares real native/Python subprocess acquisition on disposable loopback nodes, verifies consent/offline/no-egress zero-hit refusals and reproduces reference quirks",
}}
REVIEWS["tests/test_rust_lightning_service.py"] = {"*": {
    "disposition": "oracle", "group": "reports",
    "reason": "compares real native CLI/daemon report bytes, CSV artifacts, request order and all 112 database tables with unchanged Python on owned loopback nodes; proves zero hybrid fallbacks, inert configuration/display reads, mid-acquisition scope refusals, TLS/proxy parity and explicit privacy differences",
}}
REVIEWS["tests/test_review_regressions.py"] = {
    "ReviewRegressionTest::test_river_parser_accepts_xbt_amount_suffix": {
        "disposition": "rust-test", "group": "imports",
        "reason": "native River vectors pin XBT amount suffixes and the accepted currency aliases",
        "replacement": "crates/wallets/src/file_import/tests.rs::python_river_import_vectors",
    },
    "ReviewRegressionTest::test_public_diagnostics_collect_omits_sensitive_state": {
        "disposition": "boundary-fixture", "group": "secrets-backup",
        "reason": "runs diagnostics collect/save with planted wallet/backend/transaction secrets; native hybrid routing is required",
    },
    "ReviewRegressionTest::test_diagnostics_out_auto_writes_public_error_report": {
        "disposition": "boundary-fixture", "group": "secrets-backup",
        "reason": "runs automatic/custom public error exports; native hybrid routing is required",
    },
    "ReviewRegressionTest::test_public_diagnostics_sanitizes_xpubs_integer_amounts_and_argument_text": {
        "disposition": "rust-test", "group": "secrets-backup",
        "reason": "Rust replacements: all_text_rules_match_python_bytes, every_sensitive_key_matches_python, recursive_details_shapes_match_python, every_argument_class_matches_python in crates/diagnostics/tests",
    },

}

REVIEWS["tests/test_wasabi_import.py"] = {
    "WasabiBundleParserTest": {
        "disposition": "rust-test", "group": "imports",
        "reason": "Native frozen bundle and transaction vectors preserve sanitized metadata, integer/msat coercions, source aliases and privacy fields; coordinator and late-failure cases cover both codecs. The mixed CLI/daemon flow class remains pending its complete transport boundary.",
        "replacement": "crates/wallets/src/file_import/tests.rs::python_wasabi_import_vectors; crates/wallets/src/file_import/tests.rs::wasabi_bundle_metadata_vectors; crates/wallets/src/file_import/tests.rs::wasabi_coordinator_preserves_review_notes_inventory_and_identities; crates/wallets/src/file_import/tests.rs::wasabi_late_failure_rolls_back_transactions_metadata_inventory_and_readiness; crates/wallets/src/file_import/tests.rs::wasabi_surrogate_inputs_cannot_become_valid_quantities_or_flags; crates/wallets/src/file_import/tests.rs::wasabi_missing_file_refusal_is_atomic",
    },
}

REVIEWS["tests/test_rust_samourai_import.py"] = {"*": {
    "disposition": "oracle", "group": "imports",
    "reason": "Frozen Python source files, raw CLI bytes and complete typed book dumps compare native Samourai ingestion on SQLite and SQLCipher; hybrid scenarios assert native routing, including refusals and group conflicts.",
}}

REVIEWS["tests/test_rust_document_ingestion.py"] = {"*": {
    "disposition": "oracle", "group": "imports",
    "reason": "compares frozen Python OCR draft, reviewed-row, interactive CLI, provider and PDF page-selection vectors, native core and local-provider replies, exact Poppler OCR input preparation and outgoing payloads, every typed schema/table row and managed evidence bytes on SQLite/SQLCipher; retains Python as the document reference",
}}

REVIEWS["tests/test_rust_document_cli.py"] = {"*": {
    "disposition": "oracle", "group": "imports",
    "reason": "Runs Python, raw native and hybrid document CLI processes with terminal stdin, compares exact exit/stdout/stderr, complete typed books, outgoing provider requests and managed source files on SQLite/SQLCipher, and asserts every hybrid document call is served in Rust.",
}}

REVIEWS["tests/test_rust_psbt.py"] = {"*": {"disposition": "oracle", "group": "chain-sync", "reason": "compares unchanged Python PSBT v0/v2 decoding, bounded framing, UTXO evidence, locks and redacted public analysis with a native guarded JSON driver; preserves complete 112-table reads without registering report or chain-analysis routes, and retains an uncaught embedded-decoder reference-error reproduction"}}

REVIEWS["tests/test_rust_psbt_privacy.py"] = {"*": {"disposition": "oracle", "group": "reports", "reason": "compares unchanged Python reduced PSBT inventory/linkage overlays and scoped report redaction against a guarded native service driver with an injected admitted graph; retains canonical bytes, all 112 tables and mutation applicability without claiming native graph capture, transport acceptance or product routing"}}

REVIEWS["tests/test_rust_psbt_inventory.py"] = {"*": {"disposition": "oracle", "group": "reports", "reason": "compares native inventory-only PSBT baseline capture with the unchanged full Python graph, canonical report bytes and all 112 tables under a strict no-egress observer; pins fail-closed source admission and bounds without claiming historical graph, transport or product routing acceptance"}}


REVIEWS["tests/test_rust_analysis_index.py"] = {"*": {
    "disposition": "oracle", "group": "chain-sync",
    "reason": "compares native occurrence and captured public-observer primitives with unchanged Python, exact canonical bytes and all 112 table read invariants; retains the Python hash-seed repro and named mutations; keeps durable-index and Mirror product acceptance separate",
}}


REVIEWS["tests/test_rust_analysis_ingestion.py"] = {"*": {
    "disposition": "oracle", "group": "chain-sync",
    "reason": "compares the native observation merge kernel and confirmation overlay with unchanged Python, exact bytes, eight ambiguity-order variants and all 112 tables; retains mutation killers and no-consent process checks; durable synchronization and Mirror routing remain separate",
}}


REVIEWS["tests/test_rust_analysis_legacy.py"] = {"*": {
    "disposition": "oracle", "group": "chain-sync",
    "reason": "compares legacy non-finite JSON and surrogate values, keys, observer aliases and source commitments with unchanged Python bytes and all 112 tables; retains manifest reference-failure repros, named mutations and no-consent native processes; durable synchronization and product Mirror routing remain unported",
}}

REVIEWS["tests/test_rust_analysis_traversal.py"] = {"*": {
    "disposition": "oracle", "group": "chain-sync",
    "reason": "compares native traversal over captured snapshots with unchanged Python canonical bytes, shared path budgets, named mutations and no-consent process checks; durable sourcing and Mirror product routing remain separate",
}}


REVIEWS["tests/test_rust_analysis_analytics.py"] = {"*": {
    "disposition": "oracle", "group": "chain-sync",
    "reason": "compares native clustering, topology patterns, attribution and full analytics composition with unchanged Python over captured observer snapshots; retains byte checks, fixed hash-seed replay and named mutations; conditional entropy and product report transports are checked separately",

    "reason": "compares native clustering, topology patterns, attribution and full analytics composition with unchanged Python over captured observer snapshots; retains byte checks, fixed hash-seed replay and named mutations; conditional entropy is checked separately and product report composition remains unported",
}}


REVIEWS["tests/test_rust_analysis_datasets.py"] = {"*": {
    "disposition": "oracle", "group": "chain-sync",
    "reason": "compares native exact subject canonicalization and captured-row dataset matching with unchanged Python bytes, independent visibility limits, fixed hash seeds and named mutations; dataset lifecycle/query services remain separate",
}}



REVIEWS["tests/test_rust_analysis_entropy.py"] = {"*": {
    "disposition": "oracle", "group": "chain-sync",
    "reason": "compares native conditional financial-flow partitions with unchanged Python over captured transaction facts, exact count/link/example/progress bytes, forced cancellation/time/memory boundaries, large-integer logarithms, fixed hash seeds and named mutations; product routing remains separate",
}}


REVIEWS["tests/test_rust_analysis_sources.py"] = {"*": {
    "disposition": "oracle", "group": "chain-sync",
    "reason": "compares SQL source orchestration, canonical index bytes, dataset limits and coverage with unchanged Python and all 112 table read invariants; retains hash-seed variants, a reference script repro, named mutations and no-consent native process checks; legacy JSON, durable synchronization and Mirror product routing remain unported",
}}

REVIEWS["tests/test_rust_psbt_history.py"] = {"*": {"disposition": "oracle", "group": "reports", "reason": "compares native historical/adversary PSBT baselines with the unchanged database graph and exact report bytes; preserves all 112 tables under strict no-egress with both offline-switch states, without counting fixture calls as served product reads"}}

REVIEWS["tests/test_rust_psbt_transport.py"] = {"*": {"disposition": "oracle", "group": "reports", "reason": "compares the actual native CLI/file and daemon adapters with unchanged Python helpers across historical PSBT baselines, exact report/error bytes, all 112 tables and both offline states under strict no-egress; adapter fixtures do not count as served product routes"}}

REVIEWS["tests/test_rust_psbt_product.py"] = {"*": {"disposition": "oracle", "group": "reports", "reason": "compares registered native PSBT CLI/file/output and daemon process bytes with unchanged Python across both offline states, all 112 tables and strict no-egress; proves non-zero Rust hybrid counts with zero fallbacks and pins the explicit native empty unhandled-debug contract"}}

REVIEWS["tests/test_rust_legacy_privacy.py"] = {"*": {"disposition": "oracle", "group": "reports", "reason": "retains unchanged Python legacy scorer vectors for every transaction/inventory rule, canonical bytes and complete 112-table reads through a guarded native driver; checks six hash seeds and named mutation applicability"}}

REVIEWS["tests/test_rust_legacy_privacy_services.py"] = {"*": {"disposition": "oracle", "group": "reports", "reason": "compares the existing legacy snapshot through CLI-launched native daemon sessions and the unchanged configuration-report CLI, including exact wire bytes, all 112 tables, zero fallback routing, strict no-egress and honest native diagnostics; retains named transport mutation applicability"}}

REVIEWS["tests/test_rust_analysis_durable.py"] = {"*": {
    "disposition": "oracle", "group": "chain-sync",
    "reason": "compares native durable triggers, contributions and incremental generation publication with unchanged Python canonical bytes, schema objects and all 112 tables; retains named mutations and no-consent native process checks; product Mirror routing remains separate",
}}

REVIEWS["tests/test_rust_analysis_recovery.py"] = {"*": {
    "disposition": "oracle", "group": "chain-sync",
    "reason": "kills native Mirror publication at every SQL write and commit boundary on disposable SQLite and SQLCipher books in DELETE and WAL modes; compares interrupted atomic state and Rust/Python restarts with unchanged Python canonical bytes and all 112 tables under strict no-egress guards",
}}

REVIEWS["tests/test_rust_chain_analysis_reads.py"] = {"*": {"disposition": "oracle", "group": "chain-sync", "reason": "compares native local graph, entropy, immutable cases, labels and dataset metadata/claim reads with unchanged Python unsorted envelope bytes, all 112 tables, transaction rollback, recorded hash-seed variants and guarded CLI/daemon routing with zero fallback; PSBT, preview and async runtime remain separate"}}

REVIEWS["tests/test_rust_chain_analysis_psbt_reads.py"] = {"*": {"disposition": "oracle", "group": "chain-sync", "reason": "compares native PSBT analyze/compare/entropy, BIP78 signature and output checks, staged source grants and analysis receipts with unchanged Python unsorted envelope bytes and all 112 tables; daemon receipts are compared with job identifiers, elapsed time and completion races masked; CLI/daemon routing runs under strict no-consent guards with zero fallback"}}

REVIEWS["tests/test_rust_chain_analysis_dataset_preview.py"] = {"*": {"disposition": "oracle", "group": "chain-sync", "reason": "compares native streaming dataset preview (strict CSV, JSONL, manifests, claims and URL rules), staged grants and preview receipts with unchanged Python unsorted envelope bytes and all 112 tables; records the reference's hash-seed manifest order variants and pins sorted order; CLI/daemon routing runs under strict no-consent guards with zero fallback"}}

REVIEWS["tests/test_rust_analysis_ops.py"] = {"*": {
    "disposition": "oracle", "group": "chain-sync",
    "reason": "compares seven native label, saved-case and dataset lifecycle CLI/daemon routes with unchanged Python bytes and all 112 tables; checks SQLite/SQLCipher rollback, named mutations, the retained Liquid address reference bug and zero-fallback no-consent processes with offline mode off/on; watches, source/acquisition consent and async workers remain unported",
}}

REVIEWS["tests/test_rust_analysis_watches.py"] = {"*": {
    "disposition": "oracle", "group": "chain-sync",
    "reason": "compares all eight native evidence-watch CLI/daemon routes with unchanged Python bytes and all 112 tables on keyed SQLCipher books; covers saved-report impacts, encryption refusal, SQLCipher DELETE/WAL rollback, named mutations, the retained acknowledgement-transaction reference quirk and zero-fallback no-consent processes with offline mode off/on; the background watch tick remains unported",
}}

REVIEWS["tests/test_rust_analysis_grants.py"] = {"*": {
    "disposition": "oracle", "group": "chain-sync",
    "reason": "compares all five native recurring-acquisition source CLI/daemon routes with unchanged Python bytes and all 112 tables; covers plan identity and routing fingerprints, encryption, exact-plan authorization, revision, expiry, source/book/domain fences, offline refusal, SQLCipher DELETE/WAL rollback, named mutations and zero-fallback processes; no source is contacted and the background runner remains unported",
}}

REVIEWS["tests/test_rust_analysis_acquisition.py"] = {"*": {
    "disposition": "oracle", "group": "chain-sync",
    "reason": "compares the native acquisition planner with unchanged Python bytes and all 112 tables across argument, genesis-pin, backend capability, domain, instance and subject-resolution cases, plus encrypted-book CLI/daemon parity with offline off/on; planning contacts no backend",
}}

REVIEWS["tests/test_rust_analysis_acquisition_apply.py"] = {"*": {
    "disposition": "oracle", "group": "chain-sync",
    "reason": "serves synthetic transactions from loopback Esplora/Core and Electrum fakes and compares native and unchanged-Python request sequences, CLI/daemon bytes and all 112 tables for acquisition apply, including offline, KASSIBER_NO_EGRESS, stale-plan, genesis and spender-index refusals that must send nothing or match; retains the truncated-Electrum Python reference repro",
}}

REVIEWS["tests/test_rust_analysis_dataset_import.py"] = {"*": {
    "disposition": "oracle", "group": "chain-sync",
    "reason": "compares native local attribution imports (CSV/JSONL/adapters, staging batches, replacement, failure marking, hash binding, progress and cancellation for the daemon job form, incomplete-pack cleanup) with unchanged Python bytes and all 112 tables, on SQLite/SQLCipher DELETE/WAL books and through the product CLI with offline mode off/on; nothing egresses",
}}

REVIEWS["tests/test_rust_analysis_dataset_jobs.py"] = {"*": {
    "disposition": "oracle", "group": "chain-sync",
    "reason": "compares the native dataset job starters and staged-token import on the shared analysis runtime with unchanged Python job_starter receipts, progress and all 112 tables over the same staged local file, checks cancellation leaves no active pack, and compares daemon refusals byte for byte with offline mode off/on",
}}

REVIEWS["tests/test_rust_analysis_watch_worker.py"] = {"*": {
    "disposition": "oracle", "group": "chain-sync",
    "reason": "runs the native and unchanged-Python daemon background workers on encrypted books and compares wake-up events, loopback Bitcoin Core request sequences and all 112 tables for watch evaluation, subject and block-range source runs, and a reorg with resume; a delivered event is not repeated",
}}

REVIEWS["tests/test_rust_operator.py"] = {
    "*": {
        "disposition": "oracle",
        "group": "ai-mcp-operator",
        "reason": "runs one broker scenario through the Python client against the Python broker fixture and the Rust --operator-broker-server on copies of one book; compares every reply, error, operation record and child envelope after declared id/time/build/path mappings",
    }
}

REVIEWS["tests/test_rust_operator_cli.py"] = {
    "*": {
        "disposition": "oracle",
        "group": "ai-mcp-operator",
        "reason": "runs one operator CLI scenario and one agent-access daemon scenario per language, each through its own broker on a copy of one book (Python CLI and fixture broker; hybrid Rust CLI, daemon and --operator-broker-server); compares exit codes, output bytes and daemon records after declared id/time/build/path mappings",
    }
}
