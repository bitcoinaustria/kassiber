use super::exchange::Columns;
use super::river::{abs, arithmetic, nonnegative, positive, repr, truth, zero};
use super::*;

const REQUIRED: &[&str] = &[
    "Type",
    "Buy Amount",
    "Buy Currency",
    "Sell Amount",
    "Sell Currency",
    "Date",
];
const FIAT: &[&str] = &[
    "AED", "ARS", "AUD", "BRL", "CAD", "CHF", "CLP", "CNY", "CZK", "DKK", "EUR", "GBP", "HKD",
    "HUF", "ILS", "INR", "JPY", "KRW", "MXN", "NOK", "NZD", "PLN", "RON", "SEK", "SGD", "THB",
    "TRY", "USD", "ZAR",
];
fn fail(message: impl Into<String>, hint: &str) -> AppError {
    AppError::new("validation", message).with_hint(hint)
}
fn number(
    value: Option<String>,
    field: &str,
    label: &str,
) -> Result<Option<PriceNumber>, AppError> {
    let Some(raw) = value else {
        return Ok(None);
    };
    let mut text = kassiber_pytext::strip(&raw).to_owned();
    let negative = text.starts_with('(') && text.ends_with(')');
    if negative {
        text = kassiber_pytext::strip(&text[1..text.len() - 1]).into();
    }
    for symbol in [
        "$", "€", "£", "CHF", "USD", "EUR", "GBP", "BTC", "XBT", "LBTC", "SATS", "SAT",
    ] {
        text = text.replace(symbol, "");
    }
    text = kassiber_pytext::strip(&text.replace(['\u{a0}', ' '], "")).into();
    if text.is_empty() {
        return Ok(None);
    }
    if text.contains('.') && text.contains(',') {
        text = if text.rfind('.') > text.rfind(',') {
            text.replace(',', "")
        } else {
            text.replace('.', "").replace(',', ".")
        };
    } else if text.contains(',') {
        text = if text.matches(',').count() > 1 {
            text.replace(',', "")
        } else {
            text.replace(',', ".")
        };
    } else if text.matches('.').count() > 1 {
        text = text.replace('.', "");
    }
    if negative && !text.starts_with('-') {
        text.insert(0, '-');
    }
    PriceNumber::parse(&text).map(Some).map_err(|_| {
        fail(
            format!(
                "{label}: {field} is not a number ({})",
                kassiber_pytext::repr_str(&raw)
            ),
            "Use digits like 0.05000000 or 3000,00 — amounts are in BTC unless the asset is SATS.",
        )
    })
}
fn date(value: Option<String>, index: usize) -> Result<String, AppError> {
    let text = value
        .as_deref()
        .map(kassiber_pytext::strip)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            fail(
                format!("Ledger row {index}: missing Date"),
                "Use an ISO date such as 2026-01-15, a full timestamp, or DD.MM.YYYY.",
            )
        })?;
    let mut candidate = text.to_owned();
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(&format!(
            r"^({d}{{1,2}})\.({d}{{1,2}})\.({d}{{4}})(?:[ T](.+))?$",
            d = kassiber_pytext::REGEX_DIGIT
        ))
        .unwrap()
    });
    if let Some(c) = re.captures(text) {
        let digits = |s: &str| -> String {
            s.chars()
                .map(|c| {
                    char::from(b'0' + kassiber_pytext::decimal_value(u32::from(c)).unwrap() as u8)
                })
                .collect()
        };
        candidate = format!(
            "{}-{:02}-{:02}",
            &c[3],
            digits(&c[2]).parse::<u32>().unwrap(),
            digits(&c[1]).parse::<u32>().unwrap()
        );
        if let Some(rest) = c.get(4) {
            candidate.push('T');
            candidate.push_str(kassiber_pytext::strip(rest.as_str()));
        }
    }
    crate::time::timestamp(&candidate).map_err(|_| {
        fail(
            format!("Ledger row {index}: Date '{text}' is not a recognized date"),
            "Use ISO 2026-01-15, a full RFC3339 timestamp, or DD.MM.YYYY.",
        )
    })?;
    Ok(candidate)
}
fn mapped(
    raw: Option<String>,
    inbound: bool,
) -> Result<(&'static str, &'static str, Option<&'static str>), AppError> {
    let text = raw
        .as_deref()
        .map(kassiber_pytext::strip)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            AppError::new(
                "validation",
                "CoinTracking row is missing its transaction type",
            )
        })?;
    let key = river::column(text);
    let (label,direction,kind) = match key.as_str() {
        "trade" if inbound => ("Buy","inbound",Some("buy")),
        "trade" => ("Sell","outbound",Some("sell")),
        "transfer" if inbound => ("Deposit","inbound",Some("deposit")),
        "transfer" => ("Withdrawal","outbound",Some("withdrawal")),
        "deposit" => ("Deposit","inbound",Some("deposit")),
        "withdrawal" => ("Withdrawal","outbound",Some("withdrawal")),
        "gift / tip" | "gift/tip(in)" | "income (non taxable)" | "other income (non taxable)" | "airdrop (non taxable)" => ("Gift received","inbound",None),
        "mining" | "mining (commercial)" => ("Mining","inbound",Some("mining")),
        "staking" => ("Staking","inbound",Some("staking")),
        "airdrop" => ("Airdrop","inbound",Some("airdrop")),
        "lending income" => ("Lending Interest","inbound",Some("lending_interest")),
        "interest income" => ("Interest","inbound",Some("interest")),
        "income" | "reward / bonus" | "masternode" | "minting" | "dividends income" | "lp rewards" | "other income" => ("Income","inbound",Some("income")),
        "spend" | "other fee" | "other expense" => ("Spend","outbound",Some("spend")),
        "donation" => ("Donation","outbound",Some("donation")),
        "gift" | "gift(out)" | "expense (non taxable)" => ("Gift sent","outbound",Some("gift")),
        "stolen" | "stolen / hacked / fraud" => ("Stolen","outbound",Some("stolen")),
        "lost" => ("Lost","outbound",Some("lost")),
        _ => return Err(fail(format!("CoinTracking transaction type '{text}' needs review before import"),"This type has tax semantics Kassiber cannot safely infer. Filter it into a separate export or convert the reviewed row with the Generic ledger template.")),
    };
    Ok((label, direction, kind))
}
pub(super) fn load(file: &str) -> Result<Vec<csv::Row>, AppError> {
    let (header, rows) = platform_csv::load(file)?;
    let missing: Vec<_> = REQUIRED
        .iter()
        .filter(|k| !header.iter().any(|h| river::column(h) == river::column(k)))
        .copied()
        .collect();
    if !missing.is_empty() {
        return Err(fail(
            format!(
                "CoinTracking CSV is missing required columns: {}",
                missing.join(", ")
            ),
            "Export the full transaction history from CoinTracking, not a summary or tax-form CSV.",
        ));
    }
    let mut occurrences = BTreeMap::<String, usize>::new();
    let mut result = vec![];
    for (index, row) in rows.into_iter().enumerate() {
        if let Some(mut row) = record(row, index + 1)? {
            let payload = dict([
                ("amount", number_float(&row.data["amount"])),
                ("asset", s(text(&row.data, "asset"))),
                ("direction", s(text(&row.data, "direction"))),
                ("fee", number_float(&row.data["fee"])),
                ("occurred_at", s(text(&row.data, "occurred_at"))),
            ]);
            let encoded = kassiber_pyjson::dumps(&payload, &Default::default()).unwrap();
            let digest = Sha256::digest(encoded.as_bytes())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>();
            let occurrence = occurrences.entry(digest.clone()).or_default();
            *occurrence += 1;
            row.data["_tax_platform_row_identity"] =
                json!(format!("cointracking:{digest}:{occurrence}"));
            result.push(row);
        }
    }
    Ok(result)
}
fn number_float(value: &Json) -> Py {
    let n: f64 = value.as_str().unwrap().parse().unwrap_or(f64::NAN);
    Py::Float(n)
}
fn record(row: csv::Row, index: usize) -> Result<Option<csv::Row>, AppError> {
    let c = Columns::new(&row);
    let code = |names: &[&str]| c.opt(names).map(|v| asset(&v));
    let incoming = code(&["Buy Currency"]);
    let mut outgoing = code(&["Sell Currency"]);
    let inbound = exchange::bitcoin(incoming.as_deref());
    let mut outbound = exchange::bitcoin(outgoing.as_deref());
    if inbound.is_some() && outbound.is_some() {
        return Err(fail(
            format!("CoinTracking row {index} has Bitcoin on both sides"),
            "Review BTC/LBTC swaps explicitly instead of importing them as a fiat trade.",
        ));
    }
    let mut fee_code = code(&["Fee Currency", "Fee Cur."]);
    let mut fee_raw = c.get(&["Fee", "Fee Amount"]);
    let mut fee = number(
        fee_raw.clone(),
        "Fee Amount",
        &format!("CoinTracking row {index}"),
    )?;
    let standalone =
        inbound.is_none() && outbound.is_none() && exchange::bitcoin(fee_code.as_deref()).is_some();
    let mut sent_raw = c.get(&["Sell Amount"]);
    if standalone {
        outgoing = fee_code.clone();
        outbound = exchange::bitcoin(outgoing.as_deref());
        sent_raw = fee_raw.take();
        fee_code = None;
        fee = None;
    }
    if inbound.is_none() && outbound.is_none() {
        return Ok(None);
    }
    let opposing = if inbound.is_some() {
        &outgoing
    } else {
        &incoming
    };
    if let Some(code) = opposing.as_deref().filter(|v| !FIAT.contains(v)) {
        return Err(fail(format!("CoinTracking row {index} trades Bitcoin against unsupported asset {code}"),"The provider CSV does not identify the currency behind its account-currency value columns. Review this cross-asset row with the Generic ledger and enter an explicit fiat currency and value."));
    }
    let (label, type_direction, mut kind) = if standalone {
        ("Spend", "outbound", Some("spend"))
    } else {
        mapped(c.get(&["Type"]), inbound.is_some())?
    };
    let fiat = opposing.as_deref().filter(|v| FIAT.contains(v));
    let accepted_fee = exchange::bitcoin(fee_code.as_deref()).is_some()
        || fee_code
            .as_deref()
            .is_some_and(|v| FIAT.contains(&v) && Some(v) == fiat);
    if !accepted_fee {
        if fee
            .as_ref()
            .map(|n| abs(n.clone()).and_then(|n| positive(&n)))
            .transpose()?
            .unwrap_or(false)
        {
            return Err(fail(format!("CoinTracking row {index} has a fee in unsupported asset {}",fee_code.as_deref().or(c.get(&["Fee Currency","Fee Cur."]).as_deref()).unwrap_or("None")),"Kassiber cannot safely convert a third-asset fee into the Bitcoin or fiat leg. Review this row with the Generic ledger template."));
        }
        fee_raw = None;
    }
    let ledger = format!("Ledger row {index}");
    let received = if inbound.is_some() || fiat.is_some() {
        number(c.get(&["Buy Amount"]), "Received Amount", &ledger)?
    } else {
        None
    };
    let sent = if outbound.is_some() || fiat.is_some() {
        number(sent_raw, "Sent Amount", &ledger)?
    } else {
        None
    };
    let fee = number(fee_raw, "Fee Amount", &ledger)?;
    let amount = if inbound.is_some() { received.clone() } else { sent.clone() }.ok_or_else(|| fail(format!("Ledger row {index}: no Bitcoin leg found (expected BTC, LBTC, or SATS on the Received or Sent side)"),"Put the Bitcoin amount + asset on the Received side (inbound) or the Sent side (outbound)."))?;
    let amount = abs(amount)?;
    let direction = if inbound.is_some() {
        "inbound"
    } else {
        "outbound"
    };
    if direction != type_direction {
        return Err(fail(format!("Ledger row {index}: Type '{label}' is {type_direction} but the Bitcoin leg is {direction}"),"Inbound Types (Buy/Deposit/Income/…) need the Bitcoin amount on the Received side; outbound Types (Sell/Withdrawal/Spend/Gift sent/…) on the Sent side."));
    }
    let (mut btc_fee, mut fiat_fee) = (zero(), zero());
    if let Some(fee) = fee {
        let fee = abs(fee)?;
        if positive(&fee)? {
            if exchange::bitcoin(fee_code.as_deref()).is_some() {
                btc_fee = fee;
            } else {
                let cash = if inbound.is_some() {
                    sent.as_ref()
                } else {
                    received.as_ref()
                };
                if cash.is_none() {
                    return Err(fail(format!("{ledger}: a fiat fee ({}) needs a matching cash leg",fee_code.as_deref().unwrap()),"Put network fees in BTC (Fee Asset BTC or blank), or add the Sent/Received fiat amount this fee applies to."));
                }
                fiat_fee = fee;
            }
        }
    }
    let cash = if inbound.is_some() { sent } else { received };
    let priced = cash.is_some() && fiat.is_some();
    let value = if let Some(cash) = cash.filter(|_| priced) {
        let base = abs(cash)?;
        Some(if direction == "inbound" {
            arithmetic(&base, &fiat_fee, '+')?
        } else {
            nonnegative(arithmetic(&base, &fiat_fee, '-')?)?
        })
    } else {
        None
    };
    let rate = if let Some(value) = &value {
        if truth(&amount) && positive(&amount)? {
            Some(arithmetic(value, &amount, '/')?)
        } else {
            None
        }
    } else {
        None
    };
    let occurred = date(c.get(&["Date"]), index)?;
    if river::column(&c.get(&["Type"]).unwrap_or_default()) == "expense (non taxable)" {
        kind = Some("expense_non_taxable");
    }
    let counterparty = c
        .opt(&["Exchange"])
        .unwrap_or_else(|| "CoinTracking".into());
    let note = [c.opt(&["Trade-Group"]), c.opt(&["Comment"])]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
    let txid = c.opt(&["Tx-ID (optional)", "Tx-ID", "Trade ID"]);
    Ok(Some(csv::Row {
        data: json!({"txid":txid.clone().unwrap_or_default(),"occurred_at":occurred,"direction":direction,"asset":inbound.or(outbound).unwrap(),"amount":repr(&amount),"fee":repr(&btc_fee),"fiat_value":value.as_ref().map(repr),"fiat_rate":rate.as_ref().map(repr),"fiat_currency":if priced { fiat } else { None },"pricing_source_kind":if priced {Some("exchange_execution")} else {None},"pricing_provider":if priced {Some(&counterparty)} else {None},"pricing_pair":if priced {Some(format!("{}-{}",inbound.or(outbound).unwrap(),fiat.unwrap()))} else {None},"pricing_method":"cointracking_csv","pricing_external_ref":txid,"pricing_quality":if priced {Some("exact")} else {None},"kind":kind,"description":if note.is_empty() {None} else {Some(note)},"counterparty":counterparty,"raw_json":config::stored_json(&c.json())}),
        headers: vec![],
        rest: None,
        original: None,
    }))
}

