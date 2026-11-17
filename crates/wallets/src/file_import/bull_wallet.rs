//! Bull Bitcoin's unified wallet-history export (`bullbitcoin_wallet_csv`):
//! Bitcoin, Liquid and Lightning rows in one file, imported per network
//! wallet. Preimages are hashed into the payment hash and redacted from the
//! stored row; failed, expired and refunded rows are skipped, as are pending
//! rows the app exported without a date.
//!
//! Bull's app (v6.11 onwards) writes one header in three formatter
//! generations. v6.13+ guards formula-like cells with a leading `'`, adds the
//! `aborted` payjoin status, reports a chain swap's outgoing fee as the real
//! on-chain fee, and gives a payjoin's negotiated amount instead of the
//! wallet's net. Only the guard and `aborted` identify a v6.13+ file; nothing
//! identifies the older generations, so the import never guesses one.
//!
//! Lightning rows are Boltz swaps paid from or to the Bull Bitcoin or Liquid
//! wallet; their txid is that wallet's on-chain lockup or claim, and the
//! export does not say which wallet. They book only where the book has
//! already observed that txid ([`route`]); otherwise they are held for review.
use super::exchange::Columns;
use super::*;

const REQUIRED: &[&str] = &[
    "date",
    "type",
    "direction",
    "amount_sats",
    "amount_btc",
    "fee_sats",
    "status",
    "txid",
    "network",
    "address",
    "swap_id",
    "preimage",
    "total_swap_fees_sats",
    "send_network",
    "receive_network",
    "send_txid",
    "receive_txid",
];

/// The row field carrying what the import needs beyond the normalized record
/// (see `file_import::Bull`). It never reaches the stored `raw_json`.
pub(super) const MARK: &str = "_bullbitcoin_wallet";
/// A pending row without a date: nothing to book until it confirms.
const UNDATED_PENDING: &str = "undated_pending";
/// A self-transfer without a miner fee moves nothing.
const SELF_WITHOUT_FEE: &str = "self_without_fee";
/// A Lightning row whose lockup/claim txid this wallet has not observed.
const LIGHTNING_NOT_OBSERVED: &str = "lightning_not_observed_in_wallet";
/// A Lightning row whose txid another wallet of the book already holds.
const LIGHTNING_ELSEWHERE: &str = "lightning_booked_in_other_wallet";
/// Review reason: the Lightning row's paying/receiving network is unproven.
pub(super) const REVIEW_LIGHTNING: &str = "lightning_network_unproven";
/// Review reason: the payjoin amount may be negotiated rather than the net.
pub(super) const REVIEW_PAYJOIN: &str = "payjoin_amount_unverified";

fn app(message: String) -> AppError {
    AppError::new("app_error", message)
}

/// `load_bullbitcoin_wallet_csv_records` over loaded CSV rows: the rows to
/// import, without those skipped as undated or fee-less.
pub(super) fn parse(rows: Vec<csv::Row>) -> Result<Vec<csv::Row>, AppError> {
    Ok(parse_all(rows)?
        .into_iter()
        .filter(|row| skip_reason(row).is_none())
        .collect())
}

/// Every non-terminal row, including those marked to be skipped, so a
/// wallet's import can count its own skipped rows.
pub(super) fn parse_all(mut rows: Vec<csv::Row>) -> Result<Vec<csv::Row>, AppError> {
    exchange::headers(&rows, REQUIRED, "Bull Bitcoin wallet")?;
    // v6.13+ prefixes formula-like cells with `'` for spreadsheets. Remove
    // exactly that guard so a guarded value parses as itself: a guarded
    // negative amount still fails closed below.
    let mut generation_c = false;
    for row in &mut rows {
        let headers = row.headers.clone();
        if let Json::Object(fields) = &mut row.data {
            for key in headers {
                if let Some(Json::String(text)) = fields.get_mut(&key) {
                    if let Some(rest) = text.strip_prefix('\'') {
                        if rest.starts_with(['=', '+', '-', '@', '\t', '\r']) {
                            *text = rest.to_owned();
                            generation_c = true;
                        }
                    }
                }
            }
        }
        generation_c |= folded(Columns::new(row).get(&["status"])) == "aborted";
    }
    let mut out = Vec::new();
    for (index, row) in rows.into_iter().enumerate() {
        if let Some(record) = record(row, index + 1, generation_c)? {
            out.push(record);
        }
    }
    Ok(out)
}

