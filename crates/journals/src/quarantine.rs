//! `ui.journals.quarantine`: the explained quarantine page, ported from
//! `kassiber.core.quarantine_review` and `quarantine_catalog`.
//!
//! It reads the stored derived journal state and the book's wallets and pair
//! reviews. It performs no network I/O and never changes accounting state:
//! every listed action points at a separately confirmed workflow.
use std::collections::{BTreeMap, BTreeSet, HashMap};

use kassiber_envelope::{dict, get, py_str, s, AppError};
use kassiber_pydecimal::{Context, Decimal, Rounding};
use kassiber_pyjson::{BigInt, Key, Value as Py};
use kassiber_store::rusqlite::{types::Value as Sql, Connection};

use crate::views::{
    active, args_object, btc, int_or_zero, int_py, label, limit, py, py_truthy, query, truthy,
    unknown, Row,
};

pub mod groups;

const KIND: &str = "ui.journals.quarantine";
const MAX_UI_PREVIEW_LIMIT: i64 = 100;
const MAX_GROUPS: usize = 50;
const MAX_ASSUMPTION_ITEMS: usize = 50;
const MAX_GROUP_ROOTS: usize = 25;
const SCOPES: [&str; 3] = ["all", "attention", "waiting"];

/// `QUARANTINE_SUMMARY_DEFINITIONS`: one sentence per summary counter.
const SUMMARY_DEFINITIONS: [(&str, &str); 7] = [
    ("count", "Every quarantined row in the active book, one per transaction."),
    ("by_reason", "Rows per stored primary reason."),
    ("attention_count", "Root causes plus downstream rows whose root cannot be named; the review badge's quarantine_attention counts the same set."),
    ("waiting_count", "Downstream rows that wait on a named root and clear when it is resolved."),
    ("blocking_count", "Rows named by a report-blocking custody quantity issue."),
    ("scope_count", "Rows the chosen scope lists; items pages through them."),
    ("group_count", "Cause groups in summary.groups; ui.journals.quarantine.groups counts per root transaction instead."),
];

const DOWNSTREAM: &str = "downstream";
const CATEGORIES: [&str; 7] = [
    "missing_wallet_history",
    "missing_chain_evidence",
    "missing_price",
    "missing_acquisition_history",
    "needs_decision",
    "unsupported",
    DOWNSTREAM,
];
const GENERIC_OUTBOUND_KINDS: [&str; 8] = [
    "",
    "withdrawal",
    "withdraw",
    "send",
    "sent",
    "transfer",
    "transfer_out",
    "unknown",
];
/// `transaction_kinds.INBOUND_KIND_TO_RP2_TYPE` keys.
const INBOUND_KINDS: [&str; 12] = [
    "buy",
    "airdrop",
    "hardfork",
    "hard_fork",
    "income",
    "interest",
    "lending_interest",
    "mining",
    "mining_reward",
    "routing_income",
    "staking",
    "wages",
];

#[derive(Clone, Copy)]
struct Info {
    category: &'static str,
    actions: &'static [&'static str],
}
impl Info {
    fn downstream(self) -> bool {
        self.category == DOWNSTREAM
    }
}
const fn info(category: &'static str, actions: &'static [&'static str]) -> Info {
    Info { category, actions }
}

/// `quarantine_catalog._REASONS`.
fn reason(reason: &str) -> Info {
    match reason {
        "ownership_transfer_source_missing" => {
            info("missing_wallet_history", &["sync_wallet", "import_history"])
        }
        "ownership_transfer_destination_missing_ref" => {
            info("missing_wallet_history", &["connect_wallet"])
        }
        "channel_open_unresolved" | "channel_close_unresolved" => info(
            "missing_wallet_history",
            &["import_history", "connect_wallet"],
        ),
        "ownership_transfer_amount_mismatch"
        | "ownership_transfer_conservation_mismatch"
        | "ownership_transfer_fee_evidence_incomplete"
        | "ownership_transfer_asset_evidence_incomplete"
        | "recorded_transfer_authority_conflict"
        | "samourai_native_event_unverified" => info("missing_chain_evidence", &["sync_wallet"]),
        "liquid_transfer_graph_incomplete" => {
            info("missing_chain_evidence", &["sync_wallet", "connect_wallet"])
        }
        "pending_onchain_confirmation" => {
            info("missing_chain_evidence", &["wait_for_confirmation"])
        }
        "conflicting_spend" => info(
            "missing_chain_evidence",
            &["wait_for_confirmation", "sync_wallet"],
        ),
        "missing_spot_price" | "pricing_review_required" | "at_swap_price_required" => {
            info("missing_price", &["set_price"])
        }
        "insufficient_lots" => info(
            "missing_acquisition_history",
            &["import_history", "connect_wallet"],
        ),
        "missing_cost_basis" => info(
            "missing_acquisition_history",
            &["import_history", "set_price"],
        ),
        "ownership_transfer_source_ambiguous"
        | "ownership_transfer_destination_ambiguous"
        | "unscoped_transfer_review"
        | "manual_multi_pair_ambiguous"
        | "transfer_mismatch"
        | "transfer_pair_chronology_mismatch"
        | "transfer_network_mismatch"
        | "native_transition_ambiguous" => info("needs_decision", &["pair_transfer"]),
        "owned_fanout_unresolved" | "transfer_fee_implausible" => {
            info("needs_decision", &["pair_transfer", "connect_wallet"])
        }
        "privacy_hop_unresolved" => info("needs_decision", &["connect_wallet", "attach_evidence"]),
        "native_transition_amount_mismatch" => info("needs_decision", &["attach_evidence"]),
        "unclassified_income_kind" | "non_sale_disposal_kind" => {
            info("needs_decision", &["classify"])
        }
        "native_transition_fee_timing_unresolved" => info("unsupported", &[]),
        "acquisition_valuation_unsupported" => info("unsupported", &["classify"]),
        "custody_basis_barrier"
        | "transfer_pair_dependency_blocked"
        | "basis_provenance_incomplete"
        | "derived_transfer_group_blocked"
        | "at_swap_basis_carry_unresolved"
        | "bitcoin_rail_carry_basis_unresolved" => info(DOWNSTREAM, &["resolve_root"]),
        // ownership_transfer_ambiguous_output, ..._duplicate_outbound, the
        // custody blockers and every unknown reason.
        _ => info("needs_decision", &[]),
    }
}

/// `quarantine_catalog.reason_info`, resolving the quantity umbrella.
fn reason_info(name: &str, detail: &Py) -> Info {
    if name != "custody_quantity_unresolved" {
        return reason(name);
    }
    match text_or_empty(get(detail, "blocker_code")).as_str() {
        "custody_gap_review_required" => info(
            "missing_wallet_history",
            &["connect_wallet", "review_custody_gap"],
        ),
        "implicit_wallet_delta_unallocated" => {
            info("missing_wallet_history", &["sync_wallet", "import_history"])
        }
        "search_capacity_incomplete" | "capacity_source_suspense_required" => {
            info("needs_decision", &["review_custody_gap"])
        }
        "reviewed_residual_suspense" => {
            info("needs_decision", &["review_pair", "review_custody_gap"])
        }
        "unclaimed_source_residual" => {
            info("needs_decision", &["connect_wallet", "attach_evidence"])
        }
        _ => info("needs_decision", &[]),
    }
}

/// `str(value or "")`.
fn text_or_empty(value: Option<&Py>) -> String {
    match value {
        Some(value) if py_truthy(value) => py_str(value),
        _ => String::new(),
    }
}

fn field<'a>(value: &'a Py, key: &str) -> Option<&'a Py> {
    get(value, key)
}

