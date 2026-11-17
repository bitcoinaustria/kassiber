//! The replay bundle's inputs: an explicit, hand-written projection schema
//! of the custody snapshot (docs/reference/replay-bundle.md#what-the-inputs-keep).
//!
//! Every section of the snapshot and every document held in it has its own
//! schema. A field the schema does not name is dropped; a named field is
//! kept only in its class's form:
//!
//! - `Row`: a local row identity (generated ids), kept when it has an id's
//!   shape, otherwise replaced whole;
//! - `External`/`Digest`: replaced whole by a keyed pseudonym (txids,
//!   outpoints, addresses and hashes keep their shape, nothing else does);
//! - `Amount`, `Time`, `Int`, `Bool`: kept when the value has that format;
//! - `Enum`: kept when the value is in the list written here;
//! - `Marker`: kept when it is one of the residual markers the engine
//!   interprets (`MARKERS`), otherwise authored text;
//! - `Text`: authored text, replaced whole by a label pseudonym.
//!
//! A value that fails its class's check is replaced whole, never kept.
//! Codec tags are admitted only from `CODEC_TAGS`. `tests/projection_coverage.rs`
//! fails when the loader hands the pipeline a column no schema classifies.
use crate::registry;
use crate::swap;
use serde_json::Value;
use std::collections::BTreeSet;

/// The residual markers the custody engine interprets in leg and component
/// `notes`, each with the source site that reads it
/// (`tests/projection_coverage.rs` checks every site still names it).
pub const MARKERS: &[(&str, &str)] = &[
    (
        "reviewed_residual:retained_custody",
        "custody/src/components/anchors.rs",
    ),
    (
        "reviewed_residual:network_fee",
        "custody/src/components/anchors.rs",
    ),
    // `quantity/reviewed.rs` strips the prefix and reads the classification.
    (
        "reviewed_residual:external_payment",
        "custody/src/quantity/reviewed.rs",
    ),
    (
        "reviewed_residual:external_disposal",
        "custody/src/quantity/reviewed.rs",
    ),
    (
        "reviewed_residual:external_gift",
        "custody/src/quantity/reviewed.rs",
    ),
    (
        "reviewed_residual:external_loss",
        "custody/src/quantity/reviewed.rs",
    ),
    // Read by its prefix there; the classification comes from the gap review.
    (
        "reviewed_residual:suspense_continuation",
        "custody/src/quantity/reviewed.rs",
    ),
];

/// Local reference prefixes of `location_ref` (`network-fee:<review id>`,
/// `reviewed-custody-suspense:<review id>`, written by `transfers/src/pair_spec.rs`).
pub const LOCAL_REFS: &[&str] = &["network-fee", "reviewed-custody-suspense"];