fn query(conn: &Connection, sql: &str, args: &[Sql]) -> Result<Vec<Fields>, AppError> {
    let mut stmt = conn.prepare(sql).map_err(database_error)?;
    let keys = stmt
        .column_names()
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();
    let rows = stmt
        .query_map(params_from_iter(args), |row| {
            keys.iter()
                .enumerate()
                .map(|(i, key)| Ok((key.clone(), row.get(i)?)))
                .collect()
        })
        .map_err(database_error)?;
    rows.collect::<Result<_, _>>().map_err(database_error)
}
fn chain_id(value: &str) -> Option<String> {
    let value = kassiber_pytext::strip(value);
    (value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| value.to_ascii_lowercase())
}
fn economic(row: &Fields) -> (String, String, String, i64) {
    let amount = if let Sql::Integer(amount) = row["amount"] {
        amount
    } else {
        0
    };
    (
        string(row, "occurred_at"),
        string(row, "direction"),
        string(row, "asset"),
        amount,
    )
}
pub(super) fn reconcile(
    conn: &Connection,
    request: &Request,
    wallet: &crate::Wallet,
    ids: &mut Ids<'_>,
    result: &mut Vec<(&str, Py)>,
    invalidated: bool,
) -> Result<(), AppError> {
    let workspace: String = conn
        .query_row(
            "SELECT workspace_id FROM profiles WHERE id=?",
            [&request.profile_id],
            |r| r.get(0),
        )
        .map_err(database_error)?;
    let authoritative = query(conn,"SELECT t.external_id,t.occurred_at,t.direction,t.asset,t.amount FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE t.profile_id=? AND w.kind IN ('descriptor','silent-payment','xpub')",&[Sql::Text(request.profile_id.clone())])?;
    let rows = query(conn,"SELECT t.id,t.external_id,t.occurred_at,t.direction,t.asset,t.amount,t.fee,t.excluded FROM transactions t WHERE t.wallet_id=? ORDER BY t.occurred_at,t.created_at,t.id",&[Sql::Text(wallet.id.clone())])?;
    let edits = query(conn,"SELECT e.transaction_id,f.after_value FROM transaction_edit_events e JOIN transaction_edit_fields f ON f.event_id=e.id JOIN transactions t ON t.id=e.transaction_id WHERE t.wallet_id=? AND f.field='excluded' ORDER BY e.transaction_id,e.journal_input_version_after DESC,e.changed_at DESC,e.id DESC,f.id DESC",&[Sql::Text(wallet.id.clone())])?;
    let mut authored = BTreeMap::new();
    for edit in edits {
        authored
            .entry(string(&edit, "transaction_id"))
            .or_insert_with(|| {
                serde_json::from_str::<Json>(&string(&edit, "after_value"))
                    .ok()
                    .and_then(|v| v.as_bool())
            });
    }
    let tags = query(conn,"SELECT id,code FROM tags WHERE profile_id=? AND code IN ('cointracking-matched','cointracking-ambiguous')",&[Sql::Text(request.profile_id.clone())])?;
    let mut tag_ids: BTreeMap<String, String> = tags
        .iter()
        .map(|r| (string(r, "code"), string(r, "id")))
        .collect();
    let affected: std::collections::BTreeSet<String> = result
        .iter()
        .filter(|(key, _)| ["inserted_records", "updated_records"].contains(key))
        .flat_map(|(_, value)| {
            if let Py::List(rows) = value {
                rows.iter().collect()
            } else {
                vec![]
            }
        })
        .filter_map(|row| {
            kassiber_envelope::get(row, "transaction_id")
                .and_then(|v| if let Py::Str(s) = v { s.as_str() } else { None })
                .map(str::to_owned)
        })
        .collect();
    let mut by_txid = BTreeMap::new();
    let mut by_economics = std::collections::BTreeSet::new();
    for row in &authoritative {
        if let Some(id) = chain_id(&string(row, "external_id")) {
            *by_txid
                .entry((id, string(row, "direction"), string(row, "asset")))
                .or_insert(0) += 1;
        }
        by_economics.insert(economic(row));
    }
    let mut source_counts = BTreeMap::new();
    let string_fee = |row: &Fields| {
        if let Sql::Integer(fee) = row["fee"] {
            fee
        } else {
            0
        }
    };
    for row in &rows {
        if string(row, "external_id").is_empty() {
            *source_counts
                .entry((economic(row), string_fee(row)))
                .or_insert(0) += 1;
        }
    }
    let (
        mut matched,
        mut ambiguous,
        mut unmatched,
        mut excluded,
        mut changed,
        mut cleared,
        mut reactivated,
        mut total,
    ) = (0, 0, 0, 0, 0, 0, 0, 0);
    for row in &rows {
        let id = string(row, "id");
        if !affected.contains(&id) {
            continue;
        }
        total += 1;
        let txid = chain_id(&string(row, "external_id"));
        let matches = txid
            .and_then(|id| {
                by_txid
                    .get(&(id, string(row, "direction"), string(row, "asset")))
                    .copied()
            })
            .unwrap_or(0);
        let collision = string(row, "external_id").is_empty()
            && source_counts
                .get(&(economic(row), string_fee(row)))
                .copied()
                .unwrap_or(0)
                > 1;
        let status = if matches > 0 {
            if matches == 1 {
                "matched"
            } else {
                "ambiguous"
            }
        } else if collision || by_economics.contains(&economic(row)) {
            "ambiguous"
        } else {
            "unmatched"
        };
        match status {
            "matched" => matched += 1,
            "ambiguous" => ambiguous += 1,
            _ => unmatched += 1,
        };
        let current = query(conn,"SELECT tags.code FROM transaction_tags tt JOIN tags ON tags.id=tt.tag_id WHERE tt.transaction_id=? AND tags.profile_id=? AND tags.code IN ('cointracking-matched','cointracking-ambiguous')",&[Sql::Text(id.clone()),Sql::Text(request.profile_id.clone())])?;
        let is_excluded = row.get("excluded") != Some(&Sql::Integer(0));
        let choice = authored.get(&id).copied().flatten();
        if status == "unmatched" {
            if !current.is_empty() {
                for tag in tag_ids.values() {
                    conn.execute(
                        "DELETE FROM transaction_tags WHERE transaction_id=? AND tag_id=?",
                        [&id, tag],
                    )
                    .map_err(database_error)?;
                }
                changed += 1;
                cleared += 1;
                if choice != Some(true) && is_excluded {
                    conn.execute("UPDATE transactions SET excluded=0 WHERE id=?", [&id])
                        .map_err(database_error)?;
                    reactivated += 1;
                }
            }
            continue;
        }
        let desired = choice.unwrap_or(true);
        excluded += i64::from(desired);
        let code = format!("cointracking-{status}");
        if current.len() == 1 && string(&current[0], "code") == code && is_excluded == desired {
            continue;
        }
        for (suffix, label) in [
            ("matched", "CoinTracking matched"),
            ("ambiguous", "CoinTracking ambiguous"),
        ] {
            let code = format!("cointracking-{suffix}");
            if let std::collections::btree_map::Entry::Vacant(entry) = tag_ids.entry(code) {
                let tag = ids.next(conn)?;
                conn.execute("INSERT INTO tags(id,workspace_id,profile_id,code,label,created_at) VALUES(?,?,?,?,?,?)",params![tag,workspace,request.profile_id,entry.key(),label,ids.time(conn)?]).map_err(database_error)?;
                entry.insert(tag);
            }
        }
        for tag in tag_ids.values() {
            conn.execute(
                "DELETE FROM transaction_tags WHERE transaction_id=? AND tag_id=?",
                [&id, tag],
            )
            .map_err(database_error)?;
        }
        conn.execute(
            "INSERT OR IGNORE INTO transaction_tags(transaction_id,tag_id) VALUES(?,?)",
            [&id, &tag_ids[&code]],
        )
        .map_err(database_error)?;
        conn.execute(
            "UPDATE transactions SET excluded=? WHERE id=?",
            params![i64::from(desired), id],
        )
        .map_err(database_error)?;
        changed += 1;
    }
    if changed > 0 && !invalidated {
        invalidate_journals(conn, &request.profile_id)?;
    }
    result
        .iter_mut()
        .find(|(k, _)| *k == "journal_invalidated")
        .unwrap()
        .1 = Py::Bool(invalidated || changed > 0);
    result.extend([
        ("matched", Py::from(matched)),
        ("ambiguous", Py::from(ambiguous)),
        ("unmatched", Py::from(unmatched)),
        ("excluded", Py::from(excluded)),
        ("reconciliation_rows", Py::from(total)),
        ("reconciliation_changed", Py::from(changed)),
    ]);
    if cleared > 0 {
        result.push(("reconciliation_flags_cleared", Py::from(cleared)));
    }
    if reactivated > 0 {
        result.push(("reactivated", Py::from(reactivated)));
    }
    Ok(())
}