/// `_parse_detail`: a JSON object, otherwise `{}`.
fn parse_detail(value: &Sql) -> Py {
    let text = match value {
        value if !truthy(value) => "{}".to_owned(),
        Sql::Text(text) => text.clone(),
        Sql::Blob(bytes) => match String::from_utf8(bytes.clone()) {
            Ok(text) => text,
            Err(_) => return dict::<&str>([]),
        },
        _ => return dict::<&str>([]),
    };
    match kassiber_pytext::loads(&text) {
        Ok(value @ Py::Dict(_)) => value,
        _ => dict::<&str>([]),
    }
}

/// `_asset_msat`: decimal asset units to msat, or `None`.
fn asset_msat(value: Option<&Py>) -> Option<Py> {
    let value = value?;
    if matches!(value, Py::Null | Py::Bool(_)) {
        return None;
    }
    let text = py_str(value);
    let amount: Decimal = kassiber_pytext::strip(&text).parse().ok()?;
    let scaled = amount
        .mul(&Decimal::from(100_000_000_000_i64), &Context::RP2)
        .ok()?;
    let integral = scaled
        .quantize(
            0,
            &Context::new(kassiber_pydecimal::MAX_PRECISION, Rounding::HalfEven),
        )
        .ok()?;
    Some(Py::Int(integral.to_string().parse().ok()?))
}

fn str_list(value: Option<&Py>) -> Option<&Vec<Py>> {
    match value {
        Some(Py::List(items)) => Some(items),
        _ => None,
    }
}

struct Wallet {
    id: String,
    label: String,
    deprecated: bool,
}

fn wallets(conn: &Connection, profile_id: &Sql) -> Result<HashMap<String, Wallet>, AppError> {
    let mut wallets = HashMap::new();
    for row in query(
        conn,
        "SELECT id, label, config_json FROM wallets WHERE profile_id = ?",
        std::slice::from_ref(profile_id),
    )? {
        let config = parse_detail(row.get("config_json"));
        let id = py_str(&py(row.get("id")));
        wallets.insert(
            id.clone(),
            Wallet {
                id,
                label: py_str(&py(row.get("label"))),
                deprecated: field(&config, "deprecated").is_some_and(py_truthy),
            },
        );
    }
    Ok(wallets)
}

fn wallet_payload(wallet: &Wallet) -> Py {
    dict([
        ("id", s(wallet.id.as_str())),
        ("label", s(wallet.label.as_str())),
    ])
}

fn blocking_ids(conn: &Connection, profile_id: &Sql) -> Result<BTreeSet<String>, AppError> {
    let mut blocking = BTreeSet::new();
    for row in query(
        conn,
        "SELECT transaction_ids_json FROM journal_quantity_issues WHERE profile_id = ?",
        std::slice::from_ref(profile_id),
    )? {
        let raw = row.get("transaction_ids_json");
        let text = match raw {
            value if !truthy(value) => "[]".to_owned(),
            Sql::Text(text) => text.clone(),
            _ => continue,
        };
        if let Ok(Py::List(items)) = kassiber_pytext::loads(&text) {
            blocking.extend(items.iter().filter(|item| py_truthy(item)).map(py_str));
        }
    }
    Ok(blocking)
}

/// `pair_fingerprint`: a digest of what a pair review says and allocates.
fn pair_fingerprint(conn: &Connection, pair: &Py) -> Result<String, AppError> {
    let component = field(pair, "component_id").cloned().unwrap_or(Py::Null);
    let mut allocations = Vec::new();
    if py_truthy(&component) {
        for row in query(
            conn,
            "SELECT source.role AS source_role,
                   COALESCE(source.anchor_transaction_id, source.transaction_id) AS source_tx,
                   sink.role AS sink_role,
                   COALESCE(sink.anchor_transaction_id, sink.transaction_id) AS sink_tx,
                   allocation.source_amount_msat, allocation.sink_amount_msat
            FROM custody_component_allocations allocation
            JOIN custody_component_legs source ON source.id = allocation.source_leg_id
            JOIN custody_component_legs sink ON sink.id = allocation.sink_leg_id
            WHERE allocation.component_id = ?",
            &[Sql::Text(py_str(&component))],
        )? {
            allocations.push(Py::List(vec![
                py(row.get("source_role")),
                py(row.get("source_tx")),
                py(row.get("sink_role")),
                py(row.get("sink_tx")),
                int_py(int_or_zero(row.get("source_amount_msat"))?),
                int_py(int_or_zero(row.get("sink_amount_msat"))?),
            ]));
        }
    }
    let sort_key = |item: &Py| {
        kassiber_pyjson::dumps(item, &kassiber_pyjson::Options::new().default_str(true))
            .unwrap_or_default()
    };
    allocations.sort_by_key(sort_key);
    let mut reading = vec![(
        Key::from("pair_id"),
        s(text_or_empty(field(pair, "id")).as_str()),
    )];
    for key in [
        "kind",
        "policy",
        "out_amount",
        "swap_fee_msat",
        "swap_fee_kind",
    ] {
        reading.push((
            Key::from(key),
            field(pair, key).cloned().unwrap_or(Py::Null),
        ));
    }
    for side in ["out", "in"] {
        for name in [
            "transaction_id",
            "asset",
            "full_amount_msat",
            "amount_msat",
            "external_id",
            "occurred_at",
        ] {
            let key = format!("{side}_{name}");
            reading.push((
                Key::from(key.as_str()),
                field(pair, &key).cloned().unwrap_or(Py::Null),
            ));
        }
    }
    reading.push((Key::from("allocations"), Py::List(allocations)));
    let encoded = kassiber_pyjson::dumps(
        &Py::Dict(reading),
        &kassiber_pyjson::Options::new()
            .sort_keys(true)
            .separators(",", ":")
            .default_str(true),
    )
    .map_err(|error| AppError::new("internal_error", error.to_string()))?;
    use bitcoin_hashes::{sha256, Hash};
    Ok(sha256::Hash::hash(encoded.as_bytes()).to_string())
}

fn canonical_txid(value: Option<&Py>) -> Option<String> {
    let text = kassiber_pytext::lower(kassiber_pytext::strip(&text_or_empty(value)));
    (text.chars().count() == 64
        && text
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()))
    .then_some(text)
}