#[derive(Clone, Copy)]
pub enum F {
    Row,
    External,
    Digest,
    Amount,
    Time,
    Int,
    Bool,
    Enum(&'static [&'static str]),
    /// A value a parser recognizes (the tax-country registry's settings),
    /// printed again from its constant.
    Parsed(fn(&str) -> Option<String>),
    Marker,
    Text,
    /// A JSON document held as text, projected by its schema.
    Doc(&'static [(&'static str, F)]),
    /// A nested object (or a list of them), projected by its schema.
    Obj(&'static [(&'static str, F)]),
    /// A list of values of one class.
    List(&'static F),
    /// An integer here, a nested object or list there (`vout`).
    IntOr(&'static [(&'static str, F)]),
    /// `network-fee:<row id>` and similar local references, else external.
    LocationRef,
    /// A JSON document held as text that is a list of values of one class.
    DocList(&'static F),
    /// A scalar of one class here, a nested object there (a swap provider's
    /// `status` text beside an Esplora `status` object).
    Or(&'static F, &'static [(&'static str, F)]),
}

type Schema = &'static [(&'static str, F)];

const DIRECTIONS: &[&str] = &[
    "inbound", "outbound", "in", "out", "internal", "self", "send", "receive", "incoming",
    "outgoing",
];
const ASSETS: &[&str] = &[
    "BTC", "LBTC", "L-BTC", "btc", "lbtc", "l-btc", "LN", "SAT", "SATS",
];
const CHAINS: &[&str] = &["bitcoin", "liquid", "lightning", "elements"];
const NETWORKS: &[&str] = &[
    "main",
    "mainnet",
    "bitcoin",
    "test",
    "testnet",
    "testnet3",
    "testnet4",
    "signet",
    "regtest",
    "liquid",
    "liquidv1",
    "liquidtestnet",
    "elementsregtest",
    "lightning",
];
const TX_KINDS: &[&str] = &[
    "airdrop",
    "buy",
    "deposit",
    "hardfork",
    "income",
    "interest",
    "lending_interest",
    "mining",
    "staking",
    "wages",
    "donation",
    "gift",
    "lost",
    "sell",
    "spend",
    "stolen",
    "withdrawal",
    "transfer",
    "fee",
    "swap",
    "payment",
    "trade",
    "receive",
    "send",
    "refund",
    "rebate",
    "reward",
    "cashback",
    "bonus",
    "loan",
    "repayment",
    "collateral",
    "conversion",
    "exchange",
    "internal",
    "self_transfer",
    "peg_in",
    "peg_out",
    "channel_open",
    "channel_close",
    "routing",
    "invoice",
    "expense",
];
const WALLET_KINDS: &[&str] = &[
    "descriptor",
    "xpub",
    "address",
    "silent-payment",
    "coreln",
    "lnd",
    "nwc",
    "phoenix",
    "river",
    "bullbitcoin",
    "coinfinity",
    "21bitcoin",
    "pocketbitcoin",
    "strike",
    "ledgerlive",
    "cointracking",
    "blockpit",
    "kraken",
    "coinbase",
    "binance",
    "bitpanda",
    "wasabi",
    "samourai",
    "untracked",
    "custom",
    "manual",
    "btcpay",
    "bark",
    "liquid",
    "lightning",
];
const EXTERNAL_ID_KINDS: &[&str] = &[
    "txid",
    "outpoint",
    "payment_hash",
    "invoice",
    "order_id",
    "trade_id",
    "transaction_id",
    "native",
    "row",
    "reference",
];
const PRICING_SOURCE_KINDS: &[&str] = &[
    "manual",
    "market",
    "import",
    "exchange",
    "provider",
    "reviewed",
    "cache",
    "fallback",
    "transaction",
    "receipt",
    "rates_cache",
    "manual_override",
];
const PRICING_QUALITIES: &[&str] = &[
    "exact",
    "coarse",
    "estimated",
    "derived",
    "fallback",
    "missing",
    "approximate",
    "hourly",
    "daily",
];
const PRICING_GRANULARITIES: &[&str] = &[
    "exact",
    "minute",
    "hourly",
    "hour",
    "daily",
    "day",
    "transaction",
];
const PRICING_PROVIDERS: &[&str] = &[
    "coingecko",
    "kraken",
    "coinbase",
    "binance",
    "bitpanda",
    "bitstamp",
    "ecb",
    "mempool",
    "reviewed-receipt",
    "manual",
    "import",
    "cointracking",
    "blockpit",
    "strike",
    "river",
];
const PRICING_METHODS: &[&str] = &[
    "quarantine_price_override",
    "import",
    "manual",
    "nearest",
    "interpolated",
    "close",
    "rates_cache",
];
const REVIEW_STATUSES: &[&str] = &[
    "review",
    "needs_review",
    "blocked",
    "pending",
    "failed",
    "reviewed",
    "ok",
    "done",
    "ignored",
];
const AT_REGIMES: &[&str] = &["alt", "neu"];
/// The tax engine's Austrian categories (`tax-engine/core/src/austria.rs`).
const AT_CATEGORIES: &[&str] = &[
    "alt",
    "alt_spekulation",
    "alt_taxfree",
    "income_capital_yield",
    "income_general",
    "neu",
    "neu_gain",
    "neu_loss",
    "neu_swap",
];
const PRIVACY_BOUNDARIES: &[&str] = &["coinjoin", "payjoin", "whirlpool", "none", "mixed"];
const PAYMENT_HASH_SOURCES: &[&str] = &["invoice", "import", "native", "provider", "derived"];
const ACCOUNT_CODES: &[&str] = &["treasury"];
const FEE_ATTRIBUTIONS: &[&str] = &["sender", "receiver", "wallet", "none", "shared", "unknown"];
const OBSERVER_KINDS: &[&str] = &["bdk", "lwk", "electrum", "esplora", "bitcoinrpc", "import"];
const COMPONENT_TYPES: &[&str] = &[
    "manual_bridge",
    "transfer",
    "swap",
    "peg",
    "conversion",
    "consolidation",
    "payout",
    "channel",
    "migration",
];
const CONSERVATION_MODES: &[&str] = &["quantity", "value", "none"];
const COMPONENT_STATES: &[&str] = &["draft", "active", "superseded", "reopened", "rejected"];
const EVIDENCE_KINDS: &[&str] = &[
    "reviewed_custody",
    "custody_gap_review",
    "transaction_pair",
    "native",
    "import",
    "migration",
];
const EVIDENCE_GRADES: &[&str] = &["reviewed", "native", "inferred", "imported", "legacy"];
const AUTHORED_SOURCES: &[&str] = &[
    "cli",
    "gui",
    "ai_tool",
    "import",
    "migration",
    "user",
    "manual",
    "auto",
    "rule",
    "automatic",
    "suggestion",
    "review",
    "desktop",
];
/// The leg roles conservation accepts (`custody/src/components/conservation.rs`).
const ROLES: &[&str] = &[
    "source",
    "destination",
    "fee",
    "external",
    "retained",
    "suspense",
];
const RAILS: &[&str] = &[
    "bitcoin",
    "liquid",
    "lightning",
    "ecash",
    "exchange",
    "custodial",
];
const UNITS: &[&str] = &["msat", "sat", "BTC", "EUR", "USD", "CHF", "GBP"];
const TERM_KINDS: &[&str] = &[
    "transaction_pair",
    "swap",
    "direct_payout",
    "conversion",
    "peg",
    "fee",
    "loan",
];
const POLICIES: &[&str] = &[
    "carrying-value",
    "taxable",
    "direct-swap-payout",
    "carrying_value",
    "disposal",
    "non-taxable",
    "non_taxable",
    "swap",
    "transfer",
    "default",
];
const CONFIDENCES: &[&str] = &["exact", "strong", "weak", "manual", "reviewed"];
const RESIDUALS: &[&str] = &[
    "retained_custody",
    "network_fee",
    "suspense",
    "external_payment",
    "external_disposal",
    "external_gift",
    "external_loss",
];
const LOAN_ROLES: &[&str] = &[
    "collateral",
    "release",
    "borrow",
    "repay",
    "liquidation",
    "interest",
];
const QUARANTINE_REASONS: &[&str] = &[
    "synthetic_import_duplicate_review_required",
    "book_reset_decision_held",
];
const REFERENCE_TABLES: &[&str] = &[
    "chain_analysis_observations",
    "chain_observer_instances",
    "chain_observation_provenance",
    "wallet_policy_epochs",
    "wallet_utxos",
];
const ENVIRONMENTS: &[&str] = &["main", "test", "signet", "regtest"];
const GAP_ACTIONS: &[&str] = &["accept", "dismiss", "defer", "reopen", "hold", "resolve"];
const OWNED_SOURCES: &[&str] = &[
    "wallet_utxos",
    "descriptor",
    "address",
    "receipt",
    "silent_payments",
    "import",
    "derived",
];
const SAMOURAI_ROLES: &[&str] = &["deposit", "premix", "postmix", "badbank", "ricochet"];
const CHANNEL_TAGS: &[&str] = &["open", "close", "funding", "sweep", "force_close", "splice"];

static ROW_LIST: F = F::Row;
static EXTERNAL_LIST: F = F::External;
static OBSERVER_LIST: F = F::Enum(OBSERVER_KINDS);

const CONFIG: Schema = &[
    ("chain", F::Enum(CHAINS)),
    ("network", F::Enum(NETWORKS)),
    ("bitcoin_network", F::Enum(NETWORKS)),
    ("chain_network", F::Enum(NETWORKS)),
    ("chain_instance_id", F::External),
    ("samourai", F::Obj(SAMOURAI)),
];
const SAMOURAI: Schema = &[
    ("role", F::Enum(SAMOURAI_ROLES)),
    ("group_id", F::Row),
    ("section", F::Enum(SAMOURAI_ROLES)),
];

/// The on-chain and Lightning payload fields the graph, confirmation,
/// network and channel interpreters read from `raw_json`.
static RAW_FIELDS: [(&str, F); 84] = [
    ("txid", F::External),
    ("hash", F::External),
    ("tx_hash", F::External),
    ("transaction_hash", F::External),
    ("funding_txid", F::External),
    ("closing_txid", F::External),
    ("sweep_txid", F::External),
    ("outpoint", F::External),
    ("funding_outpoint", F::External),
    ("address", F::External),
    ("scriptpubkey", F::External),
    ("script_pubkey", F::External),
    ("scriptpubkey_address", F::External),
    ("script", F::External),
    ("payment_hash", F::External),
    ("channel_id", F::External),
    ("hex", F::External),
    ("raw_hex", F::External),
    ("asset_id", F::External),
    ("observer_owned_scripts", F::List(&EXTERNAL_LIST)),
    ("tx", F::Obj(&RAW_FIELDS)),
    // The ownership graph an observer stored beside the transaction, read
    // like `tx` (custody/src/confirmation.rs, graph.rs).
    ("ownership_graph", F::Obj(&RAW_FIELDS)),
    // Liquid asset facts (`asset_id`, `asset`) (custody/src/graph/ownership.rs).
    ("component", F::Obj(&RAW_FIELDS)),
    // The import that wrote the row (custody/src/interpretation/recorded.rs,
    // channels.rs, evidence.rs).
    ("_kassiber_provenance", F::Obj(PROVENANCE)),
    // LND close sweeps (`sweep_txid`, `outpoint`) (interpretation/channels.rs).
    ("channel_close_local_sweeps", F::Obj(&RAW_FIELDS)),
    // A verified HTLC spend (custody/src/interpretation/htlc.rs): the proof,
    // the observer that wrote it, and the input witness it hashes.
    ("htlc_spend", F::Obj(HTLC_PROOF)),
    ("observer", F::Enum(OBSERVERS)),
    ("witness", F::List(&EXTERNAL_LIST)),
    ("txinwitness", F::List(&EXTERNAL_LIST)),
    // Swap provider evidence (custody/src/interpretation/matcher/providers.rs).
    ("provider", F::Parsed(swap::provider)),
    ("source", F::Parsed(swap::provider)),
    ("source_format", F::Parsed(swap::provider)),
    ("id", F::External),
    ("swap_id", F::External),
    ("swapId", F::External),
    ("swapID", F::External),
    ("provider_swap_id", F::External),
    ("boltz_id", F::External),
    ("boltzId", F::External),
    ("flow", F::Parsed(swap::flow)),
    ("type", F::Parsed(swap::flow)),
    ("swap_type", F::Parsed(swap::flow)),
    ("swapType", F::Parsed(swap::flow)),
    ("kind", F::Parsed(swap::flow)),
    ("state", SWAP_STATUS),
    ("finality", SWAP_STATUS),
    ("send_txid", F::External),
    ("sendTxid", F::External),
    ("lockup_txid", F::External),
    ("lockupTxid", F::External),
    ("receive_txid", F::External),
    ("receiveTxid", F::External),
    ("claim_txid", F::External),
    ("claimTxid", F::External),
    ("refund_txid", F::External),
    ("refundTxid", F::External),
    ("send_amount_msat", F::Amount),
    ("receive_amount_msat", F::Amount),
    ("vin", F::Obj(&RAW_FIELDS)),
    ("vout", F::IntOr(&RAW_FIELDS)),
    ("inputs", F::Obj(&RAW_FIELDS)),
    ("outputs", F::Obj(&RAW_FIELDS)),
    ("prevout", F::Obj(&RAW_FIELDS)),
    ("status", F::Or(&SWAP_STATUS, &RAW_FIELDS)),
    ("n", F::Int),
    ("index", F::Int),
    ("block_height", F::Int),
    ("block_time", F::Int),
    ("value", F::Amount),
    ("value_sats", F::Amount),
    ("amount", F::Amount),
    ("amount_msat", F::Amount),
    ("fee", F::Amount),
    ("fee_msat", F::Amount),
    ("funding_amount_msat", F::Amount),
    ("close_balance_msat", F::Amount),
    ("confirmed", F::Bool),
    ("_kassiber_verified_chain_observation", F::Bool),
    ("custody_native_fee_normalized", F::Bool),
    ("chain", F::Enum(CHAINS)),
    ("network", F::Enum(NETWORKS)),
    ("asset", F::Enum(ASSETS)),
    ("direction", F::Enum(DIRECTIONS)),
    ("tag", F::Enum(CHANNEL_TAGS)),
];
const RAW: Schema = &RAW_FIELDS;

/// The importers that stamp `_kassiber_provenance.import_source`
/// (`chain/src/lightning/lnd_wallet.rs`, `wallets/src/file_import/generic_json.rs`).
const IMPORT_SOURCES: &[&str] = &["lnd_adapter", "lnd", "core-lightning"];
const PROVENANCE: Schema = &[("import_source", F::Enum(IMPORT_SOURCES))];

/// The observers whose HTLC proofs the interpreter trusts, and others that
/// stamp payloads.
const OBSERVERS: &[&str] = &["lwk", "bitcoinrpc", "bdk", "esplora", "electrum"];
const HTLC_PROOF: Schema = &[
    ("role", F::Enum(&["claim", "refund"])),
    ("version", F::Int),
    ("input_count", F::Int),
    ("funding_txid", F::External),
    ("funding_vout", F::Int),
    ("payment_hash", F::External),
];
static SWAP_STATUS: F = F::Parsed(swap::status);

const TRANSACTION: Schema = &[
    ("id", F::Row),
    ("workspace_id", F::Row),
    ("profile_id", F::Row),
    ("wallet_id", F::Row),
    ("wallet_account_id", F::Row),
    ("account_code", F::Enum(ACCOUNT_CODES)),
    ("account_label", F::Text),
    ("wallet_label", F::Text),
    ("wallet_kind", F::Enum(WALLET_KINDS)),
    ("external_id", F::External),
    ("external_id_kind", F::Enum(EXTERNAL_ID_KINDS)),
    ("fingerprint", F::Row),
    ("occurred_at", F::Time),
    ("confirmed_at", F::Time),
    ("direction", F::Enum(DIRECTIONS)),
    ("asset", F::Enum(ASSETS)),
    ("amount", F::Amount),
    ("fee", F::Amount),
    ("amount_includes_fee", F::Bool),
    ("fiat_currency", F::Parsed(registry::fiat)),
    ("fiat_rate", F::Amount),
    ("fiat_value", F::Amount),
    ("fiat_price_source", F::Enum(PRICING_PROVIDERS)),
    ("fiat_rate_exact", F::Amount),
    ("fiat_value_exact", F::Amount),
    ("pricing_source_kind", F::Enum(PRICING_SOURCE_KINDS)),
    ("pricing_provider", F::Enum(PRICING_PROVIDERS)),
    (
        "pricing_pair",
        F::Enum(&[
            "BTC-EUR", "BTC-USD", "BTC-CHF", "BTC-GBP", "BTCEUR", "BTCUSD",
        ]),
    ),
    ("pricing_timestamp", F::Time),
    ("pricing_fetched_at", F::Time),
    ("pricing_granularity", F::Enum(PRICING_GRANULARITIES)),
    ("pricing_method", F::Enum(PRICING_METHODS)),
    ("pricing_external_ref", F::External),
    ("pricing_quality", F::Enum(PRICING_QUALITIES)),
    ("pricing_review_accepted_price", F::Amount),
    ("commercial_applied_link_id", F::Row),
    ("review_status", F::Enum(REVIEW_STATUSES)),
    ("taxability_override", F::Int),
    ("at_regime_override", F::Enum(AT_REGIMES)),
    ("at_category_override", F::Enum(AT_CATEGORIES)),
    ("kind", F::Enum(TX_KINDS)),
    ("kind_override", F::Enum(TX_KINDS)),
    ("privacy_boundary", F::Enum(PRIVACY_BOUNDARIES)),
    ("description", F::Text),
    ("counterparty", F::Text),
    ("note", F::Marker),
    ("excluded", F::Int),
    ("raw_json", F::Doc(RAW)),
    ("payment_hash", F::External),
    ("payment_hash_source", F::Enum(PAYMENT_HASH_SOURCES)),
    ("swap_refund_funding_txid", F::External),
    ("swap_refund_funding_vout", F::Int),
    ("created_at", F::Time),
    ("config_json", F::Doc(CONFIG)),
    ("wallet_config_json", F::Doc(CONFIG)),
    ("config_text_mentions_samourai", F::Bool),
    ("observation_application_revision", F::Int),
    ("observation_authority_version", F::Int),
    ("observation_fee_attribution", F::Enum(FEE_ATTRIBUTIONS)),
    ("observation_graph_hash", F::Digest),
    ("observation_quantity_hash", F::Digest),
    ("observation_observed_at", F::Time),
    (
        "observation_observer_kinds_json",
        F::DocList(&OBSERVER_LIST),
    ),
];

const PROFILE: Schema = &[
    ("id", F::Row),
    ("workspace_id", F::Row),
    ("label", F::Text),
    ("fiat_currency", F::Parsed(registry::fiat)),
    ("tax_country", F::Parsed(registry::country)),
    ("tax_long_term_days", F::Int),
    ("gains_algorithm", F::Parsed(registry::method)),
    ("require_coarse_review", F::Int),
    ("bitcoin_rail_carrying_value", F::Int),
    ("cost_basis_pool_scope", F::Parsed(registry::pool_scope)),
    ("journal_input_version", F::Int),
    ("last_processed_input_version", F::Int),
    ("required_journal_semantics_version", F::Int),
    ("journal_semantics_version", F::Int),
    ("last_processed_at", F::Time),
    ("last_processed_tx_count", F::Int),
    ("swap_candidate_count", F::Int),
    ("created_at", F::Time),
];

const WALLET: Schema = &[
    ("id", F::Row),
    ("label", F::Text),
    ("kind", F::Enum(WALLET_KINDS)),
    ("config_json", F::Doc(CONFIG)),
    ("config_text_mentions_samourai", F::Bool),
    ("wallet_account_id", F::Row),
    ("account_code", F::Enum(ACCOUNT_CODES)),
    ("account_label", F::Text),
];

const RATE_ROW: Schema = &[
    ("asset", F::Enum(ASSETS)),
    ("amount", F::Amount),
    ("fiat_rate", F::Amount),
    ("fiat_rate_exact", F::Amount),
    ("fiat_value", F::Amount),
    ("fiat_value_exact", F::Amount),
];

const EVIDENCE: Schema = &[
    ("review_id", F::Row),
    ("term_kind", F::Enum(TERM_KINDS)),
    (
        "residual_classification",
        F::Obj(&[("classification", F::Enum(RESIDUALS))]),
    ),
];

const HEADER: Schema = &[
    ("id", F::Row),
    ("lineage_id", F::Row),
    ("workspace_id", F::Row),
    ("profile_id", F::Row),
    ("revision", F::Int),
    ("component_type", F::Enum(COMPONENT_TYPES)),
    ("conservation_mode", F::Enum(CONSERVATION_MODES)),
    ("state", F::Enum(COMPONENT_STATES)),
    ("evidence_kind", F::Enum(EVIDENCE_KINDS)),
    ("evidence_grade", F::Enum(EVIDENCE_GRADES)),
    ("evidence_json", F::Doc(EVIDENCE)),
    ("conversion_policy", F::Enum(POLICIES)),
    ("conversion_reviewed", F::Int),
    ("conversion_metadata_json", F::Doc(&[])),
    ("expected_leg_count", F::Int),
    ("expected_allocation_count", F::Int),
    ("expected_economic_term_count", F::Int),
    ("expected_evidence_count", F::Int),
    ("authored_source", F::Enum(AUTHORED_SOURCES)),
    ("notes", F::Marker),
    ("change_reason", F::Text),
    ("supersedes_component_id", F::Row),
    ("superseded_by_component_id", F::Row),
    ("activated_at", F::Time),
    ("superseded_at", F::Time),
    ("created_at", F::Time),
];

const LEG: Schema = &[
    ("id", F::Row),
    ("component_id", F::Row),
    ("workspace_id", F::Row),
    ("profile_id", F::Row),
    ("ordinal", F::Int),
    ("role", F::Enum(ROLES)),
    ("rail", F::Enum(RAILS)),
    ("chain", F::Enum(CHAINS)),
    ("network", F::Enum(NETWORKS)),
    ("asset", F::Enum(ASSETS)),
    ("exposure", F::Enum(RAILS)),
    ("conservation_unit", F::Enum(UNITS)),
    ("amount_msat", F::Amount),
    ("valuation_unit", F::Enum(UNITS)),
    ("valuation_amount", F::Amount),
    ("occurred_at", F::Time),
    ("transaction_id", F::Row),
    ("anchor_transaction_id", F::Row),
    ("wallet_id", F::Row),
    ("location_ref", F::LocationRef),
    ("notes", F::Marker),
    ("created_at", F::Time),
];

const ALLOCATION: Schema = &[
    ("id", F::Row),
    ("component_id", F::Row),
    ("workspace_id", F::Row),
    ("profile_id", F::Row),
    ("ordinal", F::Int),
    ("source_leg_id", F::Row),
    ("sink_leg_id", F::Row),
    ("source_amount_msat", F::Amount),
    ("sink_amount_msat", F::Amount),
    ("created_at", F::Time),
];

const TERM: Schema = &[
    ("id", F::Row),
    ("component_id", F::Row),
    ("workspace_id", F::Row),
    ("profile_id", F::Row),
    ("ordinal", F::Int),
    ("source_leg_id", F::Row),
    ("target_leg_id", F::Row),
    ("term_kind", F::Enum(TERM_KINDS)),
    ("legacy_source_id", F::Row),
    ("source_row_hash", F::Digest),
    ("review_kind", F::Enum(AUTHORED_SOURCES)),
    ("tax_policy", F::Enum(POLICIES)),
    ("reviewed_source_amount_msat", F::Amount),
    ("swap_fee_msat", F::Amount),
    (
        "swap_fee_kind",
        F::Enum(&["network", "service", "spread", "none", "mixed"]),
    ),
    ("confidence_at_review", F::Enum(CONFIDENCES)),
    ("review_source", F::Enum(AUTHORED_SOURCES)),
    ("review_notes", F::Text),
    ("payout_asset", F::Enum(ASSETS)),
    ("payout_amount_msat", F::Amount),
    ("payout_occurred_at", F::Time),
    ("payout_fiat_value_exact", F::Amount),
    ("payout_external_id", F::External),
    ("counterparty", F::Text),
    ("created_at", F::Time),
];

const COMMITMENT: Schema = &[
    ("id", F::Digest),
    ("component_id", F::Row),
    ("workspace_id", F::Row),
    ("profile_id", F::Row),
    ("ordinal", F::Int),
    ("quantity_hash", F::Digest),
    ("detail_hash", F::Digest),
    ("created_at", F::Time),
];

const EVENT: Schema = &[
    ("chain", F::Enum(CHAINS)),
    ("network", F::Enum(NETWORKS)),
    ("native_event_id", F::External),
    // A wallet id or a namespace word: a local identity.
    ("native_namespace", F::Row),
];

const QUANTITY: Schema = &[
    ("amount_includes_fee", F::Bool),
    ("amount_msat", F::Amount),
    ("asset", F::Enum(ASSETS)),
    ("direction", F::Enum(DIRECTIONS)),
    ("event", F::Obj(EVENT)),
    ("fee_msat", F::Amount),
    ("schema_version", F::Int),
    ("wallet_id", F::Row),
];

const PAYLOAD: Schema = &[
    ("confirmed_at", F::Time),
    ("fingerprint", F::Digest),
    ("kind", F::Enum(TX_KINDS)),
    ("occurred_at", F::Time),
    ("payment_hash", F::External),
    ("payment_hash_source", F::Enum(PAYMENT_HASH_SOURCES)),
    ("quantity", F::Obj(QUANTITY)),
    ("raw_json", F::Doc(RAW)),
    ("schema_version", F::Int),
    ("swap_refund_funding_txid", F::External),
    ("swap_refund_funding_vout", F::Int),
];

const EVIDENCE_SNAPSHOT: Schema = &[
    ("subject_id", F::Row),
    ("detail_hash", F::Digest),
    ("quantity_hash", F::Digest),
    ("payload_json", F::Doc(PAYLOAD)),
    ("created_at", F::Time),
];

const ANCHOR_WALLET: Schema = &[
    ("id", F::Row),
    ("kind", F::Enum(WALLET_KINDS)),
    ("config_json", F::Doc(CONFIG)),
    ("config_text_mentions_samourai", F::Bool),
];

const RECEIPT_SOURCE: Schema = &[
    ("profile_id", F::Row),
    ("wallet_id", F::Row),
    ("chain", F::Enum(CHAINS)),
    ("network", F::Enum(NETWORKS)),
    ("script_pubkey", F::External),
    ("source", F::Enum(OWNED_SOURCES)),
    ("deprecated", F::Bool),
    ("branch_index", F::Int),
    ("branch_label", F::Text),
    ("address_index", F::Int),
];

const CHANNEL_RECORD: Schema = &[
    ("txid", F::External),
    ("outpoint", F::External),
    ("tag", F::Enum(CHANNEL_TAGS)),
    ("wallet_id", F::Row),
    ("channel_id", F::External),
    ("amount_msat", F::Amount),
    ("raw_json", F::Doc(RAW)),
    ("config_json", F::Doc(CONFIG)),
    ("chain", F::Enum(CHAINS)),
    ("network", F::Enum(NETWORKS)),
];

const LOAN_LEG: Schema = &[("transaction_id", F::Row), ("role", F::Enum(LOAN_ROLES))];

const MIGRATION_ISSUE: Schema = &[
    ("id", F::Row),
    ("workspace_id", F::Row),
    ("profile_id", F::Row),
    (
        "legacy_table",
        F::Enum(&[
            "transaction_pairs",
            "transfer_pairs",
            "custody_gap_reviews",
            "pair_fee_allocations",
        ]),
    ),
    ("legacy_source_id", F::Row),
    (
        "issue_code",
        F::Enum(&[
            "legacy_pair_fee_review",
            "missing_evidence",
            "conflict",
            "unsupported",
        ]),
    ),
    ("transaction_ids_json", F::DocList(&ROW_LIST)),
    ("resolved_at", F::Time),
    ("created_at", F::Time),
    ("updated_at", F::Time),
];

const QUARANTINE: Schema = &[
    ("transaction_id", F::Row),
    ("workspace_id", F::Row),
    ("profile_id", F::Row),
    ("reason", F::Enum(QUARANTINE_REASONS)),
    ("detail_json", F::Doc(&[])),
];

const GAP_REVIEW: Schema = &[
    ("id", F::Row),
    ("workspace_id", F::Row),
    ("profile_id", F::Row),
    ("gap_id", F::Row),
    ("revision", F::Int),
    ("candidate_fingerprint", F::Digest),
    ("action", F::Enum(GAP_ACTIONS)),
    ("event_kind", F::Enum(TX_KINDS)),
    ("component_id", F::Row),
    ("authored_source", F::Enum(AUTHORED_SOURCES)),
    ("reason", F::Text),
    ("snapshot_json", F::Doc(&[])),
    ("created_at", F::Time),
];

const PAIR_DISMISSAL: Schema = &[
    ("id", F::Row),
    ("workspace_id", F::Row),
    ("profile_id", F::Row),
    ("out_transaction_id", F::Row),
    ("in_transaction_id", F::Row),
    ("reason", F::Text),
    ("created_at", F::Time),
    ("expires_at", F::Time),
];

const BINDING: Schema = &[
    ("profile_id", F::Row),
    ("environment_id", F::Row),
    ("environment", F::Enum(ENVIRONMENTS)),
    ("revision", F::Int),
    ("chain_instance_id", F::External),
    ("domains_json", F::Doc(&[])),
    ("acknowledgements_json", F::Doc(&[])),
    ("inventory_digest", F::Digest),
    ("created_at", F::Time),
];

/// Only network facts of an observer or policy reference: no observer
/// state, descriptors or policy material.
const REFERENCE_PAYLOAD: Schema = &[
    ("chain", F::Enum(CHAINS)),
    ("network", F::Enum(NETWORKS)),
    ("asset", F::Enum(ASSETS)),
    ("genesis_hash", F::External),
    ("chain_instance_id", F::External),
];

const REFERENCE: Schema = &[
    ("table", F::Enum(REFERENCE_TABLES)),
    ("chain", F::Enum(CHAINS)),
    ("network", F::Enum(NETWORKS)),
    ("payload_json", F::Doc(REFERENCE_PAYLOAD)),
    ("private_material_present", F::Bool),
    ("source_present", F::Bool),
];

const OBSERVATION: Schema = &[
    ("id", F::Row),
    ("wallet_id", F::Row),
    ("asset", F::Enum(ASSETS)),
    ("raw_json", F::Doc(RAW)),
    ("wallet_kind", F::Enum(WALLET_KINDS)),
    ("wallet_config_json", F::Doc(CONFIG)),
];

const OWNED_MATCH: Schema = &[
    ("wallet_id", F::Row),
    ("wallet_label", F::Text),
    ("account", F::Enum(ACCOUNT_CODES)),
    ("chain", F::Enum(CHAINS)),
    ("network", F::Enum(NETWORKS)),
    ("source", F::Enum(OWNED_SOURCES)),
    ("wallet_kind", F::Enum(WALLET_KINDS)),
    ("branch_label", F::Text),
];

/// Every section of the snapshot and the schema of its rows.
pub const SECTIONS: &[(&str, Schema)] = &[
    ("profile", PROFILE),
    ("wallets", WALLET),
    ("transactions", TRANSACTION),
    ("rate_rows", RATE_ROW),
    ("anchors", TRANSACTION),
    ("anchor_wallets", ANCHOR_WALLET),
    ("revision_headers", HEADER),
    ("receipt_sources", RECEIPT_SOURCE),
    ("channel_records", CHANNEL_RECORD),
    ("loan_legs", LOAN_LEG),
    ("evidence_snapshots", EVIDENCE_SNAPSHOT),
    ("migration_issues", MIGRATION_ISSUE),
    ("import_duplicate_quarantines", QUARANTINE),
    ("reset_hold_quarantines", QUARANTINE),
    ("gap_reviews", GAP_REVIEW),
    ("pair_dismissals", PAIR_DISMISSAL),
    ("gap_worklist.rows", TRANSACTION),
    ("components.header", HEADER),
    ("components.legs", LEG),
    ("components.allocations", ALLOCATION),
    ("components.economic_terms", TERM),
    ("components.commitments", COMMITMENT),
    ("book_network.binding", BINDING),
    ("book_network.observations", OBSERVATION),
    ("book_network.references", REFERENCE),
];

pub fn section(name: &str) -> Option<Schema> {
    SECTIONS.iter().find(|(n, _)| *n == name).map(|(_, s)| *s)
}

/// Every field name the input projection classifies: in any section, any
/// document or nested object, and the ownership index's sections and match
/// records. The interpreter coverage test checks the keys the custody
/// interpreters read against it.
pub fn field_names() -> BTreeSet<&'static str> {
    fn walk(schema: Schema, out: &mut BTreeSet<&'static str>, seen: &mut BTreeSet<usize>) {
        if !seen.insert(schema.as_ptr() as usize) {
            return;
        }
        for (name, class) in schema {
            out.insert(name);
            nested(class, out, seen);
        }
    }
    fn nested(class: &F, out: &mut BTreeSet<&'static str>, seen: &mut BTreeSet<usize>) {
        match class {
            F::Doc(schema) | F::Obj(schema) | F::IntOr(schema) | F::Or(_, schema) => {
                walk(schema, out, seen)
            }
            F::List(class) | F::DocList(class) => nested(class, out, seen),
            _ => {}
        }
    }
    let mut out = BTreeSet::new();
    let mut seen = BTreeSet::new();
    for (name, schema) in SECTIONS {
        out.extend(name.split('.'));
        walk(schema, &mut out, &mut seen);
    }
    walk(OWNED_MATCH, &mut out, &mut seen);
    out.extend(OWNED_SECTIONS);
    // Scoped ownership entries: `{key: [chain, network, id], matches: [...]}`.
    out.extend(["key", "matches"]);
    out.extend([
        "schema_version",
        "operation",
        "as_of",
        "owned_index",
        "ownership_warnings",
        "anchors",
        "components",
        "gap_worklist",
        "book_network",
    ]);
    out
}

