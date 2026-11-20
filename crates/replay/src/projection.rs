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
use crate::pseudonym::Pseudonymizer;
use crate::registry;
use crate::swap;
use crate::typed;
use kassiber_pyjson::{Key, PyStr, Value as Py};
use serde_json::{Map, Value};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

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

/// Codec wire tags a value may carry, and how their content is checked.
const CODEC_TAGS: &[&str] = &["$decimal", "$float"];

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
    "hard_fork",
    "mining_reward",
    "routing_income",
    "theft",
    "withdraw",
    "sent",
    "transfer_out",
    "unknown",
    "samourai_deposit",
    "samourai_tx0",
    "samourai_premix",
    "samourai_postmix",
    "samourai_badbank",
    "whirlpool_tx0",
    "whirlpool_mix",
    "premix",
    "postmix",
    "badbank",
    "coinjoin",
    "lnd_invoice",
    "lnd_pay",
    "cln_invoice",
    "cln_pay",
    "ln_invoice",
    "ln_pay",
    "revenue",
    "sale",
    "exchange_buy",
    "customer_payment",
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
    "samourai-whirlpool",
    "whirlpool",
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
pub(crate) const PRICING_SOURCE_KINDS: &[&str] = &[
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
    "manual_rate_cache",
    "fmv_provider",
];
pub(crate) const PRICING_QUALITIES: &[&str] = &[
    "exact",
    "coarse",
    "estimated",
    "derived",
    "fallback",
    "missing",
    "approximate",
    "hourly",
    "daily",
    "coarse_fallback",
    "provider_sample",
];
/// `fiat_price_source`: a provider, or the rates cache the journal pricing
/// stamps (`journals/src/prebuild/pricing.rs`).
pub(crate) const PRICE_SOURCES: &[&str] = &[
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
    "rates_cache",
];
pub(crate) const PRICING_GRANULARITIES: &[&str] = &[
    "exact",
    "minute",
    "hourly",
    "hour",
    "daily",
    "day",
    "transaction",
];
pub(crate) const PRICING_PROVIDERS: &[&str] = &[
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
pub(crate) const PRICING_PAIRS: &[&str] = &[
    "BTC-EUR", "BTC-USD", "BTC-CHF", "BTC-GBP", "BTCEUR", "BTCUSD",
];
pub(crate) const PRICING_METHODS: &[&str] = &[
    "quarantine_price_override",
    "import",
    "manual",
    "nearest",
    "interpolated",
    "close",
    "rates_cache",
    "unsupported_price_range",
    "direct_payout_review",
    "carrying_value",
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
const PRIVACY_BOUNDARIES: &[&str] = &[
    "coinjoin",
    "payjoin",
    "whirlpool",
    "none",
    "mixed",
    "collaborative",
    "payment_in_coinjoin",
    "sweep",
];
const PAYMENT_HASH_SOURCES: &[&str] = &[
    "invoice",
    "import",
    "native",
    "provider",
    "derived",
    "lnd",
    "core_lightning",
    "chain_script_unique_outpoint",
    "chain_script",
    "boltz",
    "boltz-regtest",
    "bullbitcoin",
    "generic_ledger",
    "importer",
    "importer_backfill",
];
const ACCOUNT_CODES: &[&str] = &["treasury"];
const FEE_ATTRIBUTIONS: &[&str] = &[
    "sender",
    "receiver",
    "wallet",
    "none",
    "shared",
    "unknown",
    "exact",
    "implicit_wallet_delta",
];
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
const CONSERVATION_MODES: &[&str] = &["quantity", "value", "none", "conversion"];
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
    "direct_swap_payout",
    "custody_component",
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
/// A term's `review_kind`: the pair kind it was reviewed as
/// (`transfers/src/pair_authoring.rs` KINDS), read for its transfer kind.
const REVIEW_KINDS: &[&str] = &[
    "manual",
    "coinjoin",
    "whirlpool",
    "chain-swap",
    "peg-in",
    "peg-out",
    "reverse-submarine-swap",
    "submarine-swap",
    "swap-refund",
    "custody-component",
];
/// A term's `review_source`: the pair source (`pair_authoring.rs`).
const REVIEW_SOURCES: &[&str] = &["manual", "bulk_exact", "bulk_selected", "rule_auto"];
/// A gap review's `event_kind` (`transfers/src/gap_review.rs`).
const GAP_EVENTS: &[&str] = &["review_decision", "bridge_reopened"];
const GAP_ACTIONS: &[&str] = &[
    "accept",
    "dismiss",
    "defer",
    "reopen",
    "hold",
    "resolve",
    "dismissed",
    "resolved",
];
const OWNED_SOURCES: &[&str] = &[
    "wallet_utxos",
    "descriptor",
    "address",
    "receipt",
    "silent_payments",
    "import",
    "derived",
];
const SAMOURAI_SECTIONS: &[&str] = &["deposit", "premix", "postmix", "badbank", "ricochet"];
/// A Samourai source set's wallet role (`wallets/src/file_import/samourai.rs`).
const SAMOURAI_ROLES: &[&str] = &["parent", "child"];
const CHANNEL_TAGS: &[&str] = &[
    "open",
    "close",
    "funding",
    "sweep",
    "force_close",
    "splice",
    "channel_open",
    "channel_close",
];

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
    ("section", F::Enum(SAMOURAI_SECTIONS)),
];

/// The on-chain and Lightning payload fields the graph, confirmation,
/// network and channel interpreters read from `raw_json`.
static RAW_FIELDS: [(&str, F); 85] = [
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
    // The observer's chain instance, which the network gate compares with
    // the wallet configuration's (custody/src/gates/network.rs).
    ("chain_instance_id", F::External),
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
    ("fiat_price_source", F::Enum(PRICE_SOURCES)),
    ("fiat_rate_exact", F::Amount),
    ("fiat_value_exact", F::Amount),
    ("pricing_source_kind", F::Enum(PRICING_SOURCE_KINDS)),
    ("pricing_provider", F::Enum(PRICING_PROVIDERS)),
    ("pricing_pair", F::Enum(PRICING_PAIRS)),
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
    ("review_kind", F::Enum(REVIEW_KINDS)),
    ("tax_policy", F::Enum(POLICIES)),
    ("reviewed_source_amount_msat", F::Amount),
    ("swap_fee_msat", F::Amount),
    (
        "swap_fee_kind",
        F::Enum(&["network", "service", "spread", "none", "mixed", "combined"]),
    ),
    ("confidence_at_review", F::Enum(CONFIDENCES)),
    ("review_source", F::Enum(REVIEW_SOURCES)),
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
    ("event_kind", F::Enum(GAP_EVENTS)),
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

/// The class of the field at `path` (a field, then fields of nested
/// documents and objects) in section `name`.
pub fn class_of(name: &str, path: &[&str]) -> Option<F> {
    fn at(schema: Schema, path: &[&str]) -> Option<F> {
        let (first, rest) = path.split_first()?;
        let (_, class) = schema.iter().find(|(n, _)| n == first)?;
        if rest.is_empty() {
            return Some(*class);
        }
        match class {
            F::Doc(schema) | F::Obj(schema) | F::IntOr(schema) | F::Or(_, schema) => {
                at(schema, rest)
            }
            _ => None,
        }
    }
    at(section(name)?, path)
}

/// Whether the schema keeps `value` as written in the field at `path`.
pub fn kept(name: &str, path: &[&str], value: &str) -> bool {
    let leaf = |class: &F| match class {
        F::Enum(values) => values.contains(&value),
        F::Parsed(parse) => parse(value).as_deref() == Some(value),
        _ => false,
    };
    match class_of(name, path) {
        Some(F::Or(scalar, _)) => leaf(scalar),
        Some(F::List(class) | F::DocList(class)) => leaf(class),
        Some(class) => leaf(&class),
        None => false,
    }
}

/// Values the engine compares a field against (or writes and reads back),
/// each with a source file that names it: the schema must keep every one,
/// or a projected book computes differently and the export is refused.
/// `tests/projection_coverage.rs` checks both.
#[rustfmt::skip]
pub const ENGINE_CONSTANTS: &[(&str, &[&str], &[&str], &str)] = &[
    ("transactions", &["pricing_quality"], &["coarse_fallback"],
        "custody/src/coarse_review.rs"),
    ("transactions", &["pricing_quality"], &["coarse_fallback", "provider_sample", "exact"],
        "journals/src/prebuild/pricing.rs"),
    ("transactions", &["pricing_source_kind"], &["manual_rate_cache", "fmv_provider"],
        "journals/src/prebuild/pricing.rs"),
    ("transactions", &["fiat_price_source"], &["rates_cache"],
        "journals/src/prebuild/pricing.rs"),
    ("transactions", &["pricing_method"], &["unsupported_price_range"],
        "custody/src/finalized_projection/valuation.rs"),
    ("transactions", &["pricing_method"], &["direct_payout_review"],
        "custody/src/finalized_projection/payouts.rs"),
    ("transactions", &["observation_fee_attribution"], &["exact", "implicit_wallet_delta"],
        "custody/src/interpretation/pipeline.rs"),
    ("transactions", &["privacy_boundary"], &["coinjoin", "collaborative", "payjoin", "payment_in_coinjoin", "sweep"],
        "custody/src/interpretation/privacy.rs"),
    ("transactions", &["payment_hash_source"], &["lnd", "core_lightning"],
        "custody/src/interpretation/recorded.rs"),
    ("transactions", &["payment_hash_source"], &["chain_script_unique_outpoint", "chain_script", "boltz", "boltz-regtest", "bullbitcoin", "generic_ledger", "importer", "importer_backfill"],
        "custody/src/interpretation/matcher.rs"),
    ("transactions", &["kind"], &["lnd_invoice", "lnd_pay", "cln_invoice", "cln_pay", "ln_invoice", "ln_pay"],
        "custody/src/interpretation/recorded.rs"),
    ("transactions", &["kind"], &["samourai_deposit", "samourai_tx0", "samourai_premix", "samourai_postmix", "samourai_badbank", "whirlpool_tx0", "whirlpool_mix", "premix", "postmix", "badbank", "coinjoin"],
        "custody/src/gaps/normalize.rs"),
    ("transactions", &["kind"], &["buy", "airdrop", "hardfork", "hard_fork", "income", "interest", "lending_interest", "mining", "mining_reward", "routing_income", "staking", "wages", "withdrawal", "withdraw", "send", "sent", "transfer", "transfer_out", "unknown"],
        "journals/src/quarantine.rs"),
    ("transactions", &["kind"], &["lost", "stolen", "theft"],
        "tax-countries/src/lib.rs"),
    ("transactions", &["wallet_kind"], &["samourai", "samourai-whirlpool", "whirlpool"],
        "custody/src/gaps/normalize.rs"),
    ("transactions", &["raw_json", "_kassiber_provenance", "import_source"], &["lnd", "core-lightning"],
        "custody/src/interpretation/recorded.rs"),
    ("transactions", &["raw_json", "source"], &["bullbitcoin_wallet_csv"],
        "custody/src/gates/network.rs"),
    ("book_network.observations", &["raw_json", "source"], &["bullbitcoin_wallet_csv"],
        "custody/src/gates/network.rs"),
    ("channel_records", &["tag"], &["channel_open", "channel_close"],
        "custody/src/interpretation/channels.rs"),
    ("wallets", &["config_json", "samourai", "role"], &["parent", "child"],
        "custody/src/gaps/normalize.rs"),
    ("wallets", &["config_json", "samourai", "section"], &["deposit", "premix", "postmix", "badbank"],
        "custody/src/interpretation/samourai.rs"),
    ("components.header", &["conservation_mode"], &["quantity", "conversion"],
        "custody/src/components/mod.rs"),
    ("components.economic_terms", &["term_kind"], &["transaction_pair", "direct_swap_payout"],
        "custody/src/quantity/reviewed.rs"),
    ("components.economic_terms", &["review_kind"], &["manual", "coinjoin", "whirlpool", "chain-swap", "peg-in", "peg-out", "reverse-submarine-swap", "submarine-swap", "swap-refund"],
        "transfers/src/pair_authoring.rs"),
    ("components.economic_terms", &["swap_fee_kind"], &["combined"],
        "transfers/src/payout_create.rs"),
    ("gap_reviews", &["action"], &["dismissed"],
        "custody/src/gaps/mod.rs"),
    ("gap_reviews", &["action"], &["resolved"],
        "transfers/src/gap_review_store.rs"),
    ("gap_reviews", &["event_kind"], &["review_decision", "bridge_reopened"],
        "custody/src/gaps/mod.rs"),
];

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

/// One projection run: the pseudonymizer, every identifier and enum value
/// it replaced (so the book's own outputs map the same way), and every value
/// it generated (so recomputed outputs can be checked against them).
pub struct Projection<'a> {
    p: &'a Pseudonymizer,
    replaced: RefCell<BTreeMap<String, String>>,
    generated: RefCell<BTreeSet<String>>,
}

impl<'a> Projection<'a> {
    pub fn new(p: &'a Pseudonymizer) -> Self {
        Self {
            p,
            replaced: RefCell::new(BTreeMap::new()),
            generated: RefCell::new(BTreeSet::new()),
        }
    }

    /// Every identifier and enum value the projection replaced, with its
    /// replacement.
    pub fn replacements(&self) -> BTreeMap<String, String> {
        self.replaced.borrow().clone()
    }

    /// Every identifier and enum pseudonym the projection generated.
    pub fn generated(&self) -> BTreeSet<String> {
        self.generated.borrow().clone()
    }

    /// The keyed mapping of an identifier, local or outside.
    pub fn id(&self, text: &str) -> String {
        let id = self.p.id(text);
        self.replaced
            .borrow_mut()
            .insert(text.to_owned(), id.clone());
        self.generated.borrow_mut().insert(id.clone());
        id
    }

    /// Authored text: a label pseudonym, never recorded.
    fn text(&self, text: &str) -> String {
        if text.is_empty() {
            return String::new();
        }
        self.p.label(text)
    }

    /// An enum value outside its list: a label, recorded so outputs map.
    pub fn enum_value(&self, text: &str) -> String {
        let label = self.p.label(text);
        self.replaced
            .borrow_mut()
            .insert(text.to_owned(), label.clone());
        self.generated.borrow_mut().insert(label.clone());
        label
    }

    pub fn snapshot(&self, snapshot: &Value) -> Result<Value, String> {
        let Value::Object(fields) = snapshot else {
            return Err("custody snapshot is not an object".into());
        };
        let rows = |name: &str, value: &Value| -> Value {
            let schema = section(name).expect("schema");
            match value {
                Value::Array(rows) => {
                    Value::Array(rows.iter().map(|r| self.object(schema, r)).collect())
                }
                Value::Null => Value::Null,
                other => self.object(schema, other),
            }
        };
        let mut out = Map::new();
        for (name, value) in fields {
            let projected = match name.as_str() {
                "schema_version" => self.value(F::Int, value),
                "operation" => self.value(F::Enum(&["custody"]), value),
                "as_of" => self.value(F::Time, value),
                "profile" => self.object(PROFILE, value),
                "owned_index" => self.owned_index(value),
                "ownership_warnings" => match value {
                    Value::Array(items) => {
                        Value::Array(items.iter().map(|v| self.value(F::Text, v)).collect())
                    }
                    _ => Value::Array(Vec::new()),
                },
                "anchors" => match value {
                    Value::Object(rows) => Value::Object(
                        rows.iter()
                            .map(|(id, row)| {
                                let id = self.id(id);
                                (id, self.object(TRANSACTION, row))
                            })
                            .collect(),
                    ),
                    _ => Value::Object(Map::new()),
                },
                "components" => match value {
                    Value::Array(components) => Value::Array(
                        components
                            .iter()
                            .map(|component| {
                                let mut out = Map::new();
                                for part in [
                                    "header",
                                    "legs",
                                    "allocations",
                                    "economic_terms",
                                    "commitments",
                                ] {
                                    let value = component.get(part).unwrap_or(&Value::Null);
                                    out.insert(
                                        part.into(),
                                        rows(&format!("components.{part}"), value),
                                    );
                                }
                                Value::Object(out)
                            })
                            .collect(),
                    ),
                    _ => Value::Array(Vec::new()),
                },
                "gap_worklist" => {
                    let mut out = Map::new();
                    out.insert(
                        "large_book".into(),
                        self.value(
                            F::Bool,
                            value.get("large_book").unwrap_or(&Value::Bool(false)),
                        ),
                    );
                    out.insert(
                        "rows".into(),
                        rows(
                            "gap_worklist.rows",
                            value.get("rows").unwrap_or(&Value::Array(Vec::new())),
                        ),
                    );
                    Value::Object(out)
                }
                "book_network" => match value {
                    Value::Object(network) => {
                        let mut out = Map::new();
                        for part in ["binding", "observations", "references"] {
                            if let Some(value) = network.get(part) {
                                out.insert(
                                    part.into(),
                                    rows(&format!("book_network.{part}"), value),
                                );
                            }
                        }
                        Value::Object(out)
                    }
                    _ => Value::Null,
                },
                name if section(name).is_some() => rows(name, value),
                other => return Err(format!("unclassified snapshot section {other}")),
            };
            out.insert(name.clone(), projected);
        }
        Ok(Value::Object(out))
    }

    /// One object by its schema: unnamed fields dropped, kept names emitted
    /// from the schema.
    pub fn object(&self, schema: Schema, value: &Value) -> Value {
        match value {
            Value::Object(fields) => Value::Object(
                fields
                    .iter()
                    .filter_map(|(key, value)| {
                        let (name, class) = schema.iter().find(|(name, _)| name == key)?;
                        Some(((*name).to_owned(), self.value(*class, value)))
                    })
                    .collect(),
            ),
            Value::Array(items) => {
                Value::Array(items.iter().map(|v| self.object(schema, v)).collect())
            }
            Value::Null => Value::Null,
            other => Value::String(self.text(&other.to_string())),
        }
    }

    /// One value by its class.
    pub fn value(&self, class: F, value: &Value) -> Value {
        match (class, value) {
            (_, Value::Null) => Value::Null,
            (F::Obj(schema), value) => self.object(schema, value),
            (F::IntOr(schema) | F::Or(_, schema), Value::Array(_) | Value::Object(_)) => {
                self.object(schema, value)
            }
            (F::Or(scalar, _), value) => self.value(*scalar, value),
            (F::Doc(schema), Value::Object(_) | Value::Array(_)) => self.object(schema, value),
            (F::Doc(schema), Value::String(text)) => Value::String(self.document(schema, text)),
            (F::DocList(item), Value::String(text)) => {
                Value::String(self.document_list(*item, text))
            }
            (F::DocList(item), Value::Array(items)) => {
                Value::Array(items.iter().map(|v| self.value(*item, v)).collect())
            }
            (F::List(item), Value::Array(items)) => {
                Value::Array(items.iter().map(|v| self.value(*item, v)).collect())
            }
            (_, Value::Array(items)) => {
                Value::Array(items.iter().map(|v| self.value(class, v)).collect())
            }
            (_, Value::Object(tagged)) => self.tagged(class, tagged),
            (_, Value::Bool(b)) => match class {
                F::Bool | F::Int | F::Amount | F::Enum(_) | F::Parsed(_) | F::IntOr(_) => {
                    Value::Bool(*b)
                }
                _ => Value::String(self.text(&b.to_string())),
            },
            (_, Value::Number(n)) => match class {
                F::Amount | F::Int | F::Bool | F::IntOr(_) | F::Enum(_) | F::Parsed(_) => {
                    typed::number(n).unwrap_or(Value::Null)
                }
                // Integer ids and years are typed values, not spellings.
                F::Row | F::Time => match n.as_i64() {
                    Some(n) => Value::from(n),
                    None => Value::String(self.id(&n.to_string())),
                },
                _ => Value::String(self.text(&n.to_string())),
            },
            (_, Value::String(text)) => Value::String(self.string(class, text)),
        }
    }

    /// A codec-tagged value: only `$decimal`/`$float` around a parsed amount.
    fn tagged(&self, class: F, tagged: &Map<String, Value>) -> Value {
        match tagged.iter().next() {
            Some((tag, Value::String(text))) if tagged.len() == 1 && matches!(class, F::Amount) => {
                match (CODEC_TAGS.iter().find(|t| **t == tag), typed::amount(text)) {
                    (Some(tag), Some(amount)) => {
                        let mut out = Map::new();
                        out.insert((*tag).to_owned(), Value::String(amount));
                        Value::Object(out)
                    }
                    _ => Value::Null,
                }
            }
            _ => Value::Null,
        }
    }

    /// One string by its class: parsed and printed again, mapped, or
    /// pseudonymized. Never copied.
    fn string(&self, class: F, text: &str) -> String {
        if text.is_empty() {
            return String::new();
        }
        if let F::Or(scalar, _) = class {
            return self.string(*scalar, text);
        }
        let parsed = match class {
            F::Amount => typed::amount(text),
            F::Time => typed::time(text),
            F::Int | F::IntOr(_) => typed::int(text),
            F::Bool => typed::boolean(text).map(str::to_owned),
            F::Enum(values) => values.iter().find(|v| **v == text).map(|v| (*v).to_owned()),
            F::Parsed(parse) => parse(text),
            F::Marker => MARKERS
                .iter()
                .find(|(marker, _)| *marker == text)
                .map(|(marker, _)| (*marker).to_owned()),
            _ => None,
        };
        if let Some(parsed) = parsed {
            return parsed;
        }
        match class {
            F::Row | F::External | F::Digest => self.id(text),
            F::LocationRef => match text.split_once(':') {
                Some((prefix, rest)) => match LOCAL_REFS.iter().find(|p| **p == prefix) {
                    Some(prefix) => format!("{prefix}:{}", self.id(rest)),
                    None => self.id(text),
                },
                None => self.id(text),
            },
            F::Enum(_) | F::Parsed(_) => self.enum_value(text),
            _ => self.text(text),
        }
    }

    /// A JSON document held as text, projected by `schema` and written back
    /// in its own layout (Python's default or the compact form).
    fn document(&self, schema: Schema, text: &str) -> String {
        let Ok(parsed) = kassiber_pytext::loads(text) else {
            return self.text(text);
        };
        let options = layout(&parsed, text);
        let projected = match &parsed {
            Py::Dict(_) | Py::List(_) => self.py_object(schema, &parsed),
            _ => return self.text(text),
        };
        kassiber_pyjson::dumps(&projected, &options).unwrap_or_else(|_| self.text(text))
    }

    fn document_list(&self, item: F, text: &str) -> String {
        match kassiber_pytext::loads(text) {
            Ok(parsed @ Py::List(_)) => {
                let options = layout(&parsed, text);
                let Py::List(items) = &parsed else {
                    unreachable!()
                };
                let projected = Py::List(items.iter().map(|v| self.py_value(item, v)).collect());
                kassiber_pyjson::dumps(&projected, &options).unwrap_or_else(|_| "[]".into())
            }
            _ => "[]".into(),
        }
    }

    fn py_object(&self, schema: Schema, value: &Py) -> Py {
        match value {
            Py::Dict(items) => Py::Dict(
                items
                    .iter()
                    .filter_map(|(key, value)| {
                        let Key::Str(name) = key else { return None };
                        let name = name.as_str()?;
                        let (name, class) = schema.iter().find(|(n, _)| *n == name)?;
                        Some((Key::from(*name), self.py_value(*class, value)))
                    })
                    .collect(),
            ),
            Py::List(items) => Py::List(items.iter().map(|v| self.py_object(schema, v)).collect()),
            Py::Null => Py::Null,
            // A scalar where an object belongs: pseudonymized whole, as `object`
            // does for JSON values.
            other => match py_json(other) {
                Some(json) => Py::Str(PyStr::from(self.text(&json.to_string()).as_str())),
                None => Py::Null,
            },
        }
    }

    /// A document value, through the same classes as JSON values: objects
    /// and lists keep their document shape, every scalar is projected by
    /// `value` itself, so JSON held as text and JSON values cannot drift
    /// apart (`json_held_as_text_projects_like_json_values`).
    fn py_value(&self, class: F, value: &Py) -> Py {
        match (class, value) {
            (_, Py::Null) => Py::Null,
            (
                F::Obj(schema) | F::Doc(schema) | F::IntOr(schema) | F::Or(_, schema),
                Py::Dict(_) | Py::List(_),
            ) => self.py_object(schema, value),
            (F::List(item) | F::DocList(item), Py::List(items)) => {
                Py::List(items.iter().map(|v| self.py_value(*item, v)).collect())
            }
            (_, Py::List(items)) => {
                Py::List(items.iter().map(|v| self.py_value(class, v)).collect())
            }
            // A document's exact decimal keeps its spelling, re-parsed.
            (F::Amount, Py::Decimal(d)) => match typed::amount(&d.to_string()) {
                Some(text) => text.parse().map(Py::Decimal).unwrap_or(Py::Null),
                None => Py::Null,
            },
            (_, Py::Str(text)) if text.as_str().is_none() => {
                Py::Str(PyStr::from(self.p.label("surrogate").as_str()))
            }
            (_, other) => match py_json(other) {
                Some(json) => json_py(&self.value(class, &json)),
                None => Py::Null,
            },
        }
    }

    /// The ownership index by shape: maps keyed by script, outpoint or txid,
    /// scoped lists of `{key: [chain, network, id], matches}`, and
    /// `[wallet_id, wallet_label]` pairs. Unknown sections are dropped.
    pub fn owned_index(&self, value: &Value) -> Value {
        let Value::Object(sections) = value else {
            return Value::Null;
        };
        let item = |item: &Value| match item {
            Value::Array(pair) if pair.len() == 2 => Value::Array(vec![
                self.value(F::Row, &pair[0]),
                self.value(F::Text, &pair[1]),
            ]),
            other => self.object(OWNED_MATCH, other),
        };
        let items = |value: &Value| match value {
            Value::Array(values) => Value::Array(values.iter().map(item).collect()),
            _ => Value::Array(Vec::new()),
        };
        let mut out = Map::new();
        for (name, section) in sections {
            let Some(section_name) = OWNED_SECTIONS.iter().find(|s| **s == name) else {
                continue;
            };
            let projected = match section {
                Value::Object(map) => Value::Object(
                    map.iter()
                        .map(|(id, values)| (self.id(id), items(values)))
                        .collect(),
                ),
                Value::Array(entries) => Value::Array(
                    entries
                        .iter()
                        .map(|entry| {
                            let key = match entry.get("key") {
                                Some(Value::Array(key)) if key.len() == 3 => Value::Array(vec![
                                    self.value(F::Enum(CHAINS), &key[0]),
                                    self.value(F::Enum(NETWORKS), &key[1]),
                                    self.value(F::External, &key[2]),
                                ]),
                                _ => Value::Null,
                            };
                            let matches = items(entry.get("matches").unwrap_or(&Value::Null));
                            let mut out = Map::new();
                            out.insert("key".into(), key);
                            out.insert("matches".into(), matches);
                            Value::Object(out)
                        })
                        .collect(),
                ),
                _ => continue,
            };
            out.insert((*section_name).to_owned(), projected);
        }
        Value::Object(out)
    }
}

const OWNED_SECTIONS: &[&str] = &[
    "by_script",
    "by_outpoint",
    "by_address",
    "txid_wallets",
    "by_scoped_outpoint",
    "txid_wallets_by_scope",
];

/// The layout a document was written in: Python's default or compact,
/// ASCII-escaped or not. Only these four, never the book's own bytes.
/// A document value as a JSON value, for `Projection::value`. `None` for
/// what JSON cannot hold (bytes, non-finite floats, non-UTF-8 text).
fn py_json(value: &Py) -> Option<Value> {
    Some(match value {
        Py::Null => Value::Null,
        Py::Bool(b) => Value::Bool(*b),
        Py::Int(n) => Value::Number(n.to_string().parse().ok()?),
        Py::Float(n) => Value::Number(serde_json::Number::from_f64(*n)?),
        Py::Decimal(d) => Value::Number(d.to_string().parse().ok()?),
        Py::Str(text) => Value::String(text.as_str()?.to_owned()),
        Py::List(items) => Value::Array(items.iter().map(py_json).collect::<Option<_>>()?),
        Py::Dict(items) => Value::Object(
            items
                .iter()
                .map(|(key, value)| match key {
                    Key::Str(name) => Some((name.as_str()?.to_owned(), py_json(value)?)),
                    _ => None,
                })
                .collect::<Option<_>>()?,
        ),
        _ => return None,
    })
}

/// A projected JSON value back as a document value. Projected numbers are
/// always 64-bit integers or finite floats.
fn json_py(value: &Value) -> Py {
    match value {
        Value::Null => Py::Null,
        Value::Bool(b) => Py::Bool(*b),
        Value::Number(n) => match (n.as_i64(), n.as_u64(), n.as_f64()) {
            (Some(n), _, _) => Py::Int(n.into()),
            (_, Some(n), _) => Py::Int(n.into()),
            (_, _, Some(n)) => Py::Float(n),
            _ => Py::Null,
        },
        Value::String(text) => Py::Str(PyStr::from(text.as_str())),
        Value::Array(items) => Py::List(items.iter().map(json_py).collect()),
        Value::Object(items) => Py::Dict(
            items
                .iter()
                .map(|(key, value)| (Key::from(key.as_str()), json_py(value)))
                .collect(),
        ),
    }
}

fn layout(parsed: &Py, text: &str) -> kassiber_pyjson::Options {
    use kassiber_pyjson::Options;
    [
        Options::new(),
        Options::new().compact(),
        Options::new().ensure_ascii(false),
        Options::new().compact().ensure_ascii(false),
    ]
    .into_iter()
    .find(|o| kassiber_pyjson::dumps(parsed, o).is_ok_and(|t| t == text))
    .unwrap_or_else(Options::new)
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn project(schema: Schema, value: Value) -> String {
        let p = Pseudonymizer::new([9; 32]);
        Projection::new(&p).object(schema, &value).to_string()
    }

    /// Every engine constant survives an actual projection of a row that
    /// carries it, through documents held as text and nested objects.
    #[test]
    fn engine_constants_survive_a_projection_as_written() {
        let p = Pseudonymizer::new([9; 32]);
        let projection = Projection::new(&p);
        for (name, path, values, _) in ENGINE_CONSTANTS {
            for value in *values {
                // The row: `path` down to the value, documents as JSON text.
                let mut nested = json!(value);
                for depth in (1..path.len()).rev() {
                    let wrapped = json!({ path[depth]: nested });
                    nested = match class_of(name, &path[..depth]) {
                        Some(F::Doc(_)) => json!(wrapped.to_string()),
                        _ => wrapped,
                    };
                }
                let row = json!({ path[0]: nested });
                let mut out = projection.object(section(name).unwrap(), &row);
                for (depth, key) in path.iter().enumerate() {
                    out = out[*key].clone();
                    if depth + 1 < path.len() {
                        if let Value::String(text) = &out {
                            out = serde_json::from_str(text).unwrap();
                        }
                    }
                }
                assert_eq!(out, json!(value), "{name}.{}", path.join("."));
            }
        }
    }

    #[test]
    fn named_fields_are_generated_from_parsed_values_and_everything_else_is_dropped() {
        let raw = json!({"password": "correct-horse", "credentials": "u:p", "txid": "SENTINEL-ALICE-x", "value": 5}).to_string();
        let out = project(
            TRANSACTION,
            json!({
                "raw_json": raw,
                "username": "alice", "in_counterparty": "Alice", "out_description": "Bob",
                "source_url": "node.private.example:50002", "$anything": "secret",
                "account_code": "SENTINEL_ALICE_ACCOUNT", "kind": "alice_kind",
                "amount": "1.5", "fee": "five", "occurred_at": "Tuesday with Alice",
                "confirmed_at": "2001:db8::1", "id": "alice.private.example",
                "external_id": "6f9d6b8a-5b7e-4c1a-9d0e-1f2a3b4c5d6e",
            }),
        );
        for leaked in [
            "correct-horse",
            "credentials",
            "SENTINEL",
            "alice",
            "Alice",
            "Bob",
            "node.private",
            "secret",
            "$anything",
            "five",
            "Tuesday",
            "6f9d6b8a",
            "db8",
        ] {
            assert!(!out.contains(leaked), "{leaked}: {out}");
        }
        assert!(
            out.contains("\"amount\":\"1.5\"") && out.contains("\\\"value\\\":5"),
            "{out}"
        );
    }

    #[test]
    fn only_the_interpreted_markers_survive_in_notes() {
        let kept = project(LEG, json!({"notes": "reviewed_residual:network_fee"}));
        assert!(kept.contains("reviewed_residual:network_fee"), "{kept}");
        for prose in ["private", "rent_for_alice", "reviewed_residual:alice"] {
            let out = project(LEG, json!({"notes": prose}));
            assert!(!out.contains(prose), "{prose}: {out}");
        }
    }

    #[test]
    fn local_ids_and_references_go_through_the_keyed_mapping() {
        let p = Pseudonymizer::new([9; 32]);
        let projection = Projection::new(&p);
        let leg = projection.object(
            LEG,
            &json!({
                "id": "leg-1", "wallet_id": "alice.private.example",
                "location_ref": "network-fee:alice-review",
            }),
        );
        let text = leg.to_string();
        assert!(!text.contains("alice") && !text.contains("leg-1"), "{text}");
        assert_eq!(
            leg["location_ref"],
            format!("network-fee:{}", p.id("alice-review"))
        );
        assert_eq!(leg["wallet_id"], p.id("alice.private.example"));
        assert!(projection.generated().contains(&p.id("leg-1")));
    }

    #[test]
    fn ownership_index_pairs_and_keys_are_projected_by_shape() {
        const TXID: &str = "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b";
        let p = Pseudonymizer::new([9; 32]);
        let index = json!({
            "txid_wallets": {TXID: [["w1", "Alice Savings"]]},
            "txid_wallets_by_scope": [{"key": ["bitcoin", "main", TXID], "matches": [["w1", "Alice Savings"]]}],
            "by_outpoint": {format!("{TXID}:0"): [{"wallet_id": "w1", "wallet_label": "Alice Savings", "source": "wallet_utxos", "secret": "x"}]},
            "unknown_section": {"a": 1},
        });
        let out = Projection::new(&p).owned_index(&index).to_string();
        for leaked in ["Alice", TXID, "secret", "unknown_section", "\"w1\""] {
            assert!(!out.contains(leaked), "{leaked}: {out}");
        }
        assert!(out.contains("wallet_utxos"), "{out}");
    }

    #[test]
    fn coverage_recurses_into_documents_and_flags_an_emptied_nested_schema() {
        let row = json!({"raw_json": json!({"txid": "x", "vin": [{"prevout": {"scriptpubkey": "s"}}]}).to_string()});
        assert!(unclassified(TRANSACTION, &row).is_empty());
        // The same row against a transaction schema whose raw document is
        // emptied reports every nested field.
        const EMPTIED: Schema = &[("raw_json", F::Doc(&[]))];
        assert_eq!(
            unclassified(EMPTIED, &row),
            vec![".raw_json.txid", ".raw_json.vin"]
        );
        const SHALLOW: Schema = &[(
            "raw_json",
            F::Doc(&[("txid", F::External), ("vin", F::Obj(&[]))]),
        )];
        assert_eq!(unclassified(SHALLOW, &row), vec![".raw_json.vin.prevout"]);
    }

    /// Fifth review: `status` in a `raw_json` held as text keeps Esplora's
    /// object and a recognized provider status, value for value.
    #[test]
    fn text_held_raw_json_keeps_status_objects_and_known_statuses() {
        let p = Pseudonymizer::new([5; 32]);
        let projection = Projection::new(&p);
        let parse = |text: &str| -> Value { serde_json::from_str(text).unwrap() };
        let unconfirmed = parse(&projection.document(
            RAW,
            r#"{"status": {"confirmed": false, "block_height": 7}}"#,
        ));
        assert_eq!(
            unconfirmed["status"],
            serde_json::json!({"confirmed": false, "block_height": 7})
        );
        let refunded = parse(&projection.document(RAW, r#"{"status": "transaction.refunded"}"#));
        assert_eq!(refunded["status"], "transaction.refunded");
        let unknown = parse(&projection.document(RAW, r#"{"status": "alice.private.example"}"#));
        assert!(
            unknown["status"].as_str().unwrap().starts_with("label-"),
            "{unknown}"
        );
    }

    /// Every class: the match has no wildcard, so a new `F` variant fails to
    /// compile here until it is listed (and so covered by the parity test).
    fn every_class() -> Vec<F> {
        static ITEM: F = F::External;
        let classes = vec![
            F::Row,
            F::External,
            F::Digest,
            F::Amount,
            F::Time,
            F::Int,
            F::Bool,
            F::Enum(&["a", "transaction.refunded"]),
            F::Parsed(swap::status),
            F::Marker,
            F::Text,
            F::Doc(RAW),
            F::Obj(RAW),
            F::List(&ITEM),
            F::IntOr(RAW),
            F::LocationRef,
            F::DocList(&ITEM),
            F::Or(&SWAP_STATUS, RAW),
        ];
        for class in &classes {
            match class {
                F::Row
                | F::External
                | F::Digest
                | F::Amount
                | F::Time
                | F::Int
                | F::Bool
                | F::Enum(_)
                | F::Parsed(_)
                | F::Marker
                | F::Text
                | F::Doc(_)
                | F::Obj(_)
                | F::List(_)
                | F::IntOr(_)
                | F::LocationRef
                | F::DocList(_)
                | F::Or(_, _) => {}
            }
        }
        classes
    }

    /// Every class projects a document held as text (`raw_json`, evidence,
    /// configuration) exactly as it projects the same JSON value, so no class
    /// is classified but unimplemented on the text path.
    #[test]
    fn json_held_as_text_projects_like_json_values() {
        const TXID: &str = "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b";
        let samples = [
            serde_json::json!(null),
            serde_json::json!(true),
            serde_json::json!(false),
            serde_json::json!(0),
            serde_json::json!(7),
            serde_json::json!(-5),
            serde_json::json!(1.5),
            serde_json::json!("1.5"),
            serde_json::json!("2024-05-01T00:00:00Z"),
            serde_json::json!("transaction.refunded"),
            serde_json::json!("a"),
            serde_json::json!(TXID),
            serde_json::json!(format!("{TXID}:1")),
            serde_json::json!("reviewed_residual:network_fee"),
            serde_json::json!("network-fee:x"),
            serde_json::json!("Alice"),
            serde_json::json!({"confirmed": false, "block_height": 7}),
            serde_json::json!([{"confirmed": true}, {"txid": TXID}]),
            serde_json::json!([1, "a", null]),
            serde_json::json!({"$decimal": "1.50"}),
            serde_json::json!("{\"confirmed\": false}"),
            serde_json::json!("[\"ab\", \"transaction.refunded\"]"),
        ];
        let p = Pseudonymizer::new([9; 32]);
        let projection = Projection::new(&p);
        for (index, class) in every_class().into_iter().enumerate() {
            let schema: Schema = Box::leak(Box::new([("f", class)]));
            for sample in &samples {
                let document = serde_json::json!({ "f": sample });
                let as_value = projection.object(schema, &document);
                let as_text: Value =
                    serde_json::from_str(&projection.document(schema, &document.to_string()))
                        .unwrap();
                assert_eq!(as_text, as_value, "class #{index}, sample {sample}");
            }
        }
    }
}