/// `pair_evidence`: what a suspense-holding pair looks like.
fn pair_evidence(
    conn: &Connection,
    transaction_id: &str,
    pair: &Py,
) -> Result<Vec<(Key, Py)>, AppError> {
    let out_id = text_or_empty(field(pair, "out_transaction_id"));
    let in_id = text_or_empty(field(pair, "in_transaction_id"));
    let int_of = |key: &str| -> Result<Py, AppError> {
        match field(pair, key) {
            Some(value) if py_truthy(value) => match value {
                Py::Int(_) => Ok(value.clone()),
                Py::Bool(b) => Ok(Py::Int(i64::from(*b).into())),
                other => Err(AppError::new(
                    "internal_error",
                    format!(
                        "invalid literal for int() with base 10: {}",
                        kassiber_envelope::py_repr(other)
                    ),
                )),
            },
            _ => Ok(Py::Int(0.into())),
        }
    };
    let legs = ["out", "in"]
        .iter()
        .map(|side| {
            Ok((
                Key::from(*side),
                dict([
                    (
                        "transaction_id",
                        s(text_or_empty(field(pair, &format!("{side}_transaction_id"))).as_str()),
                    ),
                    (
                        "wallet",
                        s(text_or_empty(field(pair, &format!("{side}_wallet"))).as_str()),
                    ),
                    (
                        "asset",
                        s(text_or_empty(field(pair, &format!("{side}_asset"))).as_str()),
                    ),
                    ("amount_msat", int_of(&format!("{side}_full_amount_msat"))?),
                    (
                        "occurred_at",
                        field(pair, &format!("{side}_occurred_at"))
                            .cloned()
                            .unwrap_or(Py::Null),
                    ),
                    (
                        "external_id",
                        s(text_or_empty(field(pair, &format!("{side}_external_id"))).as_str()),
                    ),
                ]),
            ))
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    let mut evidence = vec![
        (
            Key::from("pair_id"),
            s(py_str(field(pair, "id").unwrap_or(&Py::Null)).as_str()),
        ),
        (
            Key::from("pair_fingerprint"),
            s(pair_fingerprint(conn, pair)?.as_str()),
        ),
        (
            Key::from("pair_counterpart_transaction_id"),
            s(if transaction_id == out_id {
                in_id.as_str()
            } else {
                out_id.as_str()
            }),
        ),
        (Key::from("pair_legs"), Py::Dict(legs)),
        (
            Key::from("pair_review"),
            dict([
                ("kind", field(pair, "kind").cloned().unwrap_or(Py::Null)),
                ("policy", field(pair, "policy").cloned().unwrap_or(Py::Null)),
                ("out_amount_msat", int_of("out_amount_msat")?),
                ("in_amount_msat", int_of("in_amount_msat")?),
            ]),
        ),
    ];
    let out_txid = canonical_txid(field(pair, "out_external_id"));
    let in_txid = canonical_txid(field(pair, "in_external_id"));
    let same_chain = kassiber_pytext::upper(&text_or_empty(field(pair, "out_asset")))
        == kassiber_pytext::upper(&text_or_empty(field(pair, "in_asset")));
    if let (Some(out_txid), Some(in_txid)) = (&out_txid, &in_txid) {
        if same_chain {
            evidence.push((
                Key::from("pair_txids_differ"),
                Py::Bool(out_txid != in_txid),
            ));
        }
    }
    let out_at = text_or_empty(field(pair, "out_occurred_at"));
    let in_at = text_or_empty(field(pair, "in_occurred_at"));
    if !out_at.is_empty() && !in_at.is_empty() {
        evidence.push((
            Key::from("pair_receipt_before_spend"),
            Py::Bool(in_at < out_at),
        ));
    }
    Ok(evidence)
}

fn iso_utc(value: &str) -> Option<i64> {
    let raw = kassiber_pytext::strip(value);
    if raw.is_empty() {
        return None;
    }
    kassiber_pytext::iso_datetime(raw).map(|parsed| parsed.utc_us())
}

/// `quarantine_review._journal_freshness`.
fn journal_freshness(conn: &Connection, profile_id: &str) -> Result<Py, AppError> {
    let id = Sql::Text(profile_id.to_owned());
    let marker = query(
        conn,
        "SELECT p.*,
                   (SELECT COUNT(*) FROM transactions t
                    WHERE t.profile_id = p.id AND t.excluded = 0)
                       AS active_transaction_count
            FROM profiles p
            WHERE p.id = ?",
        std::slice::from_ref(&id),
    )?
    .into_iter()
    .next();
    // `custody_journal.projection_freshness`, calculation semantics included.
    let (status, needs, processed_at) = match &marker {
        None => ("no_profile", false, Py::Null),
        Some(row) => {
            let active = int_or_zero(row.get("active_transaction_count"))?;
            let processed = truthy(row.get("last_processed_at"));
            let count = int_or_zero(row.get("last_processed_tx_count"))?;
            let input = int_or_zero(row.get("journal_input_version"))?;
            let done = int_or_zero(row.get("last_processed_input_version"))?;
            // Canonical freshness: a profile row without both semantics
            // columns is never current.
            let semantics = if row.has("required_journal_semantics_version")
                && row.has("journal_semantics_version")
            {
                let required = int_or_zero(row.get("required_journal_semantics_version"))?;
                let built = int_or_zero(row.get("journal_semantics_version"))?;
                let version =
                    BigInt::from(kassiber_store::semantics::JOURNAL_CALCULATION_SEMANTICS_VERSION);
                required == BigInt::from(0) || (built == version && built >= required)
            } else {
                false
            };
            let status = if active == BigInt::from(0) {
                "no_transactions"
            } else if !processed {
                "not_processed"
            } else if count != active || input != done || !semantics {
                "stale"
            } else {
                "current"
            };
            (
                status,
                matches!(status, "not_processed" | "stale"),
                py(row.get("last_processed_at")),
            )
        }
    };
    let source = query(
        conn,
        "SELECT * FROM freshness_source_states WHERE profile_id = ? AND source_key = ?",
        &[
            Sql::Text(profile_id.to_owned()),
            Sql::Text(format!("journals:{profile_id}")),
        ],
    )?
    .into_iter()
    .next();
    let mut last_error = Py::Null;
    if let Some(source) = source.filter(|row| truthy(row.get("last_error_at"))) {
        let error_at = py_str(&py(source.get("last_error_at")));
        let success_at = text_or_empty(Some(&py(source.get("last_success_at"))));
        let processed = text_or_empty(Some(&processed_at));
        let error_time = iso_utc(&error_at);
        let success_time = iso_utc(&success_at);
        let processed_time = iso_utc(&processed);
        if let Some(error_time) = error_time {
            if success_time.is_none_or(|t| error_time > t)
                && processed_time.is_none_or(|t| error_time > t)
            {
                let code = text_or_empty(Some(&py(source.get("last_error_code"))));
                let message = text_or_empty(Some(&py(source.get("last_error_message"))));
                last_error = dict([
                    (
                        "code",
                        s(if code.is_empty() {
                            "journal_refresh_failed".to_owned()
                        } else {
                            code
                        }),
                    ),
                    (
                        "message",
                        s(kassiber_envelope::redaction::redact_operational_text(
                            if message.is_empty() {
                                "Journal processing failed."
                            } else {
                                &message
                            },
                        )),
                    ),
                    ("at", s(error_at)),
                ]);
            }
        }
    }
    Ok(dict([
        ("needs_processing", Py::Bool(needs)),
        ("status", s(status)),
        ("last_processed_at", processed_at),
        ("last_error", last_error),
    ]))
}

fn evidence(
    detail: &Py,
    row: &Row,
    wallets: &HashMap<String, Wallet>,
) -> Result<Vec<(Key, Py)>, AppError> {
    let mut evidence = Vec::new();
    let mut put = |key: &str, value: Py| evidence.push((Key::from(key), value));
    let wallet_label = [field(detail, "wallet"), field(detail, "from_wallet")]
        .into_iter()
        .flatten()
        .find(|value| py_truthy(value))
        .cloned()
        .unwrap_or_else(|| py(row.get("wallet")));
    if py_truthy(&wallet_label) {
        put("wallet_label", s(py_str(&wallet_label).as_str()));
    }
    if let Some(ids) =
        str_list(field(detail, "missing_source_wallet_ids")).filter(|ids| !ids.is_empty())
    {
        put(
            "missing_source_wallets",
            Py::List(
                ids.iter()
                    .map(|id| {
                        let key = py_str(id);
                        let wallet = wallets.get(&key);
                        let label = wallet
                            .map(|w| w.label.clone())
                            .filter(|l| !l.is_empty())
                            .unwrap_or(key.clone());
                        dict([
                            ("id", s(key.as_str())),
                            ("label", s(label.as_str())),
                            ("deprecated", Py::Bool(wallet.is_some_and(|w| w.deprecated))),
                        ])
                    })
                    .collect(),
            ),
        );
    }
    if let Some(code) = field(detail, "blocker_code").filter(|v| py_truthy(v)) {
        put("blocker_code", s(py_str(code).as_str()));
        let codes = match field(detail, "blocker_codes").filter(|v| py_truthy(v)) {
            None => vec![s(py_str(code).as_str())],
            Some(Py::List(items)) => items.iter().map(|item| s(py_str(item).as_str())).collect(),
            Some(Py::Str(text)) => text
                .code_points()
                .filter_map(char::from_u32)
                .map(|c| s(c.to_string().as_str()))
                .collect(),
            Some(Py::Dict(items)) => items
                .iter()
                .map(|(key, _)| match key {
                    Key::Str(text) => Py::Str(text.clone()),
                    other => s(format!("{other:?}").as_str()),
                })
                .collect(),
            Some(other) => {
                return Err(AppError::new(
                    "internal_error",
                    format!(
                        "'{}' object is not iterable",
                        kassiber_envelope::type_name(other)
                    ),
                ))
            }
        };
        put("blocker_codes", Py::List(codes));
    }
    if let Some(first) = str_list(field(detail, "gap_ids")).and_then(|ids| ids.first()) {
        put("gap_id", s(py_str(first).as_str()));
    }
    let asset = [field(detail, "asset")]
        .into_iter()
        .flatten()
        .find(|value| py_truthy(value))
        .cloned()
        .unwrap_or_else(|| py(row.get("asset")));
    if matches!(
        kassiber_pytext::upper(&text_or_empty(Some(&asset))).as_str(),
        "BTC" | "LBTC"
    ) {
        if let Some(required) = asset_msat(field(detail, "required")) {
            put("required_msat", required);
        }
        let available = match field(detail, "available") {
            Some(value) => Some(value),
            None => field(detail, "priced_available"),
        };
        if let Some(available) = asset_msat(available) {
            put("available_msat", available);
        }
    }
    if let Some(since) = field(detail, "lot_state_uncertain_since").filter(|v| py_truthy(v)) {
        put("lot_state_uncertain_since", s(py_str(since).as_str()));
    }
    let own = py_str(&py(row.get("transaction_id")));
    let mut counterparts = BTreeSet::new();
    for key in ["out_transaction_id", "in_transaction_id"] {
        if let Some(value) = field(detail, key).filter(|v| py_truthy(v)) {
            if py_str(value) != own {
                counterparts.insert(py_str(value));
            }
        }
    }
    for key in ["outbound_ids", "blocked_by_transaction_ids"] {
        if let Some(items) = str_list(field(detail, key)) {
            counterparts.extend(items.iter().map(py_str).filter(|item| *item != own));
        }
    }
    if !counterparts.is_empty() {
        put(
            "counterpart_transaction_ids",
            Py::List(counterparts.into_iter().map(|c| s(c.as_str())).collect()),
        );
    }
    if let Some(reason) = field(detail, "blocked_by_reason").filter(|v| py_truthy(v)) {
        put("blocked_by_reason", s(py_str(reason).as_str()));
    }
    for key in [
        "row_amount_msat",
        "owned_receipts_msat",
        "owned_outputs_msat",
    ] {
        match field(detail, key) {
            Some(value @ Py::Int(_)) => put(key, value.clone()),
            Some(Py::Bool(b)) => put(key, Py::Int(i64::from(*b).into())),
            _ => {}
        }
    }
    Ok(evidence)
}

fn ev<'a>(evidence: &'a [(Key, Py)], key: &str) -> Option<&'a Py> {
    evidence
        .iter()
        .find(|(k, _)| *k == Key::from(key))
        .map(|(_, v)| v)
}