const OWNED_SECTIONS: &[&str] = &[
    "by_script",
    "by_outpoint",
    "by_address",
    "txid_wallets",
    "by_scoped_outpoint",
    "txid_wallets_by_scope",
];

/// Fields of `value` (rows of a section, or one document) that `schema`
/// does not classify, recursively through nested objects and documents,
/// as dotted paths. The coverage test asserts this is empty.
pub fn unclassified(schema: Schema, value: &Value) -> Vec<String> {
    fn walk(schema: Schema, value: &Value, path: &str, out: &mut Vec<String>) {
        match value {
            Value::Array(items) => {
                for item in items {
                    walk(schema, item, path, out);
                }
            }
            Value::Object(fields) => {
                for (key, value) in fields {
                    let here = format!("{path}.{key}");
                    match schema.iter().find(|(name, _)| name == key) {
                        None => out.push(here),
                        Some((_, class)) => nested(*class, value, &here, out),
                    }
                }
            }
            _ => {}
        }
    }
    fn nested(class: F, value: &Value, path: &str, out: &mut Vec<String>) {
        match (class, value) {
            (
                F::Obj(schema) | F::Doc(schema) | F::IntOr(schema) | F::Or(_, schema),
                Value::Object(_) | Value::Array(_),
            ) => walk(schema, value, path, out),
            (F::Doc(schema), Value::String(text)) => {
                if let Ok(parsed) = serde_json::from_str::<Value>(text) {
                    if parsed.is_object() || parsed.is_array() {
                        walk(schema, &parsed, path, out);
                    }
                }
            }
            (F::List(item), Value::Array(items)) => {
                for value in items {
                    nested(*item, value, path, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(schema, value, "", &mut out);
    out.sort();
    out.dedup();
    out
}