fn skip_reason(row: &csv::Row) -> Option<&str> {
    row.data[MARK]["skip"].as_str()
}

/// `str(value or "").strip().casefold()`.
fn folded(value: Option<String>) -> String {
    kassiber_pytext::casefold(kassiber_pytext::strip(&value.unwrap_or_default()))
}

/// `_bullbitcoin_wallet_sats(...)` as the digits of an exact, non-negative
/// integer. Bull never exports a negative count; one is refused rather than
/// read as its absolute value.
fn sats(columns: &Columns, column: &str, default: bool) -> Result<String, AppError> {
    let value = match river::number(columns.get(&[column]))? {
        Some(PriceNumber::Finite(value)) => value,
        Some(PriceNumber::Special(text)) if text.contains("sNaN") => {
            return Err(decimal_error(DecimalError::InvalidOperation));
        }
        // `NaN != NaN`, while `int(Infinity)` overflows.
        Some(PriceNumber::Special(text)) if text.contains("NaN") => {
            return Err(app(format!(
                "Bull Bitcoin wallet CSV has non-integer {column}: {text}"
            )));
        }
        Some(PriceNumber::Special(_)) => {
            return Err(AppError::new(
                "internal_error",
                "cannot convert Infinity to integer",
            ));
        }
        None if default => return Ok("0".into()),
        None => {
            return Err(app(format!(
                "Bull Bitcoin wallet CSV has a row with empty {column}"
            )))
        }
    };
    let fixed = value.to_fixed_string().map_err(decimal_error)?;
    let (integral, fraction) = fixed.split_once('.').unwrap_or((&fixed, ""));
    if fraction.bytes().any(|b| b != b'0') {
        return Err(app(format!(
            "Bull Bitcoin wallet CSV has non-integer {column}: {value}"
        )));
    }
    if value.is_negative() {
        return Err(app(format!(
            "Bull Bitcoin wallet CSV has negative {column}: {value}"
        )));
    }
    Ok(integral.trim_start_matches('-').to_owned())
}

fn btc(sats: &str) -> Result<String, AppError> {
    Ok(dec(sats)?
        .div(&dec("100000000")?, &CTX)
        .map_err(decimal_error)?
        .to_string())
}

/// The row's direction; `None` for a self-transfer.
fn direction(raw: Option<String>) -> Result<Option<&'static str>, AppError> {
    let value = folded(raw.clone());
    if ["incoming", "inbound", "received", "receive"].contains(&value.as_str()) {
        return Ok(Some("inbound"));
    }
    if ["outgoing", "outbound", "sent", "send"].contains(&value.as_str()) {
        return Ok(Some("outbound"));
    }
    if value == "self" {
        return Ok(None);
    }
    Err(app(format!(
        "Unsupported Bull Bitcoin wallet direction '{}'",
        raw.unwrap_or_else(|| "None".into())
    )))
}

fn network(columns: &Columns, kind: &str, direction: &str) -> Result<&'static str, AppError> {
    let mut found = columns.opt(&["network"]);
    if found.is_none() && kind == "chain_swap" {
        found = columns.opt(&[if direction == "inbound" {
            "receive_network"
        } else {
            "send_network"
        }]);
    }
    if found.is_none() {
        found = match kind {
            "liquid" => Some("liquid".into()),
            "onchain" | "payjoin_send" | "payjoin_receive" => Some("bitcoin".into()),
            "lightning_send" | "lightning_receive" => Some("lightning".into()),
            _ => None,
        };
    }
    match folded(found.clone()).as_str() {
        "bitcoin" | "btc" | "onchain" => Ok("bitcoin"),
        "liquid" | "lbtc" | "liquidv1" => Ok("liquid"),
        "lightning" | "ln" => Ok("lightning"),
        _ => Err(app(format!(
            "Unsupported Bull Bitcoin wallet network '{}'",
            found.unwrap_or_else(|| "None".into())
        ))),
    }
}