fn actions(
    info: Info,
    transaction_id: &str,
    wallet_id: &Py,
    evidence: &[(Key, Py)],
    root: &Py,
    wallets: &HashMap<String, Wallet>,
) -> Py {
    let mut actions = Vec::new();
    let own = wallets.get(&text_or_empty(Some(wallet_id)));
    let missing = ev(evidence, "missing_source_wallets").and_then(|v| match v {
        Py::List(items) if !items.is_empty() => Some(items),
        _ => None,
    });
    for kind in info.actions {
        match *kind {
            "sync_wallet" => {
                let targets: Vec<Py> = match missing {
                    Some(items) => items.clone(),
                    None => own.map(wallet_payload).into_iter().collect(),
                };
                for wallet in targets {
                    actions.push(dict([
                        ("kind", s(*kind)),
                        ("wallet_id", get(&wallet, "id").cloned().unwrap_or(Py::Null)),
                        (
                            "wallet_label",
                            get(&wallet, "label").cloned().unwrap_or(Py::Null),
                        ),
                    ]));
                }
            }
            "import_history" => {
                let target = match missing {
                    Some(items) => items.first().cloned(),
                    None => own.map(wallet_payload),
                };
                match target {
                    Some(wallet) => actions.push(dict([
                        ("kind", s(*kind)),
                        ("wallet_id", get(&wallet, "id").cloned().unwrap_or(Py::Null)),
                        (
                            "wallet_label",
                            get(&wallet, "label").cloned().unwrap_or(Py::Null),
                        ),
                    ])),
                    None => actions.push(dict([("kind", s(*kind))])),
                }
            }
            "review_custody_gap" => {
                if let Some(gap) = ev(evidence, "gap_id").filter(|v| py_truthy(v)) {
                    actions.push(dict([("kind", s(*kind)), ("gap_id", gap.clone())]));
                }
            }
            "resolve_root" => {
                if !matches!(root, Py::Null) {
                    actions.push(dict([
                        ("kind", s(*kind)),
                        (
                            "transaction_id",
                            get(root, "transaction_id").cloned().unwrap_or(Py::Null),
                        ),
                    ]));
                }
            }
            "review_pair" => {
                if let Some(pair) = ev(evidence, "pair_id").filter(|v| py_truthy(v)) {
                    actions.push(dict([
                        ("kind", s(*kind)),
                        ("transaction_id", s(transaction_id)),
                        ("pair_id", pair.clone()),
                    ]));
                }
            }
            "connect_wallet" | "wait_for_confirmation" => actions.push(dict([("kind", s(*kind))])),
            _ => actions.push(dict([
                ("kind", s(*kind)),
                ("transaction_id", s(transaction_id)),
            ])),
        }
    }
    Py::List(actions)
}

