BEGIN TRANSACTION;
CREATE TABLE accounts (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    code TEXT NOT NULL,
    label TEXT NOT NULL,
    account_type TEXT NOT NULL,
    asset TEXT,
    created_at TEXT NOT NULL,
    UNIQUE (profile_id, code)
);
CREATE TABLE ai_chat_messages (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES ai_chat_sessions(id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    tool_calls_json TEXT,
    provenance_json TEXT,
    finish_reason TEXT,
    created_at TEXT NOT NULL,
    UNIQUE (session_id, seq)
);
CREATE TABLE ai_chat_sessions (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    provider TEXT,
    model TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE ai_provider_secret_refs (
    provider_name TEXT PRIMARY KEY REFERENCES ai_providers(name) ON DELETE CASCADE,
    store_id TEXT NOT NULL,
    service TEXT NOT NULL,
    account TEXT NOT NULL,
    state TEXT NOT NULL,
    created_at TEXT NOT NULL,
    rotated_at TEXT
);
CREATE TABLE ai_providers (
    name TEXT PRIMARY KEY,
    display_name TEXT,
    base_url TEXT NOT NULL,
    api_key TEXT,
    default_model TEXT,
    kind TEXT NOT NULL,
    notes TEXT,
    acknowledged_at TEXT,
    enabled INTEGER NOT NULL DEFAULT 1,
    default_reasoning_effort TEXT,
    api_format TEXT NOT NULL DEFAULT 'auto',
    detected_api_format TEXT,
    omit_store_field INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE attachments (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    transaction_id TEXT REFERENCES transactions(id) ON DELETE CASCADE,
    attachment_type TEXT NOT NULL,
    label TEXT,
    original_filename TEXT,
    stored_relpath TEXT,
    source_url TEXT,
    media_type TEXT,
    size_bytes INTEGER,
    sha256 TEXT,
    copied_from_attachment_id TEXT,
    copied_from_transaction_id TEXT,
    created_at TEXT NOT NULL
);
CREATE TABLE backends (
    name TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    chain TEXT,
    network TEXT,
    url TEXT NOT NULL,
    auth_header TEXT,
    token TEXT,
    batch_size INTEGER,
    timeout INTEGER,
    tor_proxy TEXT,
    config_json TEXT NOT NULL DEFAULT '{}',
    notes TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE bip329_labels (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    wallet_id TEXT REFERENCES wallets(id) ON DELETE SET NULL,
    record_type TEXT NOT NULL,
    ref TEXT NOT NULL,
    label TEXT,
    origin TEXT,
    spendable INTEGER,
    data_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL
);
CREATE TABLE book_network_bindings (
    profile_id TEXT PRIMARY KEY REFERENCES profiles(id) ON DELETE CASCADE,
    environment_id TEXT NOT NULL UNIQUE,
    environment TEXT NOT NULL CHECK(environment IN ('main','test','signet','regtest')),
    revision INTEGER NOT NULL DEFAULT 1 CHECK(revision = 1),
    chain_instance_id TEXT,
    domains_json TEXT NOT NULL,
    acknowledgements_json TEXT NOT NULL,
    inventory_digest TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE btcpay_account_routes (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    backend_name TEXT NOT NULL,
    store_id TEXT NOT NULL,
    payment_method_id TEXT NOT NULL,
    action TEXT NOT NULL,
    label TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(profile_id, backend_name, store_id, payment_method_id, action)
);
CREATE TABLE btcpay_provenance_records (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    backend_name TEXT,
    store_id TEXT NOT NULL,
    payment_method_id TEXT,
    record_type TEXT NOT NULL,
    stable_key TEXT NOT NULL,
    invoice_id TEXT,
    payment_id TEXT,
    order_id TEXT,
    status TEXT,
    occurred_at TEXT,
    asset TEXT,
    amount INTEGER,
    txid TEXT,
    payment_hash TEXT,
    destination TEXT,
    payment_request_id TEXT,
    origin_kind TEXT,
    origin_app_id TEXT,
    origin_label TEXT,
    origin_url TEXT,
    origin_source TEXT,
    fiat_currency TEXT,
    fiat_value_exact TEXT,
    fiat_rate_exact TEXT,
    pricing_timestamp TEXT,
    raw_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(profile_id, stable_key)
);
CREATE TABLE btcpay_store_sync_states (
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    backend_name TEXT NOT NULL,
    store_id TEXT NOT NULL,
    last_attempt_at TEXT,
    last_success_at TEXT,
    last_error_code TEXT,
    last_error_message TEXT,
    invoices_seen INTEGER,
    payouts_seen INTEGER,
    updated_at TEXT NOT NULL,
    PRIMARY KEY(profile_id, backend_name, store_id)
);
CREATE TABLE chain_analysis_acquisition_grants (
    id TEXT PRIMARY KEY,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    spec_json TEXT NOT NULL,
    binding_json TEXT NOT NULL,
    source_fingerprint TEXT NOT NULL,
    database_identity TEXT NOT NULL,
    status TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 1,
    requests_used INTEGER NOT NULL DEFAULT 0,
    bytes_used INTEGER NOT NULL DEFAULT 0,
    next_run_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    cursor_height INTEGER,
    cursor_hash TEXT,
    verified_tip_height INTEGER,
    verified_tip_hash TEXT,
    verified_tip_at TEXT,
    lease_token TEXT,
    lease_until INTEGER,
    last_code TEXT,
    created_at TEXT NOT NULL
);
CREATE TABLE chain_analysis_cases (
    id TEXT PRIMARY KEY,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    query_json TEXT NOT NULL,
    result_json TEXT NOT NULL,
    snapshot_id TEXT NOT NULL,
    result_digest TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE chain_analysis_dataset_claims (
    dataset_id TEXT NOT NULL REFERENCES chain_analysis_datasets(id) ON DELETE CASCADE,
    record_number INTEGER NOT NULL,
    subject TEXT NOT NULL,
    label TEXT NOT NULL,
    category TEXT NOT NULL,
    source_record_json TEXT NOT NULL,
    valid_from TEXT,
    valid_until TEXT,
    PRIMARY KEY (dataset_id, record_number)
) WITHOUT ROWID;
CREATE TABLE chain_analysis_datasets (
    id TEXT PRIMARY KEY,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    dataset_key TEXT NOT NULL,
    version TEXT NOT NULL,
    chain TEXT NOT NULL,
    network TEXT NOT NULL,
    visibility TEXT NOT NULL CHECK (visibility IN ('public', 'private')),
    status TEXT NOT NULL CHECK (status IN ('staging', 'active', 'superseded', 'revoked', 'failed')),
    revision INTEGER NOT NULL DEFAULT 1,
    manifest_json TEXT NOT NULL,
    content_sha256 TEXT,
    claims_sha256 TEXT,
    row_count INTEGER NOT NULL DEFAULT 0,
    byte_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE chain_analysis_label_history (
    label_id TEXT NOT NULL REFERENCES chain_analysis_labels(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL,
    payload_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (label_id, revision)
);
CREATE TABLE chain_analysis_labels (
    id TEXT PRIMARY KEY,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    chain TEXT NOT NULL,
    network TEXT NOT NULL,
    subject TEXT NOT NULL,
    label TEXT NOT NULL,
    category TEXT NOT NULL,
    source TEXT NOT NULL,
    confidence TEXT NOT NULL,
    cluster_defining INTEGER NOT NULL DEFAULT 0 CHECK (cluster_defining IN (0, 1)),
    revision INTEGER NOT NULL DEFAULT 1,
    deleted INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE chain_analysis_observations (
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    chain TEXT NOT NULL,
    network TEXT NOT NULL,
    txid TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    status_json TEXT NOT NULL DEFAULT '{}',
    source_name TEXT NOT NULL,
    observed_at TEXT NOT NULL,
    PRIMARY KEY (profile_id, chain, network, txid)
);
CREATE TABLE chain_analysis_reference_assertions (
    grant_id TEXT NOT NULL REFERENCES chain_analysis_acquisition_grants(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    domain_id TEXT NOT NULL,
    chain TEXT NOT NULL,
    network TEXT NOT NULL,
    occurrence_id TEXT NOT NULL,
    txid TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    status_json TEXT NOT NULL,
    active INTEGER NOT NULL,
    observed_at TEXT NOT NULL,
    PRIMARY KEY(grant_id,occurrence_id)
);
CREATE TABLE chain_analysis_reference_blocks (
    grant_id TEXT NOT NULL REFERENCES chain_analysis_acquisition_grants(id) ON DELETE CASCADE,
    block_hash TEXT NOT NULL,
    height INTEGER NOT NULL,
    parent_hash TEXT NOT NULL,
    active INTEGER NOT NULL,
    PRIMARY KEY(grant_id,block_hash)
);
CREATE TABLE chain_analysis_watch_inbox (
 id TEXT PRIMARY KEY, watch_id TEXT REFERENCES chain_analysis_watches(id) ON DELETE CASCADE,
 profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
 sequence INTEGER NOT NULL, code TEXT NOT NULL, observation_json TEXT NOT NULL,
 created_at TEXT NOT NULL, acknowledged_at TEXT, delivered_at TEXT,
 UNIQUE(watch_id,sequence)
);
CREATE TABLE chain_analysis_watches (
 id TEXT PRIMARY KEY, profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
 book_id TEXT NOT NULL, domain_json TEXT NOT NULL, definition_json TEXT NOT NULL,
 enabled INTEGER NOT NULL DEFAULT 1, revision INTEGER NOT NULL DEFAULT 1,
 baseline_json TEXT NOT NULL, checkpoint TEXT NOT NULL, sequence INTEGER NOT NULL DEFAULT 0,
 checked_at TEXT NOT NULL, created_at TEXT NOT NULL
);
CREATE TABLE chain_index_aliases(profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE, observer TEXT NOT NULL, alias TEXT NOT NULL, node_id TEXT NOT NULL, PRIMARY KEY(profile_id,observer,alias,node_id));
CREATE TABLE chain_index_clock(id INTEGER PRIMARY KEY CHECK(id=1), revision INTEGER NOT NULL);
INSERT INTO "chain_index_clock" VALUES(1,14);
CREATE TABLE chain_index_dirty(source_table TEXT NOT NULL, source_key INTEGER NOT NULL, revision INTEGER NOT NULL, PRIMARY KEY(source_table,source_key));
INSERT INTO "chain_index_dirty" VALUES('transactions',-1,1);
INSERT INTO "chain_index_dirty" VALUES('transaction_graph_cache',-1,2);
INSERT INTO "chain_index_dirty" VALUES('wallet_utxos',-1,3);
INSERT INTO "chain_index_dirty" VALUES('chain_analysis_observations',-1,4);
INSERT INTO "chain_index_dirty" VALUES('chain_analysis_reference_assertions',-1,5);
INSERT INTO "chain_index_dirty" VALUES('wallets',-1,6);
INSERT INTO "chain_index_dirty" VALUES('profiles',-1,7);
INSERT INTO "chain_index_dirty" VALUES('journal_custody_decisions',-1,8);
INSERT INTO "chain_index_dirty" VALUES('journal_custody_economic_relations',-1,9);
INSERT INTO "chain_index_dirty" VALUES('chain_analysis_labels',-1,10);
INSERT INTO "chain_index_dirty" VALUES('chain_analysis_datasets',-1,11);
INSERT INTO "chain_index_dirty" VALUES('chain_analysis_dataset_claims',-1,12);
INSERT INTO "chain_index_dirty" VALUES('book_network_bindings',-1,13);
INSERT INTO "chain_index_dirty" VALUES('chain_analysis_acquisition_grants',-1,14);
CREATE TABLE chain_index_edges(profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE, id TEXT NOT NULL, source TEXT NOT NULL, target TEXT NOT NULL, payload_json TEXT NOT NULL, public_json TEXT, PRIMARY KEY(profile_id,id));
CREATE TABLE chain_index_finding_nodes(profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE, finding_id TEXT NOT NULL, node_id TEXT NOT NULL, PRIMARY KEY(profile_id,finding_id,node_id));
CREATE TABLE chain_index_findings(profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE, id TEXT NOT NULL, payload_json TEXT NOT NULL, public_json TEXT, PRIMARY KEY(profile_id,id));
CREATE TABLE chain_index_label_nodes(profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE, label_id TEXT NOT NULL, node_id TEXT NOT NULL, PRIMARY KEY(profile_id,label_id,node_id));
CREATE TABLE chain_index_labels(profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE, id TEXT NOT NULL, payload_json TEXT NOT NULL, PRIMARY KEY(profile_id,id));
CREATE TABLE chain_index_nodes(profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE, id TEXT NOT NULL, payload_json TEXT NOT NULL, public_json TEXT, tx_json TEXT, public_tx_json TEXT, output_json TEXT, public_output_json TEXT, seed INTEGER NOT NULL, status TEXT NOT NULL, kind TEXT NOT NULL, complete INTEGER NOT NULL, PRIMARY KEY(profile_id,id));
CREATE TABLE chain_index_source_nodes(profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE, source_table TEXT NOT NULL, source_key INTEGER NOT NULL, node_id TEXT NOT NULL, PRIMARY KEY(profile_id,source_table,source_key,node_id));
CREATE TABLE chain_index_sources(profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE, source_table TEXT NOT NULL, source_key INTEGER NOT NULL, payload_json TEXT NOT NULL, invalid_count INTEGER NOT NULL, rejected_count INTEGER NOT NULL, PRIMARY KEY(profile_id,source_table,source_key));
CREATE TABLE chain_index_state(profile_id TEXT PRIMARY KEY REFERENCES profiles(id) ON DELETE CASCADE, instance_id TEXT NOT NULL, version INTEGER NOT NULL, watermark INTEGER NOT NULL, revision INTEGER NOT NULL, snapshot_id TEXT NOT NULL, coverage_json TEXT NOT NULL, next_expiry TEXT);
CREATE TABLE chain_observation_provenance (
    transaction_id TEXT PRIMARY KEY REFERENCES transactions(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    wallet_id TEXT NOT NULL REFERENCES wallets(id) ON DELETE CASCADE,
    authority_version INTEGER NOT NULL CHECK(authority_version >= 1),
    observer_ids_json TEXT NOT NULL,
    observer_kinds_json TEXT NOT NULL,
    chain TEXT NOT NULL,
    network TEXT NOT NULL,
    application_revision TEXT NOT NULL,
    graph_hash TEXT NOT NULL,
    quantity_hash TEXT NOT NULL,
    fee_attribution TEXT NOT NULL CHECK(
        fee_attribution IN ('exact', 'implicit_wallet_delta', 'unknown')
    ),
    observed_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE chain_observer_coverage (
    observer_id TEXT NOT NULL REFERENCES chain_observer_instances(id) ON DELETE CASCADE,
    branch_key TEXT NOT NULL,
    coverage_version INTEGER NOT NULL,
    highest_used INTEGER,
    scanned_to INTEGER NOT NULL,
    details_json TEXT NOT NULL DEFAULT '{}',
    updated_at TEXT NOT NULL,
    PRIMARY KEY (observer_id, branch_key)
);
CREATE TABLE chain_observer_instances (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    logical_wallet_id TEXT NOT NULL REFERENCES wallets(id) ON DELETE CASCADE,
    source_wallet_id TEXT NOT NULL REFERENCES wallets(id) ON DELETE CASCADE,
    source_key TEXT NOT NULL,
    observer_kind TEXT NOT NULL,
    chain TEXT NOT NULL,
    network TEXT NOT NULL,
    state_version INTEGER NOT NULL,
    state_epoch INTEGER NOT NULL DEFAULT 0,
    state_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (logical_wallet_id, source_wallet_id, source_key)
);
CREATE TABLE chain_observer_values (
    observer_id TEXT NOT NULL REFERENCES chain_observer_instances(id) ON DELETE CASCADE,
    namespace_version INTEGER NOT NULL,
    key TEXT NOT NULL,
    value BLOB NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (observer_id, key)
);
CREATE TABLE commercial_links (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    btcpay_record_id TEXT REFERENCES btcpay_provenance_records(id) ON DELETE CASCADE,
    document_id TEXT REFERENCES external_documents(id) ON DELETE CASCADE,
    transaction_id TEXT REFERENCES transactions(id) ON DELETE CASCADE,
    link_type TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'suggested',
    confidence TEXT NOT NULL DEFAULT 'unknown',
    method TEXT NOT NULL DEFAULT 'manual',
    allocation_amount INTEGER,
    allocation_fiat_exact TEXT,
    reconciliation_state TEXT NOT NULL DEFAULT 'unreviewed',
    commercial_kind TEXT,
    applied_transaction_snapshot_json TEXT,
    reviewed_record_snapshot_json TEXT,
    reviewed_record_snapshot_sha256 TEXT,
    notes TEXT,
    reviewed_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    CHECK (btcpay_record_id IS NOT NULL OR document_id IS NOT NULL),
    CHECK (transaction_id IS NOT NULL OR document_id IS NOT NULL)
);
CREATE TABLE custody_ai_assistance_audits (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    tool_name TEXT NOT NULL,
    daemon_kind TEXT NOT NULL,
    call_id TEXT NOT NULL,
    provider_kind TEXT NOT NULL,
    model TEXT NOT NULL,
    gap_id TEXT,
    candidate_fingerprint TEXT,
    facts_sha256 TEXT NOT NULL CHECK(length(facts_sha256) = 64),
    model_proposal_json TEXT NOT NULL DEFAULT '{}',
    final_proposal_json TEXT NOT NULL DEFAULT '{}',
    user_edited INTEGER NOT NULL DEFAULT 0 CHECK(user_edited IN (0, 1)),
    consent_decision TEXT NOT NULL CHECK(consent_decision IN (
        'allow_once', 'allow_session', 'deny', 'consent_timeout', 'cancelled'
    )),
    consent_requested_at TEXT NOT NULL,
    consent_decided_at TEXT NOT NULL,
    execution_status TEXT NOT NULL CHECK(execution_status IN (
        'executed', 'failed', 'denied', 'cancelled'
    )),
    execution_code TEXT,
    result_sha256 TEXT CHECK(result_sha256 IS NULL OR length(result_sha256) = 64),
    review_id TEXT,
    component_id TEXT,
    created_at TEXT NOT NULL
);
CREATE TABLE custody_authored_evidence_snapshots (
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    subject_kind TEXT NOT NULL CHECK(subject_kind IN (
        'custody_component', 'custody_claim'
    )),
    subject_id TEXT NOT NULL,
    detail_hash TEXT NOT NULL,
    quantity_hash TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY(profile_id, subject_kind, subject_id, detail_hash)
);
CREATE TABLE custody_authored_migration_issues (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    legacy_table TEXT NOT NULL
        CHECK(legacy_table IN ('transaction_pairs', 'direct_swap_payouts')),
    legacy_source_id TEXT NOT NULL,
    issue_code TEXT NOT NULL,
    transaction_ids_json TEXT NOT NULL DEFAULT '[]',
    details_json TEXT NOT NULL DEFAULT '{}',
    resolved_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(legacy_table, legacy_source_id)
);
CREATE TABLE custody_component_allocations (
    id TEXT PRIMARY KEY,
    component_id TEXT NOT NULL REFERENCES custody_components(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    source_leg_id TEXT NOT NULL REFERENCES custody_component_legs(id) ON DELETE CASCADE,
    sink_leg_id TEXT NOT NULL REFERENCES custody_component_legs(id) ON DELETE CASCADE,
    source_amount_msat INTEGER NOT NULL
        CHECK (typeof(source_amount_msat) = 'integer' AND source_amount_msat >= 0),
    sink_amount_msat INTEGER NOT NULL
        CHECK (typeof(sink_amount_msat) = 'integer' AND sink_amount_msat >= 0),
    created_at TEXT NOT NULL,
    UNIQUE (component_id, ordinal),
    UNIQUE (component_id, source_leg_id, sink_leg_id)
);
CREATE TABLE custody_component_economic_terms (
    id TEXT PRIMARY KEY,
    component_id TEXT NOT NULL
        REFERENCES custody_components(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL CHECK(ordinal >= 0),
    source_leg_id TEXT NOT NULL
        REFERENCES custody_component_legs(id) ON DELETE CASCADE,
    target_leg_id TEXT NOT NULL
        REFERENCES custody_component_legs(id) ON DELETE CASCADE,
    term_kind TEXT NOT NULL
        CHECK(term_kind IN ('transaction_pair', 'direct_swap_payout')),
    legacy_source_id TEXT NOT NULL,
    source_row_hash TEXT NOT NULL CHECK(length(source_row_hash) = 64),
    review_kind TEXT NOT NULL,
    tax_policy TEXT NOT NULL,
    reviewed_source_amount_msat INTEGER
        CHECK(reviewed_source_amount_msat IS NULL OR
              typeof(reviewed_source_amount_msat) = 'integer'),
    swap_fee_msat INTEGER,
    swap_fee_kind TEXT,
    confidence_at_review TEXT,
    review_source TEXT,
    review_notes TEXT,
    payout_asset TEXT,
    payout_amount_msat INTEGER
        CHECK(payout_amount_msat IS NULL OR
              typeof(payout_amount_msat) = 'integer'),
    payout_occurred_at TEXT,
    payout_fiat_value_exact TEXT,
    payout_external_id TEXT,
    counterparty TEXT,
    created_at TEXT NOT NULL,
    UNIQUE(component_id, ordinal),
    UNIQUE(component_id, term_kind, legacy_source_id)
);
CREATE TABLE custody_component_evidence_commitments (
    id TEXT PRIMARY KEY,
    component_id TEXT NOT NULL REFERENCES custody_components(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    quantity_hash TEXT NOT NULL CHECK (length(quantity_hash) = 64),
    detail_hash TEXT NOT NULL CHECK (length(detail_hash) = 64),
    created_at TEXT NOT NULL,
    UNIQUE (component_id, ordinal)
);
CREATE TABLE custody_component_legs (
    id TEXT PRIMARY KEY,
    component_id TEXT NOT NULL REFERENCES custody_components(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    role TEXT NOT NULL
        CHECK (role IN ('source', 'destination', 'fee', 'external',
                        'retained', 'unresolved', 'suspense')),
    rail TEXT NOT NULL,
    chain TEXT,
    network TEXT,
    asset TEXT NOT NULL,
    exposure TEXT NOT NULL,
    conservation_unit TEXT NOT NULL,
    amount_msat INTEGER NOT NULL
        CHECK (typeof(amount_msat) = 'integer' AND amount_msat >= 0),
    valuation_unit TEXT,
    valuation_amount INTEGER
        CHECK (valuation_amount IS NULL OR
               (typeof(valuation_amount) = 'integer' AND valuation_amount >= 0)),
    occurred_at TEXT,
    transaction_id TEXT REFERENCES transactions(id) ON DELETE SET NULL,
    -- Durable evidence identity. ``transaction_id`` is a live FK and becomes
    -- NULL when an importer retracts a row; this copy must survive so the
    -- component becomes invalid instead of masquerading as a transactionless
    -- manual leg.
    anchor_transaction_id TEXT,
    wallet_id TEXT REFERENCES wallets(id) ON DELETE SET NULL,
    location_ref TEXT,
    notes TEXT,
    created_at TEXT NOT NULL,
    UNIQUE (component_id, ordinal)
);
CREATE TABLE custody_component_purge_authorizations (
    profile_id TEXT PRIMARY KEY REFERENCES profiles(id) ON DELETE CASCADE
);
CREATE TABLE custody_component_transaction_memberships (
    component_id TEXT NOT NULL REFERENCES custody_components(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    PRIMARY KEY (component_id, transaction_id),
    UNIQUE (profile_id, transaction_id)
);
CREATE TABLE custody_components (
    id TEXT PRIMARY KEY,
    lineage_id TEXT NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    component_type TEXT NOT NULL,
    conservation_mode TEXT NOT NULL DEFAULT 'quantity'
        CHECK (conservation_mode IN ('quantity', 'conversion')),
    state TEXT NOT NULL DEFAULT 'draft'
        CHECK (state IN ('draft', 'active', 'superseded')),
    evidence_kind TEXT,
    evidence_grade TEXT,
    evidence_json TEXT NOT NULL DEFAULT '{}',
    conversion_policy TEXT,
    conversion_reviewed INTEGER NOT NULL DEFAULT 0
        CHECK (conversion_reviewed IN (0, 1)),
    conversion_metadata_json TEXT NOT NULL DEFAULT '{}',
    expected_leg_count INTEGER CHECK (expected_leg_count >= 0),
    expected_allocation_count INTEGER CHECK (expected_allocation_count >= 0),
    expected_economic_term_count INTEGER
        CHECK (expected_economic_term_count >= 0),
    expected_evidence_count INTEGER CHECK (expected_evidence_count >= 0),
    authored_source TEXT DEFAULT 'user',
    notes TEXT,
    change_reason TEXT,
    -- Revision links are authored identifiers, not immediate relational
    -- dependencies.  A partition or restore can insert two mutually-linked
    -- headers in either order, and a book can legitimately retain two
    -- competing revisions until review.  Application validation checks the
    -- links after insertion; an immediate self-FK would reject valid evidence.
    supersedes_component_id TEXT,
    superseded_by_component_id TEXT,
    activated_at TEXT,
    superseded_at TEXT,
    created_at TEXT NOT NULL
);
CREATE TABLE custody_filed_report_impact_resolutions (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    impact_id TEXT NOT NULL UNIQUE
        REFERENCES custody_filed_report_impacts(id) ON DELETE CASCADE,
    rebuilt_at TEXT NOT NULL,
    after_classification_summary_json TEXT NOT NULL DEFAULT '{}',
    after_gain_summary_json TEXT NOT NULL DEFAULT '{}',
    classification_changed INTEGER NOT NULL CHECK(classification_changed IN (0, 1)),
    gain_changed INTEGER NOT NULL CHECK(gain_changed IN (0, 1)),
    amendment_status TEXT NOT NULL CHECK(amendment_status IN (
        'no_change', 'saved_report_changed', 'review_required'
    )),
    created_at TEXT NOT NULL
);
CREATE TABLE custody_filed_report_impacts (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    filed_report_snapshot_id TEXT NOT NULL
        REFERENCES filed_report_snapshots(id) ON DELETE CASCADE,
    component_id TEXT NOT NULL,
    review_id TEXT NOT NULL,
    gap_id TEXT NOT NULL,
    affected_period_start_year INTEGER NOT NULL
        CHECK(affected_period_start_year BETWEEN 1900 AND 9999),
    affected_period_end_year INTEGER NOT NULL
        CHECK(affected_period_end_year BETWEEN affected_period_start_year AND 9999),
    before_classification_summary_json TEXT NOT NULL DEFAULT '{}',
    after_classification_summary_json TEXT NOT NULL DEFAULT '{}',
    before_gain_summary_json TEXT NOT NULL DEFAULT '{}',
    after_gain_summary_json TEXT NOT NULL DEFAULT '{}',
    amendment_warning TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE(filed_report_snapshot_id, review_id)
);
CREATE TABLE custody_gap_review_relation_sets (
    id TEXT PRIMARY KEY,
    review_id TEXT NOT NULL UNIQUE
        REFERENCES custody_gap_reviews(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    expected_source_count INTEGER NOT NULL CHECK(expected_source_count >= 0),
    expected_return_count INTEGER NOT NULL CHECK(expected_return_count >= 0),
    created_at TEXT NOT NULL
);
CREATE TABLE custody_gap_review_transactions (
    id TEXT PRIMARY KEY,
    review_id TEXT NOT NULL REFERENCES custody_gap_reviews(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('source', 'return')),
    transaction_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE (review_id, role, transaction_id)
);
CREATE TABLE custody_gap_reviews (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    gap_id TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision >= 1),
    candidate_fingerprint TEXT NOT NULL,
    action TEXT NOT NULL CHECK(action IN ('dismissed', 'resolved')),
    event_kind TEXT NOT NULL DEFAULT 'review_decision'
        CHECK(event_kind IN (
            'review_decision', 'bridge_created', 'bridge_reopened',
            'bridge_revised', 'residual_classified'
        )),
    component_id TEXT,
    authored_source TEXT NOT NULL DEFAULT 'user'
        CHECK(authored_source IN ('user', 'cli', 'gui', 'ai_tool')),
    reason TEXT,
    snapshot_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL
);
CREATE TABLE direct_swap_payouts (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    out_transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    kind TEXT NOT NULL DEFAULT 'direct-swap-payout',
    policy TEXT NOT NULL DEFAULT 'carrying-value',
    payout_asset TEXT NOT NULL,
    payout_amount INTEGER NOT NULL,
    payout_occurred_at TEXT,
    payout_fiat_value REAL,
    payout_external_id TEXT,
    counterparty TEXT,
    notes TEXT,
    swap_fee_msat INTEGER,
    swap_fee_kind TEXT,
    out_amount INTEGER,
    component_id TEXT REFERENCES custody_components(id) ON DELETE SET NULL,
    deleted_at TEXT,
    created_at TEXT NOT NULL
);
CREATE TABLE external_document_attachments (
    document_id TEXT NOT NULL REFERENCES external_documents(id) ON DELETE CASCADE,
    attachment_id TEXT NOT NULL REFERENCES attachments(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    PRIMARY KEY(document_id, attachment_id)
);
CREATE TABLE external_documents (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    document_type TEXT NOT NULL,
    label TEXT NOT NULL,
    external_ref TEXT,
    issuer TEXT,
    counterparty TEXT,
    issued_at TEXT,
    due_at TEXT,
    fiat_currency TEXT,
    fiat_value_exact TEXT,
    review_state TEXT NOT NULL DEFAULT 'draft',
    notes TEXT,
    raw_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE filed_report_chain_captures (
 snapshot_id TEXT PRIMARY KEY REFERENCES filed_report_snapshots(id) ON DELETE CASCADE,
 profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
 source_snapshot_id TEXT REFERENCES filed_report_snapshots(id),
 captured_at TEXT NOT NULL
);
CREATE TABLE filed_report_chain_dependencies (
 snapshot_id TEXT NOT NULL REFERENCES filed_report_snapshots(id) ON DELETE CASCADE,
 profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
 transaction_id TEXT NOT NULL, wallet_id TEXT NOT NULL,
 chain TEXT NOT NULL, network TEXT NOT NULL, txid TEXT NOT NULL,
 observation_json TEXT NOT NULL,
 PRIMARY KEY(snapshot_id, transaction_id)
);
CREATE TABLE filed_report_chain_impacts (
 id TEXT PRIMARY KEY,
 snapshot_id TEXT NOT NULL REFERENCES filed_report_snapshots(id) ON DELETE CASCADE,
 profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
 transaction_id TEXT NOT NULL,
 inbox_id TEXT NOT NULL UNIQUE,
 created_at TEXT NOT NULL
);
CREATE TABLE filed_report_chain_resolutions (
 impact_id TEXT PRIMARY KEY REFERENCES filed_report_chain_impacts(id) ON DELETE CASCADE,
 rebuilt_at TEXT NOT NULL, summary_json TEXT NOT NULL, created_at TEXT NOT NULL
);
CREATE TABLE filed_report_snapshots (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    report_kind TEXT NOT NULL,
    report_state TEXT NOT NULL CHECK(report_state IN ('saved', 'filed')),
    period_start_year INTEGER NOT NULL CHECK(period_start_year BETWEEN 1900 AND 9999),
    period_end_year INTEGER NOT NULL CHECK(period_end_year BETWEEN period_start_year AND 9999),
    content_sha256 TEXT NOT NULL CHECK(length(content_sha256) = 64),
    classification_summary_json TEXT NOT NULL DEFAULT '{}',
    gain_summary_json TEXT NOT NULL DEFAULT '{}',
    report_scope_json TEXT NOT NULL DEFAULT '{}',
    authored_source TEXT NOT NULL DEFAULT 'user'
        CHECK(authored_source IN ('user', 'cli', 'gui', 'ai_tool')),
    notes TEXT,
    created_at TEXT NOT NULL
);
CREATE TABLE freshness_jobs (
    id TEXT PRIMARY KEY,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    job_type TEXT NOT NULL,
    source_key TEXT NOT NULL,
    source_type TEXT NOT NULL,
    source_label TEXT NOT NULL,
    status TEXT NOT NULL,
    phase TEXT,
    priority INTEGER NOT NULL DEFAULT 100,
    payload_json TEXT NOT NULL DEFAULT '{}',
    progress_json TEXT NOT NULL DEFAULT '{}',
    result_json TEXT NOT NULL DEFAULT '{}',
    error_json TEXT NOT NULL DEFAULT '{}',
    attempts INTEGER NOT NULL DEFAULT 0,
    cancel_requested INTEGER NOT NULL DEFAULT 0,
    run_after TEXT,
    cooldown_until TEXT,
    created_at TEXT NOT NULL,
    started_at TEXT,
    finished_at TEXT,
    updated_at TEXT NOT NULL
);
CREATE TABLE freshness_source_states (
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    source_key TEXT NOT NULL,
    source_type TEXT NOT NULL,
    source_label TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'fresh',
    state TEXT NOT NULL DEFAULT 'fresh',
    stale_reason TEXT,
    blocking_reports INTEGER NOT NULL DEFAULT 0,
    paused INTEGER NOT NULL DEFAULT 0,
    rate_limited_until TEXT,
    cooldown_reason TEXT,
    retry_count INTEGER NOT NULL DEFAULT 0,
    last_success_at TEXT,
    last_error_at TEXT,
    last_error_code TEXT,
    last_error_message TEXT,
    last_phase TEXT,
    progress_json TEXT NOT NULL DEFAULT '{}',
    checkpoint_json TEXT NOT NULL DEFAULT '{}',
    updated_at TEXT NOT NULL,
    PRIMARY KEY(profile_id, source_key)
);
CREATE TABLE import_batch_transactions (
    batch_id TEXT NOT NULL REFERENCES import_batches(id) ON DELETE CASCADE,
    transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    PRIMARY KEY (batch_id, transaction_id)
);
CREATE TABLE import_batches (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    wallet_id TEXT REFERENCES wallets(id) ON DELETE CASCADE,
    source_format TEXT NOT NULL,
    source_filename TEXT,
    column_map_json TEXT,
    imported_at TEXT NOT NULL,
    rows_inserted INTEGER NOT NULL DEFAULT 0,
    rows_updated INTEGER NOT NULL DEFAULT 0,
    rows_skipped INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE journal_account_holdings (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    account_id TEXT REFERENCES accounts(id) ON DELETE SET NULL,
    account_code TEXT,
    account_label TEXT,
    asset TEXT NOT NULL,
    quantity INTEGER NOT NULL,
    cost_basis REAL NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);
CREATE TABLE journal_custody_decisions (
    decision_id TEXT NOT NULL CHECK(length(decision_id) = 64),
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    source_transaction_id TEXT NOT NULL
        REFERENCES transactions(id) ON DELETE CASCADE,
    target_transaction_id TEXT NOT NULL
        REFERENCES transactions(id) ON DELETE CASCADE,
    source_observation_hash TEXT NOT NULL
        CHECK(length(source_observation_hash) = 64),
    source_start_msat INTEGER NOT NULL
        CHECK(typeof(source_start_msat) = 'integer' AND source_start_msat >= 0),
    source_end_msat INTEGER NOT NULL
        CHECK(
            typeof(source_end_msat) = 'integer'
            AND source_end_msat > source_start_msat
        ),
    target_observation_hash TEXT NOT NULL
        CHECK(length(target_observation_hash) = 64),
    target_start_msat INTEGER NOT NULL
        CHECK(typeof(target_start_msat) = 'integer' AND target_start_msat >= 0),
    target_end_msat INTEGER NOT NULL
        CHECK(
            typeof(target_end_msat) = 'integer'
            AND target_end_msat > target_start_msat
        ),
    source_wallet_id TEXT REFERENCES wallets(id) ON DELETE SET NULL,
    target_wallet_id TEXT REFERENCES wallets(id) ON DELETE SET NULL,
    source_network TEXT NOT NULL,
    target_network TEXT NOT NULL,
    source_rail TEXT NOT NULL,
    target_rail TEXT NOT NULL,
    source_asset TEXT NOT NULL,
    target_asset TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN (
        'internal_verified', 'internal_reviewed'
    )),
    basis_state TEXT NOT NULL CHECK(basis_state IN (
        'eligible', 'blocked_by_prior_custody_basis'
    )),
    basis_barrier_at TEXT,
    reason TEXT NOT NULL,
    atomic_group_id TEXT,
    component_id TEXT,
    occurred_at TEXT,
    target_occurred_at TEXT,
    created_at TEXT NOT NULL,
    CHECK(
        source_end_msat - source_start_msat =
        target_end_msat - target_start_msat
    ),
    PRIMARY KEY(profile_id, decision_id)
);
CREATE TABLE journal_custody_economic_relations (
    relation_id TEXT NOT NULL CHECK(length(relation_id) = 64),
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    relation_kind TEXT NOT NULL CHECK(relation_kind IN (
        'conversion', 'direct_payout'
    )),
    source_transaction_id TEXT NOT NULL
        REFERENCES transactions(id) ON DELETE CASCADE,
    target_transaction_id TEXT
        REFERENCES transactions(id) ON DELETE CASCADE,
    component_id TEXT,
    source_asset TEXT NOT NULL,
    target_asset TEXT NOT NULL,
    source_amount_msat INTEGER NOT NULL CHECK(source_amount_msat > 0),
    target_amount_msat INTEGER NOT NULL CHECK(target_amount_msat > 0),
    basis_state TEXT NOT NULL CHECK(basis_state IN (
        'eligible', 'blocked_by_prior_custody_basis'
    )),
    occurred_at TEXT,
    target_occurred_at TEXT,
    created_at TEXT NOT NULL,
    PRIMARY KEY(profile_id, relation_id)
);
CREATE TABLE journal_custody_gap_inputs (
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT PRIMARY KEY REFERENCES profiles(id) ON DELETE CASCADE,
    input_version INTEGER NOT NULL CHECK(input_version >= 0),
    ignored_ids_json TEXT NOT NULL DEFAULT '[]',
    accounting_ignored_ids_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL
);
CREATE TABLE journal_custody_reconciliations (
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    component_id TEXT NOT NULL REFERENCES custody_components(id) ON DELETE CASCADE,
    input_version INTEGER NOT NULL,
    component_revision INTEGER NOT NULL,
    proof_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY(profile_id, component_id)
);
CREATE TABLE journal_entries (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    wallet_id TEXT NOT NULL REFERENCES wallets(id) ON DELETE CASCADE,
    account_id TEXT REFERENCES accounts(id) ON DELETE SET NULL,
    occurred_at TEXT NOT NULL,
    entry_type TEXT NOT NULL,
    asset TEXT NOT NULL,
    quantity INTEGER NOT NULL,
    fiat_value REAL NOT NULL DEFAULT 0,
    unit_cost REAL NOT NULL DEFAULT 0,
    cost_basis REAL,
    proceeds REAL,
    gain_loss REAL,
    fiat_value_exact TEXT,
    unit_cost_exact TEXT,
    cost_basis_exact TEXT,
    proceeds_exact TEXT,
    gain_loss_exact TEXT,
    calculation_json TEXT,
    calculation_compact TEXT,
    pricing_source_kind TEXT,
    pricing_quality TEXT,
    description TEXT,
    at_category TEXT,
    at_kennzahl INTEGER,
    capital_gains_type TEXT,
    created_at TEXT NOT NULL
);
CREATE TABLE journal_quantity_balances (
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    location_kind TEXT NOT NULL,
    location_id TEXT NOT NULL,
    asset TEXT NOT NULL,
    amount_msat INTEGER NOT NULL CHECK(amount_msat != 0),
    created_at TEXT NOT NULL,
    PRIMARY KEY(profile_id, location_kind, location_id, asset)
);
CREATE TABLE journal_quantity_issues (
    issue_id TEXT NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    issue_type TEXT NOT NULL,
    -- custody_candidate is accepted only so a pre-simplification derived
    -- journal can be opened and rebuilt; current production emitters use
    -- custody_suspense/conflicting and replacement removes the legacy rows.
    state TEXT NOT NULL CHECK(state IN (
        'custody_candidate', 'custody_suspense', 'conflicting'
    )),
    asset TEXT,
    amount_msat INTEGER CHECK(amount_msat IS NULL OR amount_msat > 0),
    occurred_at TEXT,
    transaction_ids_json TEXT NOT NULL DEFAULT '[]',
    reason TEXT NOT NULL,
    detail_json TEXT NOT NULL DEFAULT '{}',
    blocks_from TEXT,
    created_at TEXT NOT NULL,
    PRIMARY KEY(profile_id, issue_id)
);
CREATE TABLE journal_quantity_postings (
    posting_id TEXT NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    transaction_id TEXT REFERENCES transactions(id) ON DELETE SET NULL,
    observation_hash TEXT,
    occurred_at TEXT,
    asset TEXT NOT NULL,
    location_kind TEXT NOT NULL,
    location_id TEXT NOT NULL,
    amount_msat INTEGER NOT NULL CHECK(amount_msat != 0),
    state TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY(profile_id, posting_id)
);
CREATE TABLE journal_quarantines (
    transaction_id TEXT PRIMARY KEY REFERENCES transactions(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    reason TEXT NOT NULL,
    detail_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL
);
CREATE TABLE journal_tax_summary (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    year INTEGER NOT NULL,
    asset TEXT NOT NULL,
    transaction_type TEXT NOT NULL,
    capital_gains_type TEXT,
    quantity INTEGER NOT NULL,
    proceeds REAL NOT NULL DEFAULT 0,
    cost_basis REAL NOT NULL DEFAULT 0,
    gain_loss REAL NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);
CREATE TABLE journal_wallet_holdings (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    wallet_id TEXT REFERENCES wallets(id) ON DELETE CASCADE,
    wallet_label TEXT,
    account_code TEXT,
    asset TEXT NOT NULL,
    quantity INTEGER NOT NULL,
    cost_basis REAL NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);
CREATE TABLE lightning_node_records (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    wallet_id TEXT NOT NULL REFERENCES wallets(id) ON DELETE CASCADE,
    backend_name TEXT NOT NULL,
    node_id TEXT,
    record_type TEXT NOT NULL,
    external_id TEXT NOT NULL,
    occurred_at TEXT NOT NULL,
    account TEXT,
    peer_id TEXT,
    channel_id TEXT,
    direction TEXT,
    amount_msat INTEGER NOT NULL DEFAULT 0,
    fee_msat INTEGER NOT NULL DEFAULT 0,
    tag TEXT,
    status TEXT,
    currency TEXT,
    payment_hash TEXT,
    txid TEXT,
    outpoint TEXT,
    sync_id TEXT REFERENCES lightning_node_syncs(id) ON DELETE SET NULL,
    raw_json TEXT NOT NULL DEFAULT '{}',
    first_seen_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(profile_id, wallet_id, backend_name, record_type, external_id)
);
CREATE TABLE lightning_node_syncs (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    wallet_id TEXT NOT NULL REFERENCES wallets(id) ON DELETE CASCADE,
    backend_name TEXT NOT NULL,
    node_id TEXT,
    node_alias TEXT,
    started_at TEXT NOT NULL,
    completed_at TEXT,
    status TEXT NOT NULL,
    fetched_counts_json TEXT NOT NULL DEFAULT '{}',
    error_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL
);
CREATE TABLE loan_legs (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    loan_id TEXT,
    role TEXT NOT NULL,
    note TEXT,
    deleted_at TEXT,
    created_at TEXT NOT NULL
);
CREATE TABLE profiles (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    label TEXT NOT NULL,
    fiat_currency TEXT NOT NULL DEFAULT 'USD',
    tax_country TEXT NOT NULL DEFAULT 'generic',
    tax_long_term_days INTEGER NOT NULL DEFAULT 365,
    gains_algorithm TEXT NOT NULL DEFAULT 'FIFO',
    require_coarse_review INTEGER NOT NULL DEFAULT 0,
    bitcoin_rail_carrying_value INTEGER NOT NULL DEFAULT 1,
    cost_basis_pool_scope TEXT NOT NULL DEFAULT 'global',
    journal_input_version INTEGER NOT NULL DEFAULT 0,
    last_processed_input_version INTEGER NOT NULL DEFAULT 0,
    required_journal_semantics_version INTEGER NOT NULL DEFAULT 0,
    journal_semantics_version INTEGER NOT NULL DEFAULT 0,
    last_processed_at TEXT,
    last_processed_tx_count INTEGER NOT NULL DEFAULT 0,
    ownership_review_counts_json TEXT,
    created_at TEXT NOT NULL, swap_candidate_count INTEGER,
    UNIQUE (workspace_id, label)
);
CREATE TABLE rates_cache (
    pair TEXT NOT NULL,
    timestamp TEXT NOT NULL,
    rate REAL NOT NULL,
    rate_exact TEXT,
    source TEXT NOT NULL,
    fetched_at TEXT NOT NULL,
    granularity TEXT,
    method TEXT,
    open_rate REAL,
    open_rate_exact TEXT,
    high_rate REAL,
    high_rate_exact TEXT,
    low_rate REAL,
    low_rate_exact TEXT,
    close_rate REAL,
    close_rate_exact TEXT,
    volume REAL,
    volume_exact TEXT,
    trades INTEGER,
    PRIMARY KEY (pair, timestamp, source)
);
CREATE TABLE rates_checked_minutes (
    pair TEXT NOT NULL,
    timestamp TEXT NOT NULL,
    source TEXT NOT NULL,
    checked_at TEXT NOT NULL,
    granularity TEXT,
    method TEXT,
    PRIMARY KEY (pair, timestamp, source)
);
CREATE TABLE review_workflow_receipts (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    idempotency_key TEXT NOT NULL,
    artifact_digest TEXT NOT NULL CHECK(length(artifact_digest) = 64),
    receipt_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE(profile_id, idempotency_key)
);
CREATE TABLE saved_views (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    surface TEXT NOT NULL,
    name TEXT NOT NULL,
    filter_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (profile_id, surface, name)
);
CREATE TABLE schema_migration_audits (
    id TEXT PRIMARY KEY,
    migration_name TEXT NOT NULL UNIQUE,
    schema_version INTEGER NOT NULL CHECK(schema_version >= 1),
    impact_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);
INSERT INTO "schema_migration_audits" VALUES('journal-calculation-semantics-adoption-v1','journal-calculation-semantics-adoption-v1',1,'{"affected_profile_count":0,"explanation":"Requires stamped calculation semantics for previously processed books; retained journals and input markers are unchanged.","migration":"journal-calculation-semantics-adoption-v1","required_journal_semantics_version":1,"schema_version":1}','2026-10-31T20:55:43.771Z');
INSERT INTO "schema_migration_audits" VALUES('wages-acquisition-semantics-v1','wages-acquisition-semantics-v1',1,'{"changes":[{"affected_profile_count":0,"affected_transaction_count":0,"after":{"journal_treatment":"basis_bearing_acquisition"},"before":{"journal_treatment":"employment_income"},"explanation":"Marks wage-bearing journal projections stale so they are rebuilt without employment-income or Kennzahl 172 rows.","name":"wages_acquisition_semantics"}],"migration":"wages-acquisition-semantics-v1","schema_version":1}','2026-10-31T20:55:43.771Z');
INSERT INTO "schema_migration_audits" VALUES('rp2-pool-basis-semantics-v1','rp2-pool-basis-semantics-v1',1,'{"changes":[{"affected_profile_count":0,"explanation":"Marks previously processed pool journals stale to rebuild holdings with report-cutoff basis; retained journals are unchanged.","name":"report_cutoff_pool_basis"}],"migration":"rp2-pool-basis-semantics-v1","schema_version":1}','2026-10-31T20:55:43.771Z');
INSERT INTO "schema_migration_audits" VALUES('custody-fail-closed-holds-v1','custody-fail-closed-holds-v1',1,'{"changes":[{"affected_profile_count":0,"explanation":"Marks processed journals stale so unscoped, fan-out, pending and replaced spends are held on the next rebuild; retained journals are unchanged until then.","name":"pre_arbitration_custody_holds"}],"migration":"custody-fail-closed-holds-v1","schema_version":1}','2026-10-31T20:55:43.771Z');
INSERT INTO "schema_migration_audits" VALUES('kassiber-tax-engine-v1','kassiber-tax-engine-v1',1,'{"changes":[{"affected_profile_count":0,"explanation":"Marks processed journals stale so the next rebuild uses Kassiber''s own tax engine at one fixed decimal precision; retained journals are unchanged until then.","name":"kassiber_tax_engine"}],"migration":"kassiber-tax-engine-v1","schema_version":1}','2026-10-31T20:55:43.771Z');
INSERT INTO "schema_migration_audits" VALUES('at-outflow-consumed-regime-v2','at-outflow-consumed-regime-v2',1,'{"affected_profile_count": 0, "migration": "at-outflow-consumed-regime-v2", "required_journal_semantics_version": 2, "schema_version": 1}','2026-10-31T20:55:43.771Z');
INSERT INTO "schema_migration_audits" VALUES('at-acquisition-valuation-support-v1','at-acquisition-valuation-support-v1',1,'{"schema_version": 1, "affected_profile_count": 0, "explanation": "Rebuild Austrian airdrop/hardfork journals with the explicit unsupported-valuation barrier."}','2026-10-31T20:55:43.771Z');
INSERT INTO "schema_migration_audits" VALUES('custody-partial-payout-pricing-v1','custody-partial-payout-pricing-v1',1,'{"affected_profile_count": 0, "migration": "custody-partial-payout-pricing-v1", "schema_version": 1}','2026-10-31T20:55:43.771Z');
CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
INSERT INTO "settings" VALUES('database_instance_id','6d14035458564a82af69e7b282ec3847');
CREATE TABLE source_funds_cases (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    target_transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    target_external_id TEXT,
    target_amount INTEGER NOT NULL,
    asset TEXT NOT NULL,
    label TEXT,
    reveal_mode TEXT NOT NULL,
    status TEXT NOT NULL,
    snapshot_hash TEXT NOT NULL,
    snapshot_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
, recipient_id TEXT, recipient_label_snapshot TEXT, recipient_kind_snapshot TEXT, recipient_reveal_mode_snapshot TEXT);
CREATE TABLE source_funds_link_attachments (
    link_id TEXT NOT NULL REFERENCES source_funds_links(id) ON DELETE CASCADE,
    attachment_id TEXT NOT NULL REFERENCES attachments(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    PRIMARY KEY (link_id, attachment_id)
);
CREATE TABLE source_funds_links (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    from_source_id TEXT REFERENCES source_funds_sources(id) ON DELETE CASCADE,
    from_transaction_id TEXT REFERENCES transactions(id) ON DELETE CASCADE,
    to_transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    link_type TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'suggested',
    confidence TEXT NOT NULL DEFAULT 'unknown',
    method TEXT NOT NULL DEFAULT 'manual',
    asset TEXT NOT NULL,
    allocation_amount INTEGER,
    from_asset TEXT,
    from_allocation_amount INTEGER,
    allocation_policy TEXT NOT NULL DEFAULT 'unknown',
    explanation TEXT,
    uses_chain_observation INTEGER NOT NULL DEFAULT 0,
    chain_data_confirmed INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    CHECK (
        (from_source_id IS NOT NULL AND from_transaction_id IS NULL)
        OR (from_source_id IS NULL AND from_transaction_id IS NOT NULL)
    )
);
CREATE TABLE source_funds_recipients (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    label TEXT NOT NULL,
    kind TEXT NOT NULL,
    default_reveal_mode TEXT NOT NULL DEFAULT 'standard',
    notes TEXT,
    active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE source_funds_snapshots (
    id TEXT PRIMARY KEY,
    case_id TEXT NOT NULL REFERENCES source_funds_cases(id) ON DELETE CASCADE,
    snapshot_hash TEXT NOT NULL,
    snapshot_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE source_funds_source_attachments (
    source_id TEXT NOT NULL REFERENCES source_funds_sources(id) ON DELETE CASCADE,
    attachment_id TEXT NOT NULL REFERENCES attachments(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    PRIMARY KEY (source_id, attachment_id)
);
CREATE TABLE source_funds_sources (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    source_type TEXT NOT NULL,
    label TEXT NOT NULL,
    asset TEXT NOT NULL,
    amount INTEGER,
    fiat_currency TEXT,
    fiat_value REAL,
    acquired_at TEXT,
    description TEXT,
    review_state TEXT NOT NULL DEFAULT 'reviewed',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE swap_matching_rules (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    name TEXT,
    predicate_json TEXT NOT NULL DEFAULT '{}',
    kind TEXT NOT NULL DEFAULT 'manual',
    policy TEXT NOT NULL DEFAULT 'carrying-value',
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE tags (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    code TEXT NOT NULL,
    label TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE (profile_id, code)
);
CREATE TABLE transaction_edit_events (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    wallet_id TEXT REFERENCES wallets(id) ON DELETE SET NULL,
    transaction_external_id TEXT,
    transaction_occurred_at TEXT,
    source TEXT NOT NULL,
    reason TEXT,
    changed_at TEXT NOT NULL,
    journal_input_version INTEGER NOT NULL DEFAULT 0,
    journal_input_version_after INTEGER NOT NULL DEFAULT 0,
    last_processed_input_version INTEGER NOT NULL DEFAULT 0,
    last_processed_at TEXT,
    last_processed_tx_count INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE transaction_edit_fields (
    id TEXT PRIMARY KEY,
    event_id TEXT NOT NULL REFERENCES transaction_edit_events(id) ON DELETE CASCADE,
    field TEXT NOT NULL,
    before_value TEXT,
    after_value TEXT,
    diff_json TEXT NOT NULL DEFAULT '{}'
);
CREATE TABLE transaction_graph_cache (
    schema_version INTEGER NOT NULL,
    chain TEXT NOT NULL,
    network TEXT NOT NULL,
    txid TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (schema_version, chain, network, txid)
);
CREATE TABLE transaction_pair_dismissals (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    out_transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    in_transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    reason TEXT,
    created_at TEXT NOT NULL,
    expires_at TEXT,
    UNIQUE (profile_id, out_transaction_id, in_transaction_id)
);
CREATE TABLE transaction_pairs (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    out_transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    in_transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    kind TEXT NOT NULL DEFAULT 'manual',
    policy TEXT NOT NULL DEFAULT 'carrying-value',
    notes TEXT,
    swap_fee_msat INTEGER,
    swap_fee_kind TEXT,
    confidence_at_pair TEXT,
    pair_source TEXT,
    out_amount INTEGER,
    component_id TEXT REFERENCES custody_components(id) ON DELETE SET NULL,
    deleted_at TEXT,
    created_at TEXT NOT NULL
);
CREATE TABLE transaction_tags (
    transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    tag_id TEXT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (transaction_id, tag_id)
);
CREATE TABLE transactions (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    wallet_id TEXT NOT NULL REFERENCES wallets(id) ON DELETE CASCADE,
    external_id TEXT,
    -- Closed, public discriminator for the meaning of external_id, so native
    -- chain identity survives projections that omit raw_json.
    external_id_kind TEXT CHECK(external_id_kind IS NULL OR external_id_kind = 'txid'),
    fingerprint TEXT NOT NULL UNIQUE,
    occurred_at TEXT NOT NULL,
    confirmed_at TEXT,
    direction TEXT NOT NULL,
    asset TEXT NOT NULL,
    amount INTEGER NOT NULL,
    fee INTEGER NOT NULL DEFAULT 0,
    -- 1 when `amount` is a net wallet-balance delta with the network fee folded
    -- in and no separate fee is available (BTCPay Greenfield sync). 0 (the
    -- default) means `amount` is recipient-only and `fee` carries the miner fee
    -- (esplora/electrum/bitcoinrpc and every CSV importer).
    amount_includes_fee INTEGER NOT NULL DEFAULT 0,
    fiat_currency TEXT,
    fiat_rate REAL,
    fiat_value REAL,
    fiat_price_source TEXT,
    fiat_rate_exact TEXT,
    fiat_value_exact TEXT,
    pricing_source_kind TEXT,
    pricing_provider TEXT,
    pricing_pair TEXT,
    pricing_timestamp TEXT,
    pricing_fetched_at TEXT,
    pricing_granularity TEXT,
    pricing_method TEXT,
    pricing_external_ref TEXT,
    pricing_quality TEXT,
    commercial_applied_link_id TEXT,
    review_status TEXT,
    taxability_override INTEGER,
    at_regime_override TEXT,
    at_category_override TEXT,
    -- User-assigned tax classification. Kept separate from `kind` so the
    -- importer's provenance survives: `kind` records what the source said
    -- (and gates trust checks like the Lightning payment-hash pairing in
    -- transfers.py), while this records what the user declares it to be.
    kind_override TEXT,
    privacy_boundary TEXT,
    kind TEXT,
    description TEXT,
    counterparty TEXT,
    note TEXT,
    excluded INTEGER NOT NULL DEFAULT 0,
    raw_json TEXT NOT NULL DEFAULT '{}',
    payment_hash TEXT,
    payment_hash_source TEXT,
    swap_refund_funding_txid TEXT,
    swap_refund_funding_vout INTEGER,
    created_at TEXT NOT NULL
);
CREATE TABLE wallet_policy_coverage_witnesses (
    source_id TEXT NOT NULL REFERENCES wallet_policy_sources(id) ON DELETE CASCADE,
    branch_key TEXT NOT NULL,
    scanned_to_exclusive INTEGER NOT NULL CHECK(scanned_to_exclusive >= 0),
    highest_used INTEGER,
    observer_kind TEXT NOT NULL,
    observed_at TEXT NOT NULL,
    PRIMARY KEY(source_id, branch_key)
);
CREATE TABLE wallet_policy_epochs (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    wallet_id TEXT NOT NULL REFERENCES wallets(id) ON DELETE CASCADE,
    chain TEXT NOT NULL,
    network TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('active', 'retired')),
    private_material_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL,
    retired_at TEXT
);
CREATE TABLE wallet_policy_sources (
    id TEXT PRIMARY KEY,
    epoch_id TEXT NOT NULL REFERENCES wallet_policy_epochs(id) ON DELETE CASCADE,
    source_wallet_id TEXT NOT NULL REFERENCES wallets(id) ON DELETE CASCADE,
    source_key TEXT NOT NULL,
    observer_kind TEXT NOT NULL,
    branch_keys_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(epoch_id, source_wallet_id, source_key)
);
CREATE TABLE wallet_utxo_refreshes (
    wallet_id TEXT PRIMARY KEY REFERENCES wallets(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    backend_name TEXT,
    backend_kind TEXT,
    chain TEXT NOT NULL,
    network TEXT NOT NULL,
    observed_count INTEGER NOT NULL DEFAULT 0,
    active_count INTEGER NOT NULL DEFAULT 0,
    last_seen_at TEXT NOT NULL
);
CREATE TABLE wallet_utxos (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    wallet_id TEXT NOT NULL REFERENCES wallets(id) ON DELETE CASCADE,
    backend_name TEXT,
    backend_kind TEXT,
    chain TEXT NOT NULL,
    network TEXT NOT NULL,
    asset TEXT NOT NULL,
    amount INTEGER NOT NULL,
    txid TEXT NOT NULL,
    vout INTEGER NOT NULL,
    outpoint TEXT NOT NULL,
    confirmation_status TEXT NOT NULL,
    confirmations INTEGER,
    block_height INTEGER,
    block_time TEXT,
    address TEXT,
    script_pubkey TEXT,
    address_label TEXT,
    branch_label TEXT,
    branch_index INTEGER,
    address_index INTEGER,
    anonymity_score INTEGER,
    spent_by TEXT,
    excluded_from_coinjoin INTEGER,
    key_state TEXT,
    anon_history_json TEXT NOT NULL DEFAULT '[]',
    first_seen_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    spent_at TEXT,
    raw_json TEXT NOT NULL DEFAULT '{}',
    UNIQUE (wallet_id, txid, vout)
);
CREATE TABLE wallets (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    account_id TEXT REFERENCES accounts(id) ON DELETE SET NULL,
    label TEXT NOT NULL,
    kind TEXT NOT NULL,
    config_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL,
    UNIQUE (profile_id, label)
);
CREATE TABLE workspaces (
    id TEXT PRIMARY KEY,
    label TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL
);
CREATE TRIGGER trg_schema_migration_audits_immutable
BEFORE UPDATE ON schema_migration_audits
BEGIN
    SELECT RAISE(ABORT, 'schema_migration_audits_immutable');
END;
CREATE TRIGGER trg_book_network_binding_immutable
BEFORE UPDATE ON book_network_bindings
BEGIN SELECT RAISE(ABORT, 'book_network_binding_immutable'); END;
CREATE INDEX idx_chain_observer_instances_profile
    ON chain_observer_instances(profile_id, logical_wallet_id, source_wallet_id);
CREATE UNIQUE INDEX idx_wallet_policy_epochs_one_active
    ON wallet_policy_epochs(wallet_id)
    WHERE status = 'active';
CREATE INDEX idx_wallet_policy_epochs_profile
    ON wallet_policy_epochs(profile_id, wallet_id, status, created_at);
CREATE INDEX idx_chain_observation_provenance_profile
    ON chain_observation_provenance(profile_id, wallet_id, chain, network);
CREATE INDEX idx_transaction_graph_cache_updated
    ON transaction_graph_cache(updated_at DESC);
CREATE INDEX idx_chain_analysis_cases_profile
    ON chain_analysis_cases(profile_id, created_at DESC, id DESC);
CREATE INDEX idx_chain_analysis_labels_profile
    ON chain_analysis_labels(profile_id, deleted, chain, network, subject);
CREATE UNIQUE INDEX idx_chain_analysis_dataset_active
    ON chain_analysis_datasets(profile_id, dataset_key) WHERE status = 'active';
CREATE INDEX idx_chain_analysis_dataset_profile
    ON chain_analysis_datasets(profile_id, created_at DESC, id DESC);
CREATE INDEX idx_chain_analysis_dataset_subject
    ON chain_analysis_dataset_claims(dataset_id, subject, record_number);
CREATE INDEX idx_chain_analysis_dataset_entity
    ON chain_analysis_dataset_claims(dataset_id, label, record_number);
CREATE INDEX idx_import_batches_profile_time
    ON import_batches(profile_id, imported_at DESC, id);
CREATE INDEX idx_transactions_profile_external_id
    ON transactions(profile_id, external_id) WHERE external_id IS NOT NULL;
CREATE INDEX idx_transactions_profile_page ON transactions(profile_id, occurred_at, created_at, id, wallet_id);
CREATE INDEX idx_transactions_profile_active_time
    ON transactions(profile_id, excluded, occurred_at, created_at, id);
CREATE INDEX idx_transactions_wallet_external_match
    ON transactions(wallet_id, external_id, direction, asset, amount, fee, created_at)
    WHERE external_id IS NOT NULL;
CREATE INDEX idx_transactions_wallet_external_ci_match
    ON transactions(wallet_id, LOWER(external_id), direction, asset)
    WHERE external_id IS NOT NULL;
CREATE INDEX idx_transactions_profile_economic_match
    ON transactions(profile_id, direction, asset, amount, occurred_at, created_at);
CREATE INDEX idx_wallet_utxos_wallet_active
    ON wallet_utxos(wallet_id, spent_at, asset, block_height, txid, vout);
CREATE INDEX idx_wallet_utxos_profile_wallet
    ON wallet_utxos(profile_id, wallet_id, asset);
CREATE INDEX idx_wallet_utxo_refreshes_profile
    ON wallet_utxo_refreshes(profile_id, wallet_id);
CREATE INDEX idx_transaction_edit_events_profile_changed
    ON transaction_edit_events(profile_id, changed_at DESC, id DESC);
CREATE INDEX idx_transaction_edit_events_transaction_changed
    ON transaction_edit_events(transaction_id, changed_at DESC, id DESC);
CREATE INDEX idx_transaction_edit_events_source_changed
    ON transaction_edit_events(profile_id, source, changed_at DESC);
CREATE INDEX idx_transaction_edit_events_wallet_changed
    ON transaction_edit_events(profile_id, wallet_id, changed_at DESC);
CREATE INDEX idx_transaction_edit_fields_event
    ON transaction_edit_fields(event_id);
CREATE INDEX idx_transaction_edit_fields_field
    ON transaction_edit_fields(field, event_id);
CREATE TRIGGER trg_custody_authored_evidence_immutable
BEFORE UPDATE ON custody_authored_evidence_snapshots
BEGIN
    SELECT RAISE(ABORT, 'custody_authored_evidence_immutable');
END;
CREATE INDEX idx_journal_entries_profile_time
    ON journal_entries(profile_id, occurred_at, created_at, id);
CREATE INDEX idx_journal_entries_profile_type_time
    ON journal_entries(profile_id, entry_type, occurred_at, created_at, id);
CREATE INDEX idx_journal_entries_profile_wallet_time
    ON journal_entries(profile_id, wallet_id, occurred_at, created_at, id);
CREATE INDEX idx_journal_entries_profile_account_time
    ON journal_entries(profile_id, account_id, occurred_at, created_at, id);
CREATE INDEX idx_journal_entries_transaction
    ON journal_entries(transaction_id);
CREATE INDEX idx_journal_quarantines_profile
    ON journal_quarantines(profile_id, created_at);
CREATE INDEX idx_journal_tax_summary_profile_year
    ON journal_tax_summary(profile_id, year, asset, transaction_type, capital_gains_type);
CREATE INDEX idx_journal_account_holdings_profile_asset
    ON journal_account_holdings(profile_id, asset, account_code, id);
CREATE INDEX idx_journal_wallet_holdings_profile_asset
    ON journal_wallet_holdings(profile_id, asset, wallet_label, id);
CREATE INDEX idx_journal_quantity_postings_profile_time
    ON journal_quantity_postings(profile_id, occurred_at, posting_id);
CREATE INDEX idx_journal_quantity_issues_profile_time
    ON journal_quantity_issues(profile_id, occurred_at, issue_id);
CREATE INDEX idx_journal_quantity_balances_profile_asset
    ON journal_quantity_balances(profile_id, asset, location_kind, location_id);
CREATE INDEX idx_journal_custody_decisions_profile_time
    ON journal_custody_decisions(profile_id, occurred_at, decision_id);
CREATE INDEX idx_journal_custody_decisions_source
    ON journal_custody_decisions(profile_id, source_transaction_id, decision_id);
CREATE INDEX idx_journal_custody_decisions_target
    ON journal_custody_decisions(profile_id, target_transaction_id, decision_id);
CREATE INDEX idx_journal_custody_relations_profile_time
    ON journal_custody_economic_relations(
        profile_id, occurred_at, relation_id
    );
CREATE INDEX idx_journal_custody_relations_source
    ON journal_custody_economic_relations(
        profile_id, source_transaction_id, relation_id
    );
CREATE INDEX idx_journal_custody_relations_target
    ON journal_custody_economic_relations(
        profile_id, target_transaction_id, relation_id
    );
CREATE INDEX idx_custody_authored_evidence_subject
    ON custody_authored_evidence_snapshots(
        profile_id, subject_kind, subject_id, created_at
    );
CREATE INDEX idx_filed_report_snapshots_period
    ON filed_report_snapshots(
        profile_id, period_start_year, period_end_year, created_at, id
    );
CREATE TRIGGER trg_filed_report_snapshots_immutable
BEFORE UPDATE ON filed_report_snapshots
BEGIN
    SELECT RAISE(ABORT, 'filed_report_snapshots_immutable');
END;
CREATE INDEX idx_custody_gap_reviews_latest
    ON custody_gap_reviews(profile_id, gap_id, revision DESC);
CREATE TRIGGER trg_custody_gap_reviews_immutable
BEFORE UPDATE ON custody_gap_reviews
BEGIN
    SELECT RAISE(ABORT, 'custody_gap_reviews_immutable');
END;
CREATE INDEX idx_custody_gap_review_relation_sets_scope
    ON custody_gap_review_relation_sets(profile_id, review_id, id);
CREATE TRIGGER trg_custody_gap_review_relation_set_scope_insert
BEFORE INSERT ON custody_gap_review_relation_sets
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1 FROM custody_gap_reviews r
        WHERE r.id = NEW.review_id
          AND r.workspace_id = NEW.workspace_id
          AND r.profile_id = NEW.profile_id
    ) THEN RAISE(ABORT, 'custody_gap_review_relation_set_scope_mismatch') END;
END;
CREATE TRIGGER trg_custody_gap_review_relation_sets_immutable
BEFORE UPDATE ON custody_gap_review_relation_sets
BEGIN
    SELECT RAISE(ABORT, 'custody_gap_review_relation_sets_immutable');
END;
CREATE INDEX idx_custody_gap_review_transactions_review
    ON custody_gap_review_transactions(review_id, role, transaction_id, id);
CREATE INDEX idx_custody_gap_review_transactions_scope
    ON custody_gap_review_transactions(profile_id, transaction_id, review_id);
CREATE TRIGGER trg_custody_gap_review_transaction_scope_insert
BEFORE INSERT ON custody_gap_review_transactions
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1 FROM custody_gap_reviews r
        WHERE r.id = NEW.review_id
          AND r.workspace_id = NEW.workspace_id
          AND r.profile_id = NEW.profile_id
    ) THEN RAISE(ABORT, 'custody_gap_review_transaction_review_scope_mismatch') END;
    -- A durable review anchor must survive source retraction and must remain
    -- importable into a fresh book after that retraction. Reject only a
    -- colliding live transaction from another book; absence is intentional.
    SELECT CASE WHEN EXISTS (
        SELECT 1 FROM transactions t WHERE t.id = NEW.transaction_id
    ) AND NOT EXISTS (
        SELECT 1 FROM transactions t
        WHERE t.id = NEW.transaction_id
          AND t.workspace_id = NEW.workspace_id
          AND t.profile_id = NEW.profile_id
    ) THEN RAISE(ABORT, 'custody_gap_review_transaction_scope_mismatch') END;
END;
CREATE TRIGGER trg_custody_gap_review_transactions_immutable
BEFORE UPDATE ON custody_gap_review_transactions
BEGIN
    SELECT RAISE(ABORT, 'custody_gap_review_transactions_immutable');
END;
CREATE TRIGGER trg_review_workflow_receipt_immutable
BEFORE UPDATE ON review_workflow_receipts
BEGIN
    SELECT RAISE(ABORT, 'review_workflow_receipt_immutable');
END;
CREATE INDEX idx_custody_ai_assistance_profile_time
    ON custody_ai_assistance_audits(profile_id, created_at DESC, id DESC);
CREATE TRIGGER trg_custody_ai_assistance_immutable
BEFORE UPDATE ON custody_ai_assistance_audits
BEGIN
    SELECT RAISE(ABORT, 'custody_ai_assistance_immutable');
END;
CREATE INDEX idx_custody_filed_report_impacts_profile
    ON custody_filed_report_impacts(profile_id, created_at, id);
CREATE TRIGGER trg_custody_filed_report_impacts_immutable
BEFORE UPDATE ON custody_filed_report_impacts
BEGIN
    SELECT RAISE(ABORT, 'custody_filed_report_impacts_immutable');
END;
CREATE INDEX idx_custody_filed_report_impact_resolutions_profile
    ON custody_filed_report_impact_resolutions(profile_id, rebuilt_at, id);
CREATE TRIGGER trg_custody_filed_report_impact_resolutions_immutable
BEFORE UPDATE ON custody_filed_report_impact_resolutions
BEGIN
    SELECT RAISE(ABORT, 'custody_filed_report_impact_resolutions_immutable');
END;
CREATE INDEX idx_transaction_pair_dismissals_profile
    ON transaction_pair_dismissals(profile_id, expires_at);
CREATE UNIQUE INDEX idx_direct_swap_payouts_active_out
    ON direct_swap_payouts(profile_id, out_transaction_id) WHERE deleted_at IS NULL;
CREATE INDEX idx_direct_swap_payouts_profile_active
    ON direct_swap_payouts(profile_id) WHERE deleted_at IS NULL;
CREATE INDEX idx_custody_authored_migration_issues_profile_open
    ON custody_authored_migration_issues(profile_id, created_at, id)
    WHERE resolved_at IS NULL;
CREATE INDEX idx_loan_legs_profile_active
    ON loan_legs(profile_id) WHERE deleted_at IS NULL;
CREATE UNIQUE INDEX idx_loan_legs_active_transaction
    ON loan_legs(profile_id, transaction_id)
    WHERE deleted_at IS NULL;
CREATE INDEX idx_swap_matching_rules_profile_enabled
    ON swap_matching_rules(profile_id, enabled);
CREATE INDEX idx_saved_views_profile_surface
    ON saved_views(profile_id, surface);
CREATE INDEX idx_ai_chat_sessions_profile_updated
    ON ai_chat_sessions(profile_id, updated_at);
CREATE INDEX idx_ai_chat_messages_session
    ON ai_chat_messages(session_id, seq);
CREATE INDEX idx_lightning_node_syncs_wallet_started
    ON lightning_node_syncs(wallet_id, started_at DESC);
CREATE INDEX idx_lightning_node_records_profile_type_time
    ON lightning_node_records(profile_id, record_type, occurred_at DESC);
CREATE INDEX idx_lightning_node_records_wallet_type_time
    ON lightning_node_records(wallet_id, record_type, occurred_at DESC);
CREATE INDEX idx_freshness_source_states_profile_status
    ON freshness_source_states(profile_id, status, updated_at DESC);
CREATE INDEX idx_freshness_jobs_profile_status
    ON freshness_jobs(profile_id, status, priority, created_at);
CREATE UNIQUE INDEX idx_freshness_jobs_singleflight
    ON freshness_jobs(profile_id, source_key, job_type)
    WHERE status IN ('queued', 'running', 'rate_limited');
CREATE INDEX idx_rates_cache_pair_ts
    ON rates_cache(pair, timestamp DESC);
CREATE INDEX idx_rates_checked_minutes_pair_ts
    ON rates_checked_minutes(pair, timestamp DESC);
CREATE INDEX idx_attachments_profile_tx_created
    ON attachments(profile_id, transaction_id, created_at DESC);
CREATE INDEX idx_btcpay_provenance_profile_invoice
    ON btcpay_provenance_records(profile_id, invoice_id, record_type);
CREATE INDEX idx_btcpay_provenance_profile_txid
    ON btcpay_provenance_records(profile_id, txid) WHERE txid IS NOT NULL;
CREATE INDEX idx_btcpay_account_routes_profile_backend
    ON btcpay_account_routes(profile_id, backend_name);
CREATE INDEX idx_external_documents_profile_ref
    ON external_documents(profile_id, external_ref);
CREATE INDEX idx_commercial_links_profile_state
    ON commercial_links(profile_id, state, reconciliation_state);
CREATE UNIQUE INDEX idx_external_documents_profile_external_ref_unique
    ON external_documents(profile_id, external_ref)
    WHERE external_ref IS NOT NULL AND external_ref != '';
CREATE INDEX idx_source_funds_sources_profile_type
    ON source_funds_sources(profile_id, source_type, created_at DESC);
CREATE INDEX idx_source_funds_links_profile_to
    ON source_funds_links(profile_id, to_transaction_id, state);
CREATE INDEX idx_source_funds_links_profile_from_tx
    ON source_funds_links(profile_id, from_transaction_id);
CREATE INDEX idx_source_funds_cases_profile_created
    ON source_funds_cases(profile_id, created_at DESC);
CREATE UNIQUE INDEX idx_source_funds_recipients_active_label
    ON source_funds_recipients(profile_id, label) WHERE active = 1;
CREATE INDEX idx_custody_components_lineage_active
    ON custody_components(profile_id, lineage_id) WHERE state = 'active';
CREATE INDEX idx_custody_components_lineage_draft
    ON custody_components(profile_id, lineage_id) WHERE state = 'draft';
CREATE INDEX idx_custody_components_lineage_revision
    ON custody_components(profile_id, lineage_id, revision, id);
CREATE INDEX idx_custody_components_profile_state
    ON custody_components(profile_id, state, created_at DESC, id DESC);
CREATE INDEX idx_custody_components_supersedes
    ON custody_components(supersedes_component_id)
    WHERE supersedes_component_id IS NOT NULL;
CREATE INDEX idx_custody_component_terms_profile_kind
    ON custody_component_economic_terms(profile_id, term_kind, created_at, component_id);
CREATE INDEX idx_custody_component_terms_component
    ON custody_component_economic_terms(component_id, ordinal, id);
CREATE INDEX idx_custody_component_terms_legacy_source
    ON custody_component_economic_terms(profile_id, term_kind, legacy_source_id,
                                        created_at, component_id);
CREATE TRIGGER trg_custody_component_terms_scope_insert
BEFORE INSERT ON custody_component_economic_terms
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1
        FROM custody_components c
        JOIN custody_component_legs source ON source.id = NEW.source_leg_id
        JOIN custody_component_legs target ON target.id = NEW.target_leg_id
        WHERE c.id = NEW.component_id
          AND c.workspace_id = NEW.workspace_id
          AND c.profile_id = NEW.profile_id
          AND source.component_id = c.id
          AND target.component_id = c.id
          AND source.role = 'source'
          AND target.role != 'source'
    ) THEN RAISE(ABORT, 'custody_component_terms_scope_mismatch') END;
END;
CREATE TRIGGER trg_custody_component_terms_immutable
BEFORE UPDATE ON custody_component_economic_terms
BEGIN
    SELECT RAISE(ABORT, 'custody_component_terms_immutable');
END;
CREATE TRIGGER trg_custody_component_terms_delete_immutable
BEFORE DELETE ON custody_component_economic_terms
WHEN EXISTS (SELECT 1 FROM profiles p WHERE p.id = OLD.profile_id)
AND NOT EXISTS (
    SELECT 1 FROM custody_component_purge_authorizations authorization
    WHERE authorization.profile_id = OLD.profile_id
)
BEGIN
    SELECT RAISE(ABORT, 'custody_component_terms_delete_immutable');
END;
CREATE INDEX idx_custody_evidence_commitments_component
    ON custody_component_evidence_commitments(component_id, ordinal, id);
CREATE TRIGGER trg_custody_evidence_commitment_scope_insert
BEFORE INSERT ON custody_component_evidence_commitments
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1 FROM custody_components c
        WHERE c.id = NEW.component_id
          AND c.workspace_id = NEW.workspace_id
          AND c.profile_id = NEW.profile_id
    ) THEN RAISE(ABORT, 'custody_evidence_commitment_scope_mismatch') END;
END;
CREATE INDEX idx_custody_component_legs_component
    ON custody_component_legs(component_id, ordinal, id);
CREATE INDEX idx_custody_component_legs_profile_transaction
    ON custody_component_legs(profile_id, transaction_id)
    WHERE transaction_id IS NOT NULL;
CREATE INDEX idx_custody_component_legs_profile_wallet
    ON custody_component_legs(profile_id, wallet_id)
    WHERE wallet_id IS NOT NULL;
CREATE INDEX idx_custody_allocations_component
    ON custody_component_allocations(component_id, ordinal, id);
CREATE INDEX idx_custody_allocations_source
    ON custody_component_allocations(source_leg_id);
CREATE INDEX idx_custody_allocations_sink
    ON custody_component_allocations(sink_leg_id);
CREATE TRIGGER trg_custody_allocation_scope_insert
BEFORE INSERT ON custody_component_allocations
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1
        FROM custody_components c
        JOIN custody_component_legs source ON source.id = NEW.source_leg_id
        JOIN custody_component_legs sink ON sink.id = NEW.sink_leg_id
        WHERE c.id = NEW.component_id
          AND c.workspace_id = NEW.workspace_id
          AND c.profile_id = NEW.profile_id
          AND source.component_id = c.id
          AND sink.component_id = c.id
          AND source.role = 'source'
          AND sink.role != 'source'
    ) THEN RAISE(ABORT, 'custody_allocation_scope_or_role_mismatch') END;
END;
CREATE TRIGGER trg_custody_allocation_scope_update
BEFORE UPDATE OF component_id, workspace_id, profile_id, source_leg_id, sink_leg_id
ON custody_component_allocations
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1
        FROM custody_components c
        JOIN custody_component_legs source ON source.id = NEW.source_leg_id
        JOIN custody_component_legs sink ON sink.id = NEW.sink_leg_id
        WHERE c.id = NEW.component_id
          AND c.workspace_id = NEW.workspace_id
          AND c.profile_id = NEW.profile_id
          AND source.component_id = c.id
          AND sink.component_id = c.id
          AND source.role = 'source'
          AND sink.role != 'source'
    ) THEN RAISE(ABORT, 'custody_allocation_scope_or_role_mismatch') END;
END;
CREATE INDEX idx_custody_memberships_component
    ON custody_component_transaction_memberships(component_id);
CREATE TRIGGER trg_custody_component_scope_insert
BEFORE INSERT ON custody_component_legs
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1 FROM custody_components c
        WHERE c.id = NEW.component_id
          AND c.workspace_id = NEW.workspace_id
          AND c.profile_id = NEW.profile_id
    ) THEN RAISE(ABORT, 'custody_leg_component_scope_mismatch') END;
    SELECT CASE WHEN NEW.transaction_id IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM transactions t
        WHERE t.id = NEW.transaction_id
          AND t.workspace_id = NEW.workspace_id
          AND t.profile_id = NEW.profile_id
    ) THEN RAISE(ABORT, 'custody_leg_transaction_scope_mismatch') END;
    SELECT CASE WHEN NEW.wallet_id IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM wallets w
        WHERE w.id = NEW.wallet_id
          AND w.workspace_id = NEW.workspace_id
          AND w.profile_id = NEW.profile_id
    ) THEN RAISE(ABORT, 'custody_leg_wallet_scope_mismatch') END;
END;
CREATE TRIGGER trg_custody_component_scope_update
BEFORE UPDATE OF component_id, workspace_id, profile_id, transaction_id, wallet_id
ON custody_component_legs
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1 FROM custody_components c
        WHERE c.id = NEW.component_id
          AND c.workspace_id = NEW.workspace_id
          AND c.profile_id = NEW.profile_id
    ) THEN RAISE(ABORT, 'custody_leg_component_scope_mismatch') END;
    SELECT CASE WHEN NEW.transaction_id IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM transactions t
        WHERE t.id = NEW.transaction_id
          AND t.workspace_id = NEW.workspace_id
          AND t.profile_id = NEW.profile_id
    ) THEN RAISE(ABORT, 'custody_leg_transaction_scope_mismatch') END;
    SELECT CASE WHEN NEW.wallet_id IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM wallets w
        WHERE w.id = NEW.wallet_id
          AND w.workspace_id = NEW.workspace_id
          AND w.profile_id = NEW.profile_id
    ) THEN RAISE(ABORT, 'custody_leg_wallet_scope_mismatch') END;
END;
CREATE TRIGGER trg_custody_component_memberships_supersede
AFTER UPDATE OF state ON custody_components
WHEN OLD.state = 'active' AND NEW.state != 'active'
BEGIN
    DELETE FROM custody_component_transaction_memberships
    WHERE component_id = NEW.id;
END;
CREATE TRIGGER trg_custody_component_memberships_leg_delete
AFTER DELETE ON custody_component_legs
WHEN OLD.transaction_id IS NOT NULL
BEGIN
    DELETE FROM custody_component_transaction_memberships
    WHERE component_id = OLD.component_id
      AND transaction_id = OLD.transaction_id
      AND NOT EXISTS (
          SELECT 1 FROM custody_component_legs l
          WHERE l.component_id = OLD.component_id
            AND l.transaction_id = OLD.transaction_id
      );
END;
CREATE INDEX idx_transaction_pairs_component
    ON transaction_pairs(component_id) WHERE component_id IS NOT NULL;
CREATE INDEX idx_direct_swap_payouts_component
    ON direct_swap_payouts(component_id) WHERE component_id IS NOT NULL;
CREATE INDEX idx_chain_analysis_watches_profile ON chain_analysis_watches(profile_id,enabled,id);
CREATE INDEX idx_chain_analysis_watch_inbox_profile ON chain_analysis_watch_inbox(profile_id,created_at,id);
CREATE TRIGGER trg_report_chain_captures_immutable
 BEFORE UPDATE ON filed_report_chain_captures BEGIN
 SELECT RAISE(ABORT, 'report_chain_captures_immutable'); END;
CREATE INDEX idx_report_chain_dependencies_wallet
 ON filed_report_chain_dependencies(profile_id,wallet_id,transaction_id);
CREATE INDEX idx_report_chain_dependencies_subject
 ON filed_report_chain_dependencies(profile_id,chain,network,txid);
CREATE INDEX idx_report_chain_impacts_dependency
 ON filed_report_chain_impacts(snapshot_id,transaction_id,created_at,id);
CREATE TRIGGER trg_report_chain_impacts_delete_inbox
 AFTER DELETE ON filed_report_chain_impacts BEGIN
 DELETE FROM chain_analysis_watch_inbox WHERE id=OLD.inbox_id; END;
CREATE TRIGGER trg_report_chain_dependencies_immutable
 BEFORE UPDATE ON filed_report_chain_dependencies BEGIN
 SELECT RAISE(ABORT, 'report_chain_dependencies_immutable'); END;
CREATE TRIGGER trg_report_chain_impacts_immutable
 BEFORE UPDATE ON filed_report_chain_impacts BEGIN
 SELECT RAISE(ABORT, 'report_chain_impacts_immutable'); END;
CREATE TRIGGER trg_report_chain_resolutions_immutable
 BEFORE UPDATE ON filed_report_chain_resolutions BEGIN
 SELECT RAISE(ABORT, 'report_chain_resolutions_immutable'); END;
CREATE INDEX idx_acquisition_grants_due
    ON chain_analysis_acquisition_grants(profile_id,status,next_run_at);
CREATE UNIQUE INDEX idx_reference_blocks_canonical
    ON chain_analysis_reference_blocks(grant_id,height) WHERE active=1;
CREATE INDEX idx_reference_assertions_profile
    ON chain_analysis_reference_assertions(profile_id,active,txid);
CREATE TRIGGER trg_journal_semantics_unstamped
        AFTER UPDATE OF last_processed_at, last_processed_input_version ON profiles
        BEGIN
            UPDATE profiles SET journal_semantics_version = 0,
                required_journal_semantics_version = MAX(required_journal_semantics_version, 1)
            WHERE id = NEW.id;
        END;
CREATE INDEX idx_transactions_wallet_pricing_ref
            ON transactions(wallet_id, pricing_external_ref, direction, asset, amount, created_at)
            WHERE pricing_external_ref IS NOT NULL
        ;
CREATE UNIQUE INDEX idx_bip329_labels_profile_object
            ON bip329_labels(profile_id, record_type, ref)
        ;
CREATE INDEX idx_bip329_labels_profile_created
            ON bip329_labels(profile_id, created_at DESC, id DESC)
        ;
CREATE INDEX idx_transactions_payment_hash ON transactions(payment_hash) WHERE payment_hash IS NOT NULL;
CREATE INDEX idx_transaction_pairs_active_out ON transaction_pairs(profile_id, out_transaction_id) WHERE deleted_at IS NULL;
CREATE INDEX idx_transaction_pairs_active_in ON transaction_pairs(profile_id, in_transaction_id) WHERE deleted_at IS NULL;
CREATE UNIQUE INDEX idx_transaction_pairs_active_pair ON transaction_pairs(profile_id, out_transaction_id, in_transaction_id) WHERE deleted_at IS NULL;
CREATE INDEX idx_transaction_pairs_profile_active ON transaction_pairs(profile_id) WHERE deleted_at IS NULL;
CREATE INDEX idx_transaction_pairs_component_pending ON transaction_pairs(id) WHERE component_id IS NULL;
CREATE INDEX idx_direct_swap_payouts_component_pending ON direct_swap_payouts(id) WHERE component_id IS NULL;
CREATE TRIGGER trg_transaction_pairs_component_write_frozen
        BEFORE UPDATE ON transaction_pairs
        WHEN OLD.component_id IS NOT NULL
          AND EXISTS (
              SELECT 1 FROM custody_components c
              WHERE c.id = OLD.component_id AND c.state = 'active'
          )
          AND (
              NEW.out_transaction_id IS NOT OLD.out_transaction_id OR
              NEW.in_transaction_id IS NOT OLD.in_transaction_id OR
              NEW.kind IS NOT OLD.kind OR
              NEW.policy IS NOT OLD.policy OR
              NEW.notes IS NOT OLD.notes OR
              NEW.swap_fee_msat IS NOT OLD.swap_fee_msat OR
              NEW.swap_fee_kind IS NOT OLD.swap_fee_kind OR
              NEW.confidence_at_pair IS NOT OLD.confidence_at_pair OR
              NEW.pair_source IS NOT OLD.pair_source OR
              NEW.out_amount IS NOT OLD.out_amount OR
              NEW.component_id IS NOT OLD.component_id OR
              NEW.deleted_at IS NOT OLD.deleted_at
          )
        BEGIN
            SELECT RAISE(ABORT, 'legacy_custody_review_write_frozen');
        END;
CREATE TRIGGER trg_direct_swap_payouts_component_write_frozen
        BEFORE UPDATE ON direct_swap_payouts
        WHEN OLD.component_id IS NOT NULL
          AND EXISTS (
              SELECT 1 FROM custody_components c
              WHERE c.id = OLD.component_id AND c.state = 'active'
          )
          AND (
              NEW.out_transaction_id IS NOT OLD.out_transaction_id OR
              NEW.kind IS NOT OLD.kind OR
              NEW.policy IS NOT OLD.policy OR
              NEW.payout_asset IS NOT OLD.payout_asset OR
              NEW.payout_amount IS NOT OLD.payout_amount OR
              NEW.payout_occurred_at IS NOT OLD.payout_occurred_at OR
              NEW.payout_fiat_value IS NOT OLD.payout_fiat_value OR
              NEW.payout_external_id IS NOT OLD.payout_external_id OR
              NEW.counterparty IS NOT OLD.counterparty OR
              NEW.notes IS NOT OLD.notes OR
              NEW.swap_fee_msat IS NOT OLD.swap_fee_msat OR
              NEW.swap_fee_kind IS NOT OLD.swap_fee_kind OR
              NEW.out_amount IS NOT OLD.out_amount OR
              NEW.component_id IS NOT OLD.component_id OR
              NEW.deleted_at IS NOT OLD.deleted_at
          )
        BEGIN
            SELECT RAISE(ABORT, 'legacy_custody_review_write_frozen');
        END;
CREATE VIEW journal_custody_projection_relations AS
        SELECT
            decision.profile_id,
            decision.decision_id AS id,
            'move' AS relation_kind,
            COALESCE(term.review_kind, component.component_type,
                     decision.reason) AS kind,
            COALESCE(term.tax_policy, component.conversion_policy,
                     'carrying-value') AS policy,
            term.swap_fee_msat,
            term.swap_fee_kind,
            term.confidence_at_review AS confidence_at_pair,
            COALESCE(term.review_source, component.authored_source,
                     'journal_builder') AS pair_source,
            COALESCE(term.review_notes, component.notes) AS notes,
            NULL AS target_external_id,
            NULL AS counterparty,
            NULL AS target_fiat_value_exact,
            decision.source_transaction_id AS out_transaction_id,
            decision.target_transaction_id AS in_transaction_id,
            decision.source_end_msat - decision.source_start_msat AS out_amount,
            decision.target_end_msat - decision.target_start_msat AS in_amount,
            decision.source_asset AS out_asset,
            decision.target_asset AS in_asset,
            decision.source_rail,
            decision.target_rail,
            decision.basis_state,
            decision.component_id,
            decision.occurred_at,
            decision.target_occurred_at,
            decision.created_at
        FROM journal_custody_decisions decision
        LEFT JOIN custody_components component
          ON component.id = decision.component_id
        LEFT JOIN custody_component_economic_terms term
          ON term.id = (
            SELECT candidate.id
            FROM custody_component_economic_terms candidate
            JOIN custody_component_legs source_leg
              ON source_leg.id = candidate.source_leg_id
            JOIN custody_component_legs target_leg
              ON target_leg.id = candidate.target_leg_id
            WHERE candidate.component_id = decision.component_id
              AND COALESCE(source_leg.anchor_transaction_id,
                           source_leg.transaction_id) =
                    decision.source_transaction_id
              AND COALESCE(target_leg.anchor_transaction_id,
                           target_leg.transaction_id) =
                    decision.target_transaction_id
            ORDER BY candidate.ordinal, candidate.id
            LIMIT 1
          )
        UNION ALL
        SELECT
            relation.profile_id,
            relation.relation_id AS id,
            relation.relation_kind,
            COALESCE(term.review_kind, component.component_type,
                     relation.relation_kind) AS kind,
            COALESCE(term.tax_policy, component.conversion_policy,
                     'taxable') AS policy,
            term.swap_fee_msat,
            term.swap_fee_kind,
            term.confidence_at_review AS confidence_at_pair,
            COALESCE(term.review_source, component.authored_source,
                     'journal_builder') AS pair_source,
            COALESCE(term.review_notes, component.notes) AS notes,
            term.payout_external_id AS target_external_id,
            term.counterparty,
            term.payout_fiat_value_exact AS target_fiat_value_exact,
            relation.source_transaction_id AS out_transaction_id,
            relation.target_transaction_id AS in_transaction_id,
            relation.source_amount_msat AS out_amount,
            relation.target_amount_msat AS in_amount,
            relation.source_asset AS out_asset,
            relation.target_asset AS in_asset,
            NULL AS source_rail,
            NULL AS target_rail,
            relation.basis_state,
            relation.component_id,
            relation.occurred_at,
            relation.target_occurred_at,
            relation.created_at
        FROM journal_custody_economic_relations relation
        LEFT JOIN custody_components component
          ON component.id = relation.component_id
        LEFT JOIN custody_component_economic_terms term
          ON term.id = (
            SELECT candidate.id
            FROM custody_component_economic_terms candidate
            JOIN custody_component_legs source_leg
              ON source_leg.id = candidate.source_leg_id
            JOIN custody_component_legs target_leg
              ON target_leg.id = candidate.target_leg_id
            WHERE candidate.component_id = relation.component_id
              AND COALESCE(source_leg.anchor_transaction_id,
                           source_leg.transaction_id) =
                    relation.source_transaction_id
              AND (
                relation.target_transaction_id IS NULL
                OR COALESCE(target_leg.anchor_transaction_id,
                            target_leg.transaction_id) =
                     relation.target_transaction_id
              )
              AND candidate.term_kind = CASE relation.relation_kind
                    WHEN 'direct_payout' THEN 'direct_swap_payout'
                    ELSE 'transaction_pair'
                  END
            ORDER BY candidate.ordinal, candidate.id
            LIMIT 1
          );
CREATE TRIGGER trg_custody_component_revision_immutable
        BEFORE UPDATE ON custody_components
        WHEN OLD.id IS NOT NEW.id
          OR OLD.lineage_id IS NOT NEW.lineage_id
          OR OLD.workspace_id IS NOT NEW.workspace_id
          OR OLD.profile_id IS NOT NEW.profile_id
          OR OLD.revision IS NOT NEW.revision
          OR OLD.component_type IS NOT NEW.component_type
          OR OLD.conservation_mode IS NOT NEW.conservation_mode
          OR OLD.evidence_kind IS NOT NEW.evidence_kind
          OR OLD.evidence_grade IS NOT NEW.evidence_grade
          OR OLD.evidence_json IS NOT NEW.evidence_json
          OR OLD.conversion_policy IS NOT NEW.conversion_policy
          OR OLD.conversion_reviewed IS NOT NEW.conversion_reviewed
          OR OLD.conversion_metadata_json IS NOT NEW.conversion_metadata_json
          OR OLD.expected_leg_count IS NOT NEW.expected_leg_count
          OR OLD.expected_allocation_count IS NOT NEW.expected_allocation_count
          OR OLD.expected_economic_term_count IS NOT NEW.expected_economic_term_count
          OR (
              OLD.expected_evidence_count IS NOT NEW.expected_evidence_count
              AND NOT (
                  OLD.expected_evidence_count IS NULL
                  AND NEW.expected_evidence_count IS NOT NULL
                  AND NEW.expected_evidence_count >= 0
                  AND (
                      (OLD.state IN ('active', 'superseded') AND OLD.state = NEW.state)
                      OR (OLD.state = 'draft' AND NEW.state = 'active')
                  )
              )
          )
          OR OLD.authored_source IS NOT NEW.authored_source
          OR OLD.notes IS NOT NEW.notes
          OR OLD.supersedes_component_id IS NOT NEW.supersedes_component_id
          OR OLD.created_at IS NOT NEW.created_at
        BEGIN
            SELECT RAISE(ABORT, 'custody_component_revision_immutable');
        END;
CREATE TRIGGER trg_custody_component_leg_revision_immutable
        BEFORE UPDATE ON custody_component_legs
        WHEN OLD.id IS NOT NEW.id
          OR OLD.component_id IS NOT NEW.component_id
          OR OLD.workspace_id IS NOT NEW.workspace_id
          OR OLD.profile_id IS NOT NEW.profile_id
          OR OLD.ordinal IS NOT NEW.ordinal
          OR OLD.role IS NOT NEW.role
          OR OLD.rail IS NOT NEW.rail
          OR OLD.chain IS NOT NEW.chain
          OR OLD.network IS NOT NEW.network
          OR OLD.asset IS NOT NEW.asset
          OR OLD.exposure IS NOT NEW.exposure
          OR OLD.conservation_unit IS NOT NEW.conservation_unit
          OR OLD.amount_msat IS NOT NEW.amount_msat
          OR OLD.valuation_unit IS NOT NEW.valuation_unit
          OR OLD.valuation_amount IS NOT NEW.valuation_amount
          OR OLD.occurred_at IS NOT NEW.occurred_at
          OR OLD.anchor_transaction_id IS NOT NEW.anchor_transaction_id
          OR OLD.location_ref IS NOT NEW.location_ref
          OR OLD.notes IS NOT NEW.notes
          OR OLD.created_at IS NOT NEW.created_at
          OR NOT (
              OLD.transaction_id IS NEW.transaction_id
              OR NEW.transaction_id IS NULL
              OR (
                  OLD.transaction_id IS NULL
                  AND NEW.transaction_id IS OLD.anchor_transaction_id
              )
          )
          OR NOT (
              OLD.wallet_id IS NEW.wallet_id
              OR NEW.wallet_id IS NULL
          )
        BEGIN
            SELECT RAISE(ABORT, 'custody_component_leg_revision_immutable');
        END;
CREATE TRIGGER trg_custody_component_evidence_revision_immutable
        BEFORE UPDATE ON custody_component_evidence_commitments
        BEGIN
            SELECT RAISE(ABORT, 'custody_component_evidence_revision_immutable');
        END;
CREATE TRIGGER trg_custody_component_revision_delete_immutable
        BEFORE DELETE ON custody_components
        WHEN EXISTS (
            SELECT 1 FROM profiles p WHERE p.id = OLD.profile_id
        )
        AND NOT EXISTS (
            SELECT 1 FROM custody_component_purge_authorizations authorization
            WHERE authorization.profile_id = OLD.profile_id
        )
        BEGIN
            SELECT RAISE(ABORT, 'custody_component_revision_delete_immutable');
        END;
CREATE TRIGGER trg_custody_component_leg_revision_delete_immutable
        BEFORE DELETE ON custody_component_legs
        WHEN EXISTS (
            SELECT 1 FROM profiles p WHERE p.id = OLD.profile_id
        )
        AND NOT EXISTS (
            SELECT 1 FROM custody_component_purge_authorizations authorization
            WHERE authorization.profile_id = OLD.profile_id
        )
        BEGIN
            SELECT RAISE(ABORT, 'custody_component_leg_revision_delete_immutable');
        END;
CREATE TRIGGER trg_custody_component_allocation_revision_delete_immutable
        BEFORE DELETE ON custody_component_allocations
        WHEN EXISTS (
            SELECT 1 FROM profiles p WHERE p.id = OLD.profile_id
        )
        AND NOT EXISTS (
            SELECT 1 FROM custody_component_purge_authorizations authorization
            WHERE authorization.profile_id = OLD.profile_id
        )
        BEGIN
            SELECT RAISE(ABORT, 'custody_component_allocation_revision_delete_immutable');
        END;
CREATE TRIGGER trg_custody_component_evidence_revision_delete_immutable
        BEFORE DELETE ON custody_component_evidence_commitments
        WHEN EXISTS (
            SELECT 1 FROM profiles p WHERE p.id = OLD.profile_id
        )
        AND NOT EXISTS (
            SELECT 1 FROM custody_component_purge_authorizations authorization
            WHERE authorization.profile_id = OLD.profile_id
        )
        BEGIN
            SELECT RAISE(ABORT, 'custody_component_evidence_revision_delete_immutable');
        END;
CREATE TRIGGER trg_custody_component_leg_count_commitment
        BEFORE INSERT ON custody_component_legs
        WHEN NOT EXISTS (
            SELECT 1 FROM custody_component_legs current WHERE current.id = NEW.id
        )
        AND
        (
            SELECT c.expected_leg_count FROM custody_components c
            WHERE c.id = NEW.component_id
        ) IS NOT NULL
        AND (
            SELECT COUNT(*) FROM custody_component_legs l
            WHERE l.component_id = NEW.component_id
        ) >= (
            SELECT c.expected_leg_count FROM custody_components c
            WHERE c.id = NEW.component_id
        )
        BEGIN
            SELECT RAISE(ABORT, 'custody_component_leg_count_commitment');
        END;
CREATE TRIGGER trg_custody_component_allocation_count_commitment
        BEFORE INSERT ON custody_component_allocations
        WHEN NOT EXISTS (
            SELECT 1 FROM custody_component_allocations current WHERE current.id = NEW.id
        )
        AND
        (
            SELECT c.expected_allocation_count FROM custody_components c
            WHERE c.id = NEW.component_id
        ) IS NOT NULL
        AND (
            SELECT COUNT(*) FROM custody_component_allocations a
            WHERE a.component_id = NEW.component_id
        ) >= (
            SELECT c.expected_allocation_count FROM custody_components c
            WHERE c.id = NEW.component_id
        )
        BEGIN
            SELECT RAISE(ABORT, 'custody_component_allocation_count_commitment');
        END;
CREATE TRIGGER trg_custody_component_economic_term_count_commitment
        BEFORE INSERT ON custody_component_economic_terms
        WHEN NOT EXISTS (
            SELECT 1 FROM custody_component_economic_terms current
            WHERE current.id = NEW.id
        )
        AND
        (
            SELECT c.expected_economic_term_count FROM custody_components c
            WHERE c.id = NEW.component_id
        ) IS NOT NULL
        AND (
            SELECT COUNT(*) FROM custody_component_economic_terms terms
            WHERE terms.component_id = NEW.component_id
        ) >= (
            SELECT c.expected_economic_term_count FROM custody_components c
            WHERE c.id = NEW.component_id
        )
        BEGIN
            SELECT RAISE(ABORT, 'custody_component_economic_term_count_commitment');
        END;
CREATE TRIGGER trg_custody_component_evidence_count_commitment
        BEFORE INSERT ON custody_component_evidence_commitments
        WHEN NOT EXISTS (
            SELECT 1 FROM custody_component_evidence_commitments current
            WHERE current.id = NEW.id
        )
        AND
        (
            SELECT c.expected_evidence_count FROM custody_components c
            WHERE c.id = NEW.component_id
        ) IS NOT NULL
        AND (
            SELECT COUNT(*) FROM custody_component_evidence_commitments evidence
            WHERE evidence.component_id = NEW.component_id
        ) >= (
            SELECT c.expected_evidence_count FROM custody_components c
            WHERE c.id = NEW.component_id
        )
        BEGIN
            SELECT RAISE(ABORT, 'custody_component_evidence_count_commitment');
        END;
CREATE TRIGGER trg_custody_component_allocation_revision_immutable
        BEFORE UPDATE ON custody_component_allocations
        WHEN OLD.id IS NOT NEW.id
          OR OLD.component_id IS NOT NEW.component_id
          OR OLD.workspace_id IS NOT NEW.workspace_id
          OR OLD.profile_id IS NOT NEW.profile_id
          OR OLD.ordinal IS NOT NEW.ordinal
          OR OLD.source_leg_id IS NOT NEW.source_leg_id
          OR OLD.sink_leg_id IS NOT NEW.sink_leg_id
          OR OLD.source_amount_msat IS NOT NEW.source_amount_msat
          OR OLD.sink_amount_msat IS NOT NEW.sink_amount_msat
          OR OLD.created_at IS NOT NEW.created_at
        BEGIN
            SELECT RAISE(ABORT, 'custody_component_allocation_revision_immutable');
        END;
CREATE INDEX idx_btcpay_provenance_profile_payment_request ON btcpay_provenance_records(profile_id, payment_request_id) WHERE payment_request_id IS NOT NULL;
CREATE UNIQUE INDEX idx_commercial_links_unique_payment_tx_active
            ON commercial_links(
                profile_id,
                COALESCE(btcpay_record_id, ''),
                COALESCE(transaction_id, ''),
                link_type
            ) WHERE state != 'rejected' AND link_type = 'btcpay_payment_transaction'
        ;
CREATE UNIQUE INDEX idx_commercial_links_unique_other_active
            ON commercial_links(
                profile_id,
                COALESCE(btcpay_record_id, ''),
                COALESCE(document_id, ''),
                COALESCE(transaction_id, ''),
                link_type
            ) WHERE state != 'rejected' AND link_type != 'btcpay_payment_transaction'
        ;
CREATE UNIQUE INDEX idx_commercial_links_one_reviewed_btcpay_payment
            ON commercial_links(profile_id, btcpay_record_id)
            WHERE state = 'reviewed'
              AND link_type = 'btcpay_payment_transaction'
              AND btcpay_record_id IS NOT NULL
        ;
CREATE INDEX idx_chain_index_dirty_revision ON chain_index_dirty(revision);
CREATE INDEX idx_chain_index_contributors ON chain_index_source_nodes(profile_id,node_id);
CREATE INDEX idx_chain_index_node_counts ON chain_index_nodes(profile_id,status,complete);
CREATE INDEX idx_chain_index_private_nodes ON chain_index_nodes(profile_id) WHERE public_json IS NULL;
CREATE INDEX idx_chain_index_source_counts ON chain_index_sources(profile_id,invalid_count,rejected_count);
CREATE INDEX idx_chain_index_outgoing ON chain_index_edges(profile_id,source,id);
CREATE INDEX idx_chain_index_incoming ON chain_index_edges(profile_id,target,id);
CREATE INDEX idx_chain_index_private_edges ON chain_index_edges(profile_id) WHERE public_json IS NULL;
CREATE INDEX idx_chain_index_custody ON chain_index_edges(profile_id) WHERE json_extract(payload_json,'$.kind')='custody';
CREATE INDEX idx_chain_index_alias_node ON chain_index_aliases(profile_id,node_id);
CREATE INDEX idx_chain_index_finding_node ON chain_index_finding_nodes(profile_id,node_id);
CREATE INDEX idx_chain_index_label_node ON chain_index_label_nodes(profile_id,node_id);
CREATE INDEX idx_chain_index_label_subject ON chain_index_labels(profile_id,json_extract(payload_json,'$.subject'));
CREATE INDEX idx_chain_index_claim_valid_from ON chain_analysis_dataset_claims(valid_from,dataset_id) WHERE valid_from IS NOT NULL;
CREATE INDEX idx_chain_index_claim_valid_until ON chain_analysis_dataset_claims(valid_until,dataset_id) WHERE valid_until IS NOT NULL;
CREATE TRIGGER chain_index_track_transactions_insert AFTER INSERT ON transactions BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('transactions',NEW.rowid,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
CREATE TRIGGER chain_index_track_transactions_update AFTER UPDATE ON transactions BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('transactions',OLD.rowid,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision;INSERT INTO chain_index_dirty VALUES('transactions',NEW.rowid,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
CREATE TRIGGER chain_index_track_transactions_delete AFTER DELETE ON transactions BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('transactions',OLD.rowid,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
CREATE TRIGGER chain_index_track_transaction_graph_cache_insert AFTER INSERT ON transaction_graph_cache BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('transaction_graph_cache',NEW.rowid,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
CREATE TRIGGER chain_index_track_transaction_graph_cache_update AFTER UPDATE ON transaction_graph_cache BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('transaction_graph_cache',OLD.rowid,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision;INSERT INTO chain_index_dirty VALUES('transaction_graph_cache',NEW.rowid,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
CREATE TRIGGER chain_index_track_transaction_graph_cache_delete AFTER DELETE ON transaction_graph_cache BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('transaction_graph_cache',OLD.rowid,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
CREATE TRIGGER chain_index_track_wallet_utxos_insert AFTER INSERT ON wallet_utxos BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('wallet_utxos',NEW.rowid,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
CREATE TRIGGER chain_index_track_wallet_utxos_update AFTER UPDATE ON wallet_utxos BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('wallet_utxos',OLD.rowid,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision;INSERT INTO chain_index_dirty VALUES('wallet_utxos',NEW.rowid,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
CREATE TRIGGER chain_index_track_wallet_utxos_delete AFTER DELETE ON wallet_utxos BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('wallet_utxos',OLD.rowid,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
CREATE TRIGGER chain_index_track_chain_analysis_observations_insert AFTER INSERT ON chain_analysis_observations BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('chain_analysis_observations',NEW.rowid,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
CREATE TRIGGER chain_index_track_chain_analysis_observations_update AFTER UPDATE ON chain_analysis_observations BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('chain_analysis_observations',OLD.rowid,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision;INSERT INTO chain_index_dirty VALUES('chain_analysis_observations',NEW.rowid,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
CREATE TRIGGER chain_index_track_chain_analysis_observations_delete AFTER DELETE ON chain_analysis_observations BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('chain_analysis_observations',OLD.rowid,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
CREATE TRIGGER chain_index_track_chain_analysis_reference_assertions_insert AFTER INSERT ON chain_analysis_reference_assertions BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('chain_analysis_reference_assertions',NEW.rowid,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
CREATE TRIGGER chain_index_track_chain_analysis_reference_assertions_update AFTER UPDATE ON chain_analysis_reference_assertions BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('chain_analysis_reference_assertions',OLD.rowid,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision;INSERT INTO chain_index_dirty VALUES('chain_analysis_reference_assertions',NEW.rowid,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
CREATE TRIGGER chain_index_track_chain_analysis_reference_assertions_delete AFTER DELETE ON chain_analysis_reference_assertions BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('chain_analysis_reference_assertions',OLD.rowid,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
CREATE TRIGGER chain_index_track_wallets_insert AFTER INSERT ON wallets BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('wallets',-1,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
CREATE TRIGGER chain_index_track_wallets_update AFTER UPDATE ON wallets BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('wallets',-1,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
CREATE TRIGGER chain_index_track_wallets_delete AFTER DELETE ON wallets BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('wallets',-1,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
CREATE TRIGGER chain_index_track_profiles_insert AFTER INSERT ON profiles BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('profiles',-1,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
CREATE TRIGGER chain_index_track_profiles_update AFTER UPDATE ON profiles BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('profiles',-1,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
CREATE TRIGGER chain_index_track_profiles_delete AFTER DELETE ON profiles BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('profiles',-1,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
CREATE TRIGGER chain_index_track_journal_custody_decisions_insert AFTER INSERT ON journal_custody_decisions BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('journal_custody_decisions',-1,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
CREATE TRIGGER chain_index_track_journal_custody_decisions_update AFTER UPDATE ON journal_custody_decisions BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('journal_custody_decisions',-1,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
CREATE TRIGGER chain_index_track_journal_custody_decisions_delete AFTER DELETE ON journal_custody_decisions BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('journal_custody_decisions',-1,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
CREATE TRIGGER chain_index_track_journal_custody_economic_relations_insert AFTER INSERT ON journal_custody_economic_relations BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('journal_custody_economic_relations',-1,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
CREATE TRIGGER chain_index_track_journal_custody_economic_relations_update AFTER UPDATE ON journal_custody_economic_relations BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('journal_custody_economic_relations',-1,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
CREATE TRIGGER chain_index_track_journal_custody_economic_relations_delete AFTER DELETE ON journal_custody_economic_relations BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('journal_custody_economic_relations',-1,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
CREATE TRIGGER chain_index_track_chain_analysis_labels_insert AFTER INSERT ON chain_analysis_labels BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('chain_analysis_labels',-1,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
CREATE TRIGGER chain_index_track_chain_analysis_labels_update AFTER UPDATE ON chain_analysis_labels BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('chain_analysis_labels',-1,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
CREATE TRIGGER chain_index_track_chain_analysis_labels_delete AFTER DELETE ON chain_analysis_labels BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('chain_analysis_labels',-1,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
CREATE TRIGGER chain_index_track_chain_analysis_datasets_insert AFTER INSERT ON chain_analysis_datasets BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('chain_analysis_datasets',-1,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
CREATE TRIGGER chain_index_track_chain_analysis_datasets_update AFTER UPDATE ON chain_analysis_datasets BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('chain_analysis_datasets',-1,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
CREATE TRIGGER chain_index_track_chain_analysis_datasets_delete AFTER DELETE ON chain_analysis_datasets BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('chain_analysis_datasets',-1,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
CREATE TRIGGER chain_index_track_chain_analysis_dataset_claims_insert AFTER INSERT ON chain_analysis_dataset_claims BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('chain_analysis_dataset_claims',-1,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
CREATE TRIGGER chain_index_track_chain_analysis_dataset_claims_update AFTER UPDATE ON chain_analysis_dataset_claims BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('chain_analysis_dataset_claims',-1,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
CREATE TRIGGER chain_index_track_chain_analysis_dataset_claims_delete AFTER DELETE ON chain_analysis_dataset_claims BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('chain_analysis_dataset_claims',-1,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
CREATE TRIGGER chain_index_track_book_network_bindings_insert AFTER INSERT ON book_network_bindings BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('book_network_bindings',-1,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
CREATE TRIGGER chain_index_track_book_network_bindings_update AFTER UPDATE ON book_network_bindings BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('book_network_bindings',-1,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
CREATE TRIGGER chain_index_track_book_network_bindings_delete AFTER DELETE ON book_network_bindings BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('book_network_bindings',-1,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
CREATE TRIGGER chain_index_track_chain_analysis_acquisition_grants_insert AFTER INSERT ON chain_analysis_acquisition_grants BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('chain_analysis_acquisition_grants',-1,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
CREATE TRIGGER chain_index_track_chain_analysis_acquisition_grants_update AFTER UPDATE ON chain_analysis_acquisition_grants WHEN OLD.verified_tip_height IS NOT NEW.verified_tip_height OR OLD.verified_tip_hash IS NOT NEW.verified_tip_hash OR OLD.verified_tip_at IS NOT NEW.verified_tip_at OR OLD.last_code IS NOT NEW.last_code OR OLD.status IS NOT NEW.status BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('chain_analysis_acquisition_grants',-1,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
CREATE TRIGGER chain_index_track_chain_analysis_acquisition_grants_delete AFTER DELETE ON chain_analysis_acquisition_grants BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; INSERT INTO chain_index_dirty VALUES('chain_analysis_acquisition_grants',-1,(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision; END;
COMMIT;