/// `str_or_none(value).strip().lower()` when it is 64 characters that
/// `bytes.fromhex` accepts (ASCII whitespace may separate byte pairs).
fn hex64(value: Option<String>) -> Option<Vec<u8>> {
    let text = kassiber_pytext::lower(kassiber_pytext::strip(&value?));
    if text.chars().count() != 64 {
        return None;
    }
    let mut bytes = Vec::new();
    let mut chars = text.chars().peekable();
    loop {
        while chars.next_if(|c| " \t\n\r\x0b\x0c".contains(*c)).is_some() {}
        let Some(high) = chars.next() else { break };
        let pair = [high.to_digit(16)?, chars.next()?.to_digit(16)?];
        bytes.push((pair[0] * 16 + pair[1]) as u8);
    }
    Some(bytes)
}

/// A row the import counts but does not book.
fn skipped(network: &str, reason: &str) -> csv::Row {
    csv::Row {
        headers: vec![],
        original: None,
        data: json!({"_bullbitcoin_wallet_network": network, MARK: {"skip": reason}}),
        rest: None,
    }
}

fn record(row: csv::Row, index: usize, generation_c: bool) -> Result<Option<csv::Row>, AppError> {
    let columns = Columns::new(&row);
    let kind = folded(columns.get(&["type"]));
    let status = folded(columns.get(&["status"]));
    if ["failed", "expired", "refunded"].contains(&status.as_str()) {
        return Ok(None);
    }
    let direction = direction(columns.get(&["direction"]))?;
    let network = network(&columns, &kind, direction.unwrap_or("outbound"))?;
    let date = columns.get(&["date"]);
    // The app leaves `date` empty only for an unconfirmed transaction it has
    // no time for; it books once a later export dates it.
    if date.is_none() && status == "pending" {
        return Ok(Some(skipped(network, UNDATED_PENDING)));
    }
    let mut amount = sats(&columns, "amount_sats", false)?;
    let fee = sats(&columns, "fee_sats", true)?;
    // A transfer between the wallet's own addresses moves no principal; its
    // miner fee is a fee-only outflow, as a chain observer books it.
    let self_transfer = direction.is_none();
    if self_transfer {
        if fee == "0" {
            return Ok(Some(skipped(network, SELF_WITHOUT_FEE)));
        }
        amount = "0".into();
    }
    let direction = direction.unwrap_or("outbound");
    let asset = if network == "liquid" || kind == "liquid" {
        "LBTC"
    } else {
        "BTC"
    };
    let external_id = ["txid", "receive_txid", "send_txid", "swap_id"]
        .iter()
        .find_map(|c| columns.opt(&[c]))
        .unwrap_or_else(|| {
            let cell = |c: &str| columns.get(&[c]);
            format!(
                "bullbitcoin-wallet:{}:{}:{}:{}",
                cell("date").unwrap_or_else(|| index.to_string()),
                cell("type").unwrap_or_else(|| "transaction".into()),
                cell("direction").unwrap_or_default(),
                cell("amount_sats").unwrap_or_default()
            )
        });
    // Only a valid preimage proves a Lightning payment hash. The row's txid
    // is the on-chain lockup or claim, never the payment hash.
    let payment_hash = if ["lightning_send", "lightning_receive"].contains(&kind.as_str()) {
        hex64(columns.get(&["preimage"])).map(|preimage| {
            use sha2::Digest;
            sha2::Sha256::digest(preimage)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        })
    } else {
        None
    };
    let mut raw = columns.json();
    let mut redacted = false;
    if let Json::Object(fields) = &mut raw {
        for (key, value) in fields.iter_mut() {
            let present = value
                .as_str()
                .is_some_and(|v| !kassiber_pytext::strip(v).is_empty());
            if river::column(key) == "preimage" && present {
                *value = json!("[redacted]");
                redacted = true;
            }
        }
        if redacted {
            fields.insert("preimage_redacted".into(), json!(true));
        }
        fields.insert("source".into(), json!("bullbitcoin_wallet_csv"));
        fields.insert("normalized_network".into(), json!(network));
        if generation_c {
            fields.insert("export_generation".into(), json!("v6.13+"));
        }
    }
    let (label, description) = if self_transfer {
        (
            "fee".to_owned(),
            "Bull Bitcoin wallet self-transfer fee".to_owned(),
        )
    } else {
        let label = if kind.is_empty() {
            "transaction".to_owned()
        } else {
            kind.clone()
        };
        let description = format!("Bull Bitcoin wallet {label}");
        (label, description)
    };
    let payjoin = ["payjoin_send", "payjoin_receive"].contains(&kind.as_str());
    let mut data = json!({
        "txid": external_id,
        "occurred_at": date,
        "confirmed_at": if ["confirmed", "completed"].contains(&status.as_str()) { date.clone() } else { None },
        "direction": direction,
        "asset": asset,
        "amount": btc(&amount)?,
        "fee": btc(&fee)?,
        "fiat_rate": null,
        "fiat_value": null,
        "kind": label,
        "description": description,
        "counterparty": null,
        "payment_hash": payment_hash,
        "payment_hash_source": payment_hash.as_ref().map(|_| "importer"),
        "_bullbitcoin_wallet_network": network,
        "_match_existing_ignore_fee": "True",
        "raw_json": config::stored_json(&raw),
        MARK: {
            "type": kind,
            "generation_c": generation_c,
            "match_only": false,
            // A payjoin amount is the wallet's net in v6.11-v6.12 and the
            // negotiated amount in v6.13+: only an observation settles it.
            "review": if payjoin { Some(REVIEW_PAYJOIN) } else { None },
        },
    });
    if payjoin {
        data["privacy_boundary"] = json!("payjoin");
    }
    Ok(Some(csv::Row {
        headers: vec![],
        original: None,
        data,
        rest: None,
    }))
}