type Lookups<'a> = (
    &'a HashMap<&'a str, &'a Held>,
    &'a HashMap<String, Py>,
    &'a Vec<(Py, Vec<String>)>,
    &'a Vec<((Py, Py), Vec<String>)>,
);

struct Held {
    reason: String,
    occurred_at: Py,
    asset: Py,
}

/// `quarantine_review.waiting_roots`.
fn waiting_roots(
    rows: &[(String, Held)],
    details: &HashMap<String, Py>,
) -> HashMap<String, String> {
    let by_id: HashMap<&str, &Held> = rows.iter().map(|(id, held)| (id.as_str(), held)).collect();
    let is_root = |id: &str| {
        by_id
            .get(id)
            .is_some_and(|held| !reason(&held.reason).downstream())
    };
    let mut by_group: Vec<(Py, Vec<String>)> = Vec::new();
    for (id, _) in rows {
        if let Some(group) = details.get(id).and_then(|d| field(d, "transfer_group_id")) {
            if py_truthy(group) && matches!(group, Py::Str(_) | Py::Int(_) | Py::Bool(_)) {
                match by_group.iter_mut().find(|(g, _)| g == group) {
                    Some((_, ids)) => ids.push(id.clone()),
                    None => by_group.push((group.clone(), vec![id.clone()])),
                }
            }
        }
    }
    let mut roots_at: Vec<((Py, Py), Vec<String>)> = Vec::new();
    for (id, held) in rows {
        if is_root(id) {
            let key = (held.occurred_at.clone(), held.asset.clone());
            match roots_at.iter_mut().find(|(k, _)| *k == key) {
                Some((_, ids)) => ids.push(id.clone()),
                None => roots_at.push((key, vec![id.clone()])),
            }
        }
    }
    fn resolve(
        id: &str,
        reason_name: &str,
        detail: &Py,
        visited: &BTreeSet<String>,
        ctx: &Lookups<'_>,
    ) -> Option<String> {
        let (by_id, details, by_group, roots_at) = *ctx;
        let mut visited = visited.clone();
        visited.insert(id.to_owned());
        let list = |key: &str| -> Vec<String> {
            match field(detail, key) {
                Some(Py::List(items)) => items.iter().map(py_str).collect(),
                Some(value) if py_truthy(value) => match value {
                    Py::Str(text) => text
                        .code_points()
                        .filter_map(char::from_u32)
                        .map(String::from)
                        .collect(),
                    _ => vec![],
                },
                _ => vec![],
            }
        };
        let candidates: Vec<String> = match reason_name {
            "custody_basis_barrier" => list("root_transaction_ids"),
            "transfer_pair_dependency_blocked" => list("blocked_by_transaction_ids"),
            "derived_transfer_group_blocked" => match field(detail, "transfer_group_id") {
                Some(group)
                    if py_truthy(group)
                        && matches!(group, Py::Str(_) | Py::Int(_) | Py::Bool(_)) =>
                {
                    by_group
                        .iter()
                        .find(|(g, _)| g == group)
                        .map(|(_, ids)| ids.clone())
                        .unwrap_or_default()
                }
                _ => vec![],
            },
            "basis_provenance_incomplete" => match field(detail, "lot_state_uncertain_since") {
                Some(since @ Py::Str(_)) if py_truthy(since) => {
                    let asset = by_id.get(id).map(|h| h.asset.clone()).unwrap_or(Py::Null);
                    let matches = roots_at
                        .iter()
                        .find(|((at, a), _)| at == since && *a == asset)
                        .map(|(_, ids)| ids.clone())
                        .unwrap_or_default();
                    if matches.len() == 1 {
                        matches
                    } else {
                        vec![]
                    }
                }
                _ => vec![],
            },
            _ => vec![],
        };
        let is_root = |candidate: &str| {
            by_id
                .get(candidate)
                .is_some_and(|held| !reason(&held.reason).downstream())
        };
        let mut roots: Vec<String> = candidates
            .iter()
            .filter(|c| !visited.contains(*c) && is_root(c))
            .cloned()
            .collect();
        if roots.is_empty() {
            for candidate in &candidates {
                if visited.contains(candidate) || !by_id.contains_key(candidate.as_str()) {
                    continue;
                }
                let held = by_id[candidate.as_str()];
                let empty = dict::<&str>([]);
                let nested = resolve(
                    candidate,
                    &held.reason,
                    details.get(candidate).unwrap_or(&empty),
                    &visited,
                    ctx,
                );
                if let Some(nested) = nested.filter(|n| !visited.contains(n)) {
                    roots.push(nested);
                }
            }
        }
        roots.into_iter().min_by(|a, b| {
            let at = |id: &str| text_or_empty(by_id.get(id).map(|h| &h.occurred_at));
            (at(a), a.clone()).cmp(&(at(b), b.clone()))
        })
    }
    let ctx = (&by_id, details, &by_group, &roots_at);
    let mut waiting = HashMap::new();
    let empty = dict::<&str>([]);
    for (id, held) in rows {
        let detail = details.get(id).unwrap_or(&empty);
        if !reason_info(&held.reason, detail).downstream() {
            continue;
        }
        if let Some(root) = resolve(id, &held.reason, detail, &BTreeSet::new(), &ctx) {
            waiting.insert(id.clone(), root);
        }
    }
    waiting
}

fn normalized_kind(row: &Row) -> String {
    let kind = match row.get("kind_override") {
        Sql::Null => row.get("kind"),
        Sql::Text(text) if text.is_empty() => row.get("kind"),
        other => other,
    };
    let text = text_or_empty(Some(&py(kind)));
    kassiber_pytext::lower(kassiber_pytext::strip(&text)).replace(['-', ' '], "_")
}

fn assumptions(
    conn: &Connection,
    profile_id: &Sql,
    held: &BTreeSet<String>,
) -> Result<Py, AppError> {
    let collect = |state: &str, direction: &str, outbound: bool| -> Result<Py, AppError> {
        let rows = query(
            conn,
            "SELECT p.transaction_id, SUM(ABS(p.amount_msat)) AS amount_msat,
                       t.occurred_at, t.external_id, t.kind, t.kind_override,
                       w.label AS wallet
                FROM journal_quantity_postings p
                JOIN transactions t ON t.id = p.transaction_id
                JOIN wallets w ON w.id = t.wallet_id
                WHERE p.profile_id = ? AND p.state = ? AND t.direction = ?
                  AND t.excluded = 0 AND upper(t.asset) IN ('BTC', 'LBTC')
                GROUP BY p.transaction_id
                ORDER BY t.occurred_at ASC, p.transaction_id ASC",
            &[
                profile_id.clone(),
                Sql::Text(state.into()),
                Sql::Text(direction.into()),
            ],
        )?;
        let items: Vec<&Row> = rows
            .iter()
            .filter(|row| {
                let kind = normalized_kind(row);
                !held.contains(&py_str(&py(row.get("transaction_id"))))
                    && if outbound {
                        GENERIC_OUTBOUND_KINDS.contains(&kind.as_str())
                    } else {
                        !INBOUND_KINDS.contains(&kind.as_str())
                    }
            })
            .collect();
        let mut total = BigInt::from(0);
        for row in &items {
            total += int_or_zero(row.get("amount_msat"))?;
        }
        Ok(dict([
            ("count", Py::Int(items.len().into())),
            ("amount_msat", Py::Int(total)),
            (
                "items",
                Py::List(
                    items
                        .iter()
                        .take(MAX_ASSUMPTION_ITEMS)
                        .map(|row| {
                            Ok(dict([
                                (
                                    "transaction_id",
                                    s(py_str(&py(row.get("transaction_id"))).as_str()),
                                ),
                                ("occurred_at", py(row.get("occurred_at"))),
                                ("wallet", py(row.get("wallet"))),
                                ("amount_msat", int_py(int_or_zero(row.get("amount_msat"))?)),
                                ("external_id", or_empty_py(row.get("external_id"))),
                            ]))
                        })
                        .collect::<Result<_, AppError>>()?,
                ),
            ),
        ]))
    };
    Ok(dict([
        (
            "presumed_external_outbound",
            collect("external_presumed", "outbound", true)?,
        ),
        (
            "unclassified_inbound",
            collect("unclassified_origin", "inbound", false)?,
        ),
    ]))
}

fn or_empty_py(value: &Sql) -> Py {
    if truthy(value) {
        py(value)
    } else {
        s("")
    }
}

fn group_key(item: &Item) -> String {
    let evidence = &item.evidence;
    let text = |key: &str| ev(evidence, key).map(py_str).unwrap_or_default();
    match item.reason.as_str() {
        "custody_quantity_unresolved" => format!(
            "{}:{}:{}",
            item.reason,
            text("blocker_code"),
            text("gap_id")
        ),
        "ownership_transfer_source_missing" => {
            let mut ids: Vec<String> = match ev(evidence, "missing_source_wallets") {
                Some(Py::List(items)) => items
                    .iter()
                    .map(|w| get(w, "id").map(py_str).unwrap_or_default())
                    .collect(),
                _ => vec![],
            };
            ids.sort();
            format!("{}:{}", item.reason, ids.join(","))
        }
        "insufficient_lots" | "missing_cost_basis" => format!(
            "{}:{}:{}",
            item.reason,
            py_str(&item.asset),
            text("wallet_label")
        ),
        _ => item.reason.clone(),
    }
}

struct Item {
    transaction_id: String,
    reason: String,
    occurred_at: Py,
    wallet: Py,
    external_id: Py,
    amount_msat: Py,
    direction: Py,
    asset: Py,
    category: &'static str,
    blocks: bool,
    downstream: bool,
    root: Py,
    evidence: Vec<(Key, Py)>,
    actions: Py,
    fields: Vec<(Key, Py)>,
}

/// The review summary fields and the listed page.
type Page = (Vec<(Key, Py)>, Vec<Py>);