/// `filter_bullbitcoin_wallet_records` for the wallet's configured network:
/// the plain network filter [`route`] applies before proving Lightning rows.
#[cfg(test)]
pub(super) fn filter(rows: Vec<csv::Row>, network: Option<&str>) -> Vec<csv::Row> {
    match network {
        None => rows,
        Some(network) => rows
            .into_iter()
            .filter(|row| row.data["_bullbitcoin_wallet_network"].as_str() == Some(network))
            .collect(),
    }
}

/// The rows one wallet books from an export, and the rows it skips by reason.
pub(crate) struct Routed {
    pub rows: Vec<csv::Row>,
    pub skipped: BTreeMap<&'static str, i64>,
}

impl Routed {
    /// `{reason: count}` for the import result, empty when nothing skipped.
    pub(crate) fn skipped_py(&self) -> Option<Py> {
        (!self.skipped.is_empty()).then(|| {
            dict(
                self.skipped
                    .iter()
                    .map(|(reason, count)| (*reason, Py::from(*count))),
            )
        })
    }
    /// Plain-English warnings for the skipped and held rows.
    pub(crate) fn warnings(&self, review: i64) -> Vec<String> {
        let mut out = Vec::new();
        for (reason, count) in &self.skipped {
            out.push(match *reason {
                UNDATED_PENDING => format!("Skipped {count} pending Bull Bitcoin wallet row(s) the app exported without a date; import a later export once they confirm."),
                SELF_WITHOUT_FEE => format!("Skipped {count} Bull Bitcoin wallet self-transfer row(s) without a miner fee."),
                LIGHTNING_NOT_OBSERVED => format!("Left {count} Bull Bitcoin wallet Lightning row(s) out of this wallet: their lockup or claim txid is not among its transactions, so the export cannot prove this wallet paid or received them."),
                LIGHTNING_ELSEWHERE => format!("Left {count} Bull Bitcoin wallet Lightning row(s) out: another wallet of this book already holds their txid."),
                other => format!("Skipped {count} Bull Bitcoin wallet row(s): {other}."),
            });
        }
        if review > 0 {
            out.push(format!("Held {review} Bull Bitcoin wallet row(s) out of accounting for review (tagged bullbitcoin-wallet-review); see the Bull Bitcoin wallet CSV import notes."));
        }
        out
    }
}