/// `review_quarantine` for one page.
fn review(
    conn: &Connection,
    profile: &Row,
    limit: i64,
    offset: &BigInt,
    scope: &str,
) -> Result<Page, AppError> {
    let profile_id = py_str(&py(profile.get("id")));
    let id_param = Sql::Text(profile_id.clone());
    let wallets = wallets(conn, &id_param)?;
    let blocking = blocking_ids(conn, &id_param)?;
    let workspace = py_str(&py(profile.get("workspace_id")));
    let mut pairs: HashMap<String, Py> = HashMap::new();
    for record in kassiber_transfers::pair_review_records(conn, &workspace, &profile_id)? {
        for key in ["out_transaction_id", "in_transaction_id"] {
            if let Some(id) = field(&record, key).filter(|v| py_truthy(v)) {
                pairs.entry(py_str(id)).or_insert_with(|| record.clone());
            }
        }
    }
    let rows = query(
        conn,
        "SELECT
            q.transaction_id, q.reason, q.detail_json, q.created_at,
            t.external_id, t.occurred_at, t.confirmed_at, t.direction,
            t.asset, t.amount, t.fee, t.wallet_id,
            w.label AS wallet
        FROM journal_quarantines q
        JOIN transactions t ON t.id = q.transaction_id
        JOIN wallets w ON w.id = t.wallet_id
        WHERE q.profile_id = ?",
        std::slice::from_ref(&id_param),
    )?;
    let mut held: Vec<(String, Held)> = Vec::new();
    let mut details: HashMap<String, Py> = HashMap::new();
    let mut by_id: HashMap<String, &Row> = HashMap::new();
    for row in &rows {
        let id = py_str(&py(row.get("transaction_id")));
        let entry = Held {
            reason: py_str(&py(row.get("reason"))),
            occurred_at: py(row.get("occurred_at")),
            asset: py(row.get("asset")),
        };
        match held.iter_mut().find(|(existing, _)| *existing == id) {
            Some(slot) => slot.1 = entry,
            None => held.push((id.clone(), entry)),
        }
        details.insert(id.clone(), parse_detail(row.get("detail_json")));
        by_id.insert(id, row);
    }
    let roots = waiting_roots(&held, &details);
    let mut items = Vec::new();
    for row in &rows {
        let transaction_id = py_str(&py(row.get("transaction_id")));
        let reason_name = py_str(&py(row.get("reason")));
        let detail = details[&transaction_id].clone();
        let info = reason_info(&reason_name, &detail);
        let root = match roots.get(&transaction_id) {
            Some(root_id) => {
                let root_row = by_id[root_id];
                dict([
                    (
                        "transaction_id",
                        s(py_str(&py(root_row.get("transaction_id"))).as_str()),
                    ),
                    ("reason", s(py_str(&py(root_row.get("reason"))).as_str())),
                    ("occurred_at", py(root_row.get("occurred_at"))),
                    ("wallet", py(root_row.get("wallet"))),
                    ("external_id", or_empty_py(root_row.get("external_id"))),
                ])
            }
            None => Py::Null,
        };
        let mut evidence = evidence(&detail, row, &wallets)?;
        if ev(&evidence, "blocker_code")
            .is_some_and(|code| py_str(code) == "reviewed_residual_suspense")
        {
            if let Some(pair) = pairs.get(&transaction_id) {
                for (key, value) in pair_evidence(conn, &transaction_id, pair)? {
                    match evidence.iter_mut().find(|(k, _)| *k == key) {
                        Some(slot) => slot.1 = value,
                        None => evidence.push((key, value)),
                    }
                }
            }
        }
        let mut reasons = vec![s(reason_name.as_str())];
        if let Some(Py::List(entries)) =
            field(&detail, "additional_reasons").filter(|v| py_truthy(v))
        {
            for entry in entries {
                if let Some(extra) =
                    get(entry, "reason").filter(|v| matches!(entry, Py::Dict(_)) && py_truthy(v))
                {
                    let extra = py_str(extra);
                    if extra != reason_name {
                        reasons.push(s(extra.as_str()));
                    }
                }
            }
        }
        let blocks = blocking.contains(&transaction_id);
        let actions = actions(
            info,
            &transaction_id,
            &py(row.get("wallet_id")),
            &evidence,
            &root,
            &wallets,
        );
        let fields = vec![
            (Key::from("transaction_id"), s(transaction_id.as_str())),
            (
                Key::from("external_id"),
                or_empty_py(row.get("external_id")),
            ),
            (Key::from("occurred_at"), py(row.get("occurred_at"))),
            (Key::from("confirmed_at"), py(row.get("confirmed_at"))),
            (Key::from("wallet"), py(row.get("wallet"))),
            (Key::from("direction"), py(row.get("direction"))),
            (Key::from("asset"), py(row.get("asset"))),
            (
                Key::from("amount_msat"),
                int_py(int_or_zero(row.get("amount"))?),
            ),
            (Key::from("fee_msat"), int_py(int_or_zero(row.get("fee"))?)),
            (Key::from("reason"), s(reason_name.as_str())),
            (Key::from("detail"), detail.clone()),
            (Key::from("created_at"), py(row.get("created_at"))),
            (Key::from("category"), s(info.category)),
            (Key::from("blocks_reports"), Py::Bool(blocks)),
            (Key::from("is_downstream"), Py::Bool(info.downstream())),
            (Key::from("root"), root.clone()),
            (Key::from("reasons"), Py::List(reasons)),
            (Key::from("evidence"), Py::Dict(evidence.clone())),
            (Key::from("actions"), actions.clone()),
        ];
        items.push(Item {
            transaction_id: transaction_id.clone(),
            reason: reason_name,
            occurred_at: py(row.get("occurred_at")),
            wallet: py(row.get("wallet")),
            external_id: or_empty_py(row.get("external_id")),
            amount_msat: int_py(int_or_zero(row.get("amount"))?),
            direction: py(row.get("direction")),
            asset: py(row.get("asset")),
            category: info.category,
            blocks,
            downstream: info.downstream(),
            root,
            evidence,
            actions,
            fields,
        });
    }
    items.sort_by(|a, b| {
        let key = |item: &Item| {
            (
                u8::from(item.downstream),
                u8::from(!item.blocks),
                text_or_empty(Some(&item.occurred_at)),
                item.transaction_id.clone(),
            )
        };
        key(a).cmp(&key(b))
    });

    struct Group {
        key: String,
        fields: Vec<(Key, Py)>,
        count: usize,
        downstream: usize,
        roots: usize,
        root_ids: Vec<Py>,
        has_root: bool,
        blocks: bool,
        wallets: Vec<Py>,
        earliest: Py,
        category: &'static str,
    }
    let mut group_keys = Vec::with_capacity(items.len());
    let mut groups: Vec<Group> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for position in 0..items.len() {
        let item = &items[position];
        let anchor = get(&item.root, "transaction_id").map(py_str);
        let key = if item.downstream && anchor.is_some() {
            match items
                .iter()
                .find(|other| Some(&other.transaction_id) == anchor.as_ref())
            {
                Some(anchor_item) => group_key(anchor_item),
                None => format!("downstream:{}", item.reason),
            }
        } else if item.downstream {
            format!("downstream:{}", item.reason)
        } else {
            group_key(item)
        };
        group_keys.push(key.clone());
        let slot = *index.entry(key.clone()).or_insert_with(|| {
            groups.push(Group {
                key: key.clone(),
                fields: vec![
                    (Key::from("key"), s(key.as_str())),
                    (Key::from("category"), s(item.category)),
                    (Key::from("reason"), s(item.reason.as_str())),
                    (Key::from("root_transaction_id"), Py::Null),
                    (Key::from("root_occurred_at"), Py::Null),
                    (Key::from("root_wallet"), Py::Null),
                    (Key::from("root_external_id"), Py::Null),
                    (Key::from("root_amount_msat"), Py::Null),
                    (Key::from("root_direction"), Py::Null),
                    (Key::from("root_asset"), Py::Null),
                ],
                count: 0,
                downstream: 0,
                roots: 0,
                root_ids: vec![],
                has_root: false,
                blocks: false,
                wallets: vec![],
                earliest: Py::Null,
                category: item.category,
            });
            groups.len() - 1
        });
        let group = &mut groups[slot];
        group.count += 1;
        let mut evidence_actions = None;
        if item.downstream {
            group.downstream += 1;
        } else {
            group.roots += 1;
            if group.root_ids.len() < MAX_GROUP_ROOTS {
                group.root_ids.push(s(item.transaction_id.as_str()));
            }
            if !group.has_root {
                group.has_root = true;
                group.category = item.category;
                group.fields = vec![
                    (Key::from("key"), s(group.key.as_str())),
                    (Key::from("category"), s(item.category)),
                    (Key::from("reason"), s(item.reason.as_str())),
                    (
                        Key::from("root_transaction_id"),
                        s(item.transaction_id.as_str()),
                    ),
                    (Key::from("root_occurred_at"), item.occurred_at.clone()),
                    (Key::from("root_wallet"), item.wallet.clone()),
                    (Key::from("root_external_id"), item.external_id.clone()),
                    (Key::from("root_amount_msat"), item.amount_msat.clone()),
                    (Key::from("root_direction"), item.direction.clone()),
                    (Key::from("root_asset"), item.asset.clone()),
                ];
                evidence_actions = Some((Py::Dict(item.evidence.clone()), item.actions.clone()));
            }
        }
        if let Some((evidence, actions)) = evidence_actions {
            group.fields.push((Key::from("evidence"), evidence));
            group.fields.push((Key::from("actions"), actions));
        }
        group.blocks = group.blocks || item.blocks;
        if py_truthy(&item.wallet)
            && !group.wallets.contains(&item.wallet)
            && group.wallets.len() < 5
        {
            group.wallets.push(item.wallet.clone());
        }
        if py_truthy(&item.occurred_at)
            && (matches!(group.earliest, Py::Null)
                || text_or_empty(Some(&item.occurred_at)) < text_or_empty(Some(&group.earliest)))
        {
            group.earliest = item.occurred_at.clone();
        }
    }
    let mut ordered: Vec<&Group> = groups.iter().collect();
    ordered.sort_by(|a, b| {
        let key = |g: &Group| {
            (
                u8::from(!g.blocks),
                u8::from(g.category == DOWNSTREAM),
                text_or_empty(Some(&g.earliest)),
                g.key.clone(),
            )
        };
        key(a).cmp(&key(b))
    });
    let group_payload = |group: &Group| -> Py {
        let mut fields = group.fields[..10].to_vec();
        fields.extend([
            (Key::from("count"), Py::Int(group.count.into())),
            (
                Key::from("downstream_count"),
                Py::Int(group.downstream.into()),
            ),
            (Key::from("blocks_reports"), Py::Bool(group.blocks)),
            (Key::from("wallets"), Py::List(group.wallets.clone())),
            (Key::from("earliest_occurred_at"), group.earliest.clone()),
        ]);
        let tail = &group.fields[10..];
        fields.push((
            Key::from("evidence"),
            tail.iter()
                .find(|(k, _)| *k == Key::from("evidence"))
                .map(|(_, v)| v.clone())
                .unwrap_or(dict::<&str>([])),
        ));
        fields.push((
            Key::from("actions"),
            tail.iter()
                .find(|(k, _)| *k == Key::from("actions"))
                .map(|(_, v)| v.clone())
                .unwrap_or(Py::List(vec![])),
        ));
        fields.push((
            Key::from("root_transaction_ids"),
            Py::List(group.root_ids.clone()),
        ));
        fields.push((Key::from("root_count"), Py::Int(group.roots.into())));
        Py::Dict(fields)
    };
    let mut by_category: BTreeMap<&str, usize> = BTreeMap::new();
    for item in &items {
        *by_category.entry(item.category).or_default() += 1;
    }
    let waiting = |item: &Item| item.downstream && !matches!(item.root, Py::Null);
    let waiting_count = items.iter().filter(|i| waiting(i)).count();
    let listed: Vec<Py> = items
        .iter()
        .zip(&group_keys)
        .filter(|(item, _)| scope == "all" || waiting(item) == (scope == "waiting"))
        .map(|(item, key)| {
            let mut fields = item.fields.clone();
            fields.push((Key::from("group_key"), s(key.as_str())));
            Ok(Py::Dict(fields))
        })
        .collect::<Result<_, AppError>>()?;
    let scope_count = listed.len();
    let start = usize::try_from(offset.clone())
        .unwrap_or(usize::MAX)
        .min(listed.len());
    let end = start
        .saturating_add(usize::try_from(limit).unwrap_or(0))
        .min(listed.len());
    let page = listed[start..end].to_vec();
    let held_ids: BTreeSet<String> = held.iter().map(|(id, _)| id.clone()).collect();
    let summary = vec![
        (
            Key::from("freshness"),
            journal_freshness(conn, &profile_id)?,
        ),
        (
            Key::from("blocking_count"),
            Py::Int(items.iter().filter(|i| i.blocks).count().into()),
        ),
        (Key::from("reports_blocked"), Py::Bool(!blocking.is_empty())),
        (
            Key::from("by_category"),
            Py::List(
                CATEGORIES
                    .iter()
                    .filter_map(|c| {
                        by_category
                            .get(c)
                            .map(|n| dict([("category", s(*c)), ("count", Py::Int((*n).into()))]))
                    })
                    .collect(),
            ),
        ),
        (
            Key::from("groups"),
            Py::List(
                ordered
                    .iter()
                    .take(MAX_GROUPS)
                    .map(|g| group_payload(g))
                    .collect(),
            ),
        ),
        (Key::from("group_count"), Py::Int(ordered.len().into())),
        (
            Key::from("assumptions"),
            assumptions(conn, &id_param, &held_ids)?,
        ),
        (
            Key::from("attention_count"),
            Py::Int((items.len() - waiting_count).into()),
        ),
        (Key::from("waiting_count"), Py::Int(waiting_count.into())),
        (Key::from("scope"), s(scope)),
        (Key::from("scope_count"), Py::Int(scope_count.into())),
    ];
    Ok((summary, page))
}