fn take_skipped(row: &csv::Row, skipped: &mut BTreeMap<&'static str, i64>) -> bool {
    let reason = match skip_reason(row) {
        Some(UNDATED_PENDING) => UNDATED_PENDING,
        Some(SELF_WITHOUT_FEE) => SELF_WITHOUT_FEE,
        Some(_) => "unsupported",
        None => return false,
    };
    *skipped.entry(reason).or_default() += 1;
    true
}

fn asset(network: &str) -> &'static str {
    if network == "liquid" {
        "LBTC"
    } else {
        "BTC"
    }
}

fn mark(row: &mut csv::Row, key: &str, value: Json) {
    row.data[MARK][key] = value;
}

/// Which wallets of the book hold a transaction with this external id.
fn holders(conn: &Connection, profile: &str, external_id: &str) -> Result<Vec<String>, AppError> {
    let mut statement = conn
        .prepare("SELECT DISTINCT wallet_id FROM transactions WHERE profile_id=? AND lower(external_id)=lower(?)")
        .map_err(database_error)?;
    let rows = statement
        .query_map([profile, external_id], |r| r.get(0))
        .map_err(database_error)?
        .collect::<Result<Vec<String>, _>>()
        .map_err(database_error)?;
    Ok(rows)
}

/// Route an export's rows to one wallet configured for `network` (all rows
/// for a wallet without one).
///
/// A Lightning row books only where it can be proven: a Bitcoin or Liquid
/// wallet takes it as a match against a transaction it already holds under
/// that txid (the observed lockup/claim keeps its amounts; the row adds its
/// swap metadata). A Lightning or unfiltered wallet holds it for review,
/// excluded from accounting, unless another wallet already holds the txid.
/// `conn` is `None` for enrichment, which only ever matches.
pub(crate) fn route(
    conn: Option<(&Connection, &str, &str)>,
    rows: Vec<csv::Row>,
    network: Option<&str>,
) -> Result<Routed, AppError> {
    let mut routed = Routed {
        rows: Vec::new(),
        skipped: BTreeMap::new(),
    };
    for mut row in rows {
        let own = row.data["_bullbitcoin_wallet_network"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let lightning = own == "lightning";
        let on_chain_wallet = matches!(network, Some("bitcoin" | "liquid"));
        let relevant = network.is_none_or(|n| n == own) || (lightning && on_chain_wallet);
        if !relevant {
            continue;
        }
        if take_skipped(&row, &mut routed.skipped) {
            continue;
        }
        if !lightning {
            routed.rows.push(row);
            continue;
        }
        let txid = row.data["txid"].as_str().unwrap_or_default().to_owned();
        if let Some(network) = network.filter(|_| on_chain_wallet) {
            row.data["asset"] = json!(asset(network));
            mark(&mut row, "match_only", json!(true));
            if let Some((conn, profile, wallet)) = conn {
                if !holders(conn, profile, &txid)?.iter().any(|w| w == wallet) {
                    *routed.skipped.entry(LIGHTNING_NOT_OBSERVED).or_default() += 1;
                    continue;
                }
            }
            routed.rows.push(row);
            continue;
        }
        if let Some((conn, profile, wallet)) = conn {
            let holders = holders(conn, profile, &txid)?;
            if !holders.is_empty() && !holders.iter().any(|w| w == wallet) {
                *routed.skipped.entry(LIGHTNING_ELSEWHERE).or_default() += 1;
                continue;
            }
        }
        mark(&mut row, "review", json!(REVIEW_LIGHTNING));
        routed.rows.push(row);
    }
    Ok(routed)
}