/// `build_journals_quarantine_snapshot`.
pub fn snapshot(conn: &Connection, args: Option<&Py>) -> Result<Py, AppError> {
    let args = args_object(args)?;
    unknown(KIND, &args, &["limit", "offset", "scope"])?;
    let limit = limit(&args, 20, MAX_UI_PREVIEW_LIMIT)?;
    let get_arg = |name: &str| {
        args.iter()
            .find(|(k, _)| *k == Key::from(name))
            .map(|(_, v)| v)
    };
    let offset = match get_arg("offset") {
        None => BigInt::from(0),
        Some(Py::Int(n)) if *n >= BigInt::from(0) => n.clone(),
        Some(_) => {
            return Err(AppError::new(
                "validation",
                format!("{KIND} offset must be a non-negative integer"),
            ))
        }
    };
    let scope = match get_arg("scope") {
        None => "all",
        Some(Py::Str(text)) => match SCOPES.iter().find(|s| text.is(s)) {
            Some(scope) => scope,
            None => return Err(scope_error()),
        },
        Some(_) => return Err(scope_error()),
    };
    let context = active(conn)?;
    let Some(profile) = &context.profile else {
        return Ok(dict([
            (
                "summary",
                dict([
                    ("workspace", Py::Null),
                    ("profile", Py::Null),
                    ("count", Py::Int(0.into())),
                    ("by_reason", Py::List(vec![])),
                    ("limit", Py::Int(limit.into())),
                    ("offset", Py::Int(offset)),
                ]),
            ),
            ("items", Py::List(vec![])),
        ]));
    };
    let reason_rows = query(
        conn,
        "SELECT reason, COUNT(*) AS count
        FROM journal_quarantines
        WHERE profile_id = ?
        GROUP BY reason
        ORDER BY count DESC, reason ASC",
        &[profile.get("id").clone()],
    )?;
    let mut total = BigInt::from(0);
    let mut by_reason = Vec::new();
    for row in &reason_rows {
        let count = int_or_zero(row.get("count"))?;
        total += count.clone();
        by_reason.push(dict([
            ("reason", py(row.get("reason"))),
            ("count", Py::Int(count)),
        ]));
    }
    let (review_summary, page) = review(conn, profile, limit, &offset, scope)?;
    let mut summary = vec![
        (Key::from("workspace"), label(&context, "workspace_label")),
        (Key::from("profile"), label(&context, "profile_label")),
        (
            Key::from("workspace_id"),
            s(py_str(&py(profile.get("workspace_id"))).as_str()),
        ),
        (
            Key::from("profile_id"),
            s(py_str(&py(profile.get("id"))).as_str()),
        ),
        // A price entered for a held row is in this currency.
        (Key::from("fiat_currency"), {
            let currency = py(profile.get("fiat_currency"));
            let upper = if py_truthy(&currency) {
                kassiber_pytext::upper(&py_str(&currency))
            } else {
                String::new()
            };
            if upper.is_empty() {
                Py::Null
            } else {
                s(upper)
            }
        }),
        (Key::from("count"), Py::Int(total)),
        (Key::from("by_reason"), Py::List(by_reason)),
        (Key::from("limit"), Py::Int(limit.into())),
        (Key::from("offset"), Py::Int(offset)),
    ];
    summary.extend(review_summary);
    summary.push((
        Key::from("definitions"),
        dict(SUMMARY_DEFINITIONS.iter().map(|(k, v)| (*k, s(*v)))),
    ));
    let items = page
        .into_iter()
        .map(|item| {
            let Py::Dict(mut fields) = item else {
                unreachable!()
            };
            let amount = fields
                .iter()
                .find(|(k, _)| *k == Key::from("amount_msat"))
                .map(|(_, v)| v.clone())
                .unwrap_or(Py::Null);
            let fee = fields
                .iter()
                .find(|(k, _)| *k == Key::from("fee_msat"))
                .map(|(_, v)| v.clone())
                .unwrap_or(Py::Null);
            let to_sql = |value: &Py| match value {
                Py::Int(n) => Sql::Text(n.to_string()),
                _ => Sql::Null,
            };
            fields.push((Key::from("amount"), Py::Float(btc(&to_sql(&amount))?)));
            fields.push((Key::from("fee"), Py::Float(btc(&to_sql(&fee))?)));
            Ok(Py::Dict(fields))
        })
        .collect::<Result<_, AppError>>()?;
    Ok(dict([
        ("summary", Py::Dict(summary)),
        ("items", Py::List(items)),
    ]))
}

fn scope_error() -> AppError {
    AppError::new(
        "validation",
        format!("{KIND} scope must be one of: {}", SCOPES.join(", ")),
    )
}
