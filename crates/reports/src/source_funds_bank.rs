//! Accepting bank-statement lines as source-of-funds evidence
//! (docs/reference/bank-statements.md). Acceptance goes through the existing
//! source and link mutations; a bank line never carries basis or ownership.
use super::*;
use kassiber_setup::Sources;
use kassiber_store::rusqlite::params;
use kassiber_transactions::{
    bank_statements::{mask_ibans, matching, store, Money},
    Scope,
};

/// A reviewed link from this source already funds the purchase.
const EXISTING_SOURCE: &str = "SELECT s.* FROM source_funds_links l JOIN source_funds_sources s ON s.id=l.from_source_id WHERE l.profile_id=? AND l.to_transaction_id=? AND l.state='reviewed' AND s.source_type='fiat_purchase' ORDER BY s.created_at, s.id LIMIT 1";

/// Text of a column; NULL is empty (unlike `py_str`).
fn txt(row: &Value, key: &str) -> String {
    match value(row, key) {
        Value::Null => String::new(),
        other => py_str(other),
    }
}

fn ids(values: &[String]) -> Value {
    Value::List(values.iter().map(|v| s(v.as_str())).collect())
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::new("validation", message)
}

/// Accept bank lines as the fiat source of the named purchases: an explicit
/// review, so any pairing the user names is accepted once it is outgoing
/// bank money of one currency funding inbound BTC buys in that currency.
pub fn accept(
    conn: &Connection,
    scope: &Scope<'_>,
    lines: &[String],
    transactions: &[String],
    sources: &dyn Sources,
) -> Result<Value> {
    crate::export::admitted(conn, || {
        accept_inner(conn, scope, lines, transactions, sources)
    })
}

fn accept_inner(
    conn: &Connection,
    scope: &Scope<'_>,
    lines: &[String],
    transactions: &[String],
    sources: &dyn Sources,
) -> Result<Value> {
    let (line_rows, tx_rows) = matching::resolve_pairs(conn, scope, lines, transactions)?;
    let currency = line_rows[0].currency.clone();
    let mut bank_total = Money::ZERO;
    for line in &line_rows {
        let amount = Money::parse(&line.amount)
            .ok_or_else(|| AppError::new("internal_error", "stored bank amount is not exact"))?;
        if !amount.is_negative() {
            return Err(invalid(format!(
                "Bank line '{}' is incoming; only outgoing transfers can fund a purchase",
                line.id
            )));
        }
        if line.currency != currency {
            return Err(invalid("The bank lines use different currencies"));
        }
        bank_total = bank_total.plus(amount.abs());
    }
    for tx in &tx_rows {
        let kind = Some(txt(tx, "kind_override"))
            .filter(|k| !k.is_empty())
            .unwrap_or_else(|| txt(tx, "kind"));
        if txt(tx, "direction") != "inbound"
            || !txt(tx, "asset").eq_ignore_ascii_case("BTC")
            || !kind.eq_ignore_ascii_case("buy")
        {
            return Err(invalid(format!(
                "Transaction '{}' is not an inbound BTC purchase",
                txt(tx, "id")
            ))
            .with_hint("Bank lines support only inbound BTC transactions whose kind is buy."));
        }
        let fiat = txt(tx, "fiat_currency");
        if !fiat.is_empty() && !fiat.eq_ignore_ascii_case(&currency) {
            return Err(invalid(format!(
                "The purchase is priced in {fiat}, the bank lines in {currency}"
            )));
        }
        let other = rows(
            conn,
            "SELECT l.id FROM source_funds_links l LEFT JOIN source_funds_sources s ON s.id=l.from_source_id WHERE l.profile_id=? AND l.to_transaction_id=? AND l.state='reviewed' AND (s.id IS NULL OR s.source_type!='fiat_purchase') LIMIT 1",
            params![scope.profile_id, txt(tx, "id")],
        )?;
        if !other.is_empty() {
            return Err(AppError::new(
                "conflict",
                "This purchase already has a reviewed source-of-funds origin of another kind",
            )
            .with_hint("Review the purchase's existing source-funds links first.")
            .with_details(dict([("transaction", value(tx, "id").clone())])));
        }
    }
    let key = matching::group_key(lines, transactions);
    let suggested = matching::suggest_for_book(conn, scope, None)?;
    let found = match value(&suggested, "suggestions") {
        Value::List(items) => items.as_slice(),
        _ => &[],
    };
    let reasons = found
        .iter()
        .find(|item| txt(item, "id") == key)
        .map(|item| value(item, "reasons").clone())
        .unwrap_or_else(|| {
            Value::List(vec![s("accepted as an explicit pairing, not a suggestion")])
        });
    let reasons_json = kassiber_pyjson::dumps(&reasons, &kassiber_pyjson::Options::default())
        .map_err(|e| AppError::new("internal_error", e.to_string()))?;
    let scoped = |mut pairs: Vec<(&'static str, Value)>| {
        pairs.push(("workspace", s(scope.workspace_id)));
        pairs.push(("profile", s(scope.profile_id)));
        dict(pairs)
    };
    // Stored and never masked again: Kassiber's text only (date, amount,
    // curated exchange, the account's short form), no bank text.
    let summary: Vec<String> = line_rows
        .iter()
        .map(|l| {
            let mut item = format!("{} {} {}", l.booking_date, l.amount, l.currency);
            if let Some(exchange) = l.named_exchange() {
                item.push_str(&format!(" to {}", matching::exchange_name(exchange)));
            }
            if let Some(masked) = &l.account_masked {
                item.push_str(&format!(
                    " (account {})",
                    kassiber_transactions::bank_statements::account_short(masked)
                ));
            }
            item
        })
        .collect();
    let summary = summary.join("; ");
    let at = sources.now(conn)?;
    let mut accepted = Vec::new();
    for tx in &tx_rows {
        let tx_id = txt(tx, "id");
        let msat = match value(tx, "amount") {
            Value::Int(n) => i64::try_from(n.clone()).unwrap_or(0),
            _ => 0,
        };
        let btc = format!("{}.{:011}", msat / 100_000_000_000, msat % 100_000_000_000);
        let existing = rows(conn, EXISTING_SOURCE, params![scope.profile_id, tx_id])?;
        let (source_id, created) = match existing.first() {
            Some(found) => (txt(found, "id"), false),
            None => {
                let date = txt(tx, "occurred_at");
                let args = scoped(vec![
                    ("source_type", s("fiat_purchase")),
                    (
                        "label",
                        s(format!(
                            "Bank-funded purchase {}",
                            date.get(..10).unwrap_or(&date)
                        )),
                    ),
                    ("asset", s("BTC")),
                    ("amount", s(btc.as_str())),
                    ("fiat_currency", s(currency.as_str())),
                    ("acquired_at", s(date.as_str())),
                    (
                        "description",
                        s(format!(
                            "Paid by bank transfer: {summary}. Bank statement evidence only."
                        )),
                    ),
                    ("attachments", Value::List(vec![])),
                ]);
                let source = super::source_funds_mutations::execute(
                    conn,
                    "source-funds.sources.create",
                    &args,
                    sources,
                )?;
                (txt(&source, "id"), true)
            }
        };
        let args = scoped(vec![
            ("from_source", s(source_id.as_str())),
            ("to_transaction", s(tx_id.as_str())),
            ("link_type", s("manual_source")),
            ("state", s("reviewed")),
            ("confidence", s("strong")),
            ("method", s("bank_statement")),
            ("allocation_policy", s("explicit")),
            ("allocation_amount", s(btc.as_str())),
            (
                "explanation",
                s(format!("Fiat purchase paid by bank transfer: {summary}")),
            ),
            ("attachments", Value::List(vec![])),
        ]);
        let link =
            super::source_funds_links::execute(conn, "source-funds.links.create", &args, sources)?;
        for line in lines {
            conn.execute(
                "INSERT INTO bank_statement_line_decisions(line_id,transaction_id,profile_id,state,source_id,group_key,reasons_json,created_at,updated_at) VALUES(?,?,?,'accepted',?,?,?,?,?) ON CONFLICT(line_id,transaction_id) DO UPDATE SET state='accepted', source_id=excluded.source_id, group_key=excluded.group_key, reasons_json=excluded.reasons_json, updated_at=excluded.updated_at",
                params![line, tx_id, scope.profile_id, source_id, key, reasons_json, at, at],
            )
            .map_err(sql_error)?;
        }
        accepted.push(dict([
            ("transaction_id", s(tx_id.as_str())),
            ("source_id", s(source_id.as_str())),
            ("source_created", Value::Bool(created)),
            ("link_id", value(&link, "id").clone()),
        ]));
    }
    Ok(dict([
        ("state", s("accepted")),
        ("group_key", s(key)),
        ("line_ids", ids(lines)),
        ("transaction_ids", ids(transactions)),
        ("bank_total", s(bank_total.canonical())),
        ("currency", s(currency)),
        ("reasons", reasons),
        ("accepted", Value::List(accepted)),
    ]))
}

/// Accepted bank lines supporting one source, projected for the report's
/// disclosure mode: `labels_only` keeps date, amount and currency; `minimal`
/// adds the bank; `standard` and `full` add the counterparty, masked IBANs
/// and bank reference. Remittance text is never disclosed.
pub fn evidence(conn: &Connection, source_id: &str, mode: &str) -> Result<Vec<Value>> {
    let shown = |v: &Option<String>| v.as_deref().map_or(Value::Null, |t| s(mask_ibans(t)));
    let lines = store::accepted_lines(conn, source_id)?;
    Ok(lines
        .iter()
        .map(|line| {
            let mut item = vec![
                ("booking_date", s(line.booking_date.as_str())),
                ("amount", s(line.amount.as_str())),
                ("currency", s(line.currency.as_str())),
            ];
            if mode != "labels_only" {
                item.push(("bank_name", shown(&line.bank_name)));
            }
            if matches!(mode, "standard" | "full") {
                let opt = |v: &Option<String>| v.as_deref().map_or(Value::Null, s);
                item.push(("value_date", opt(&line.value_date)));
                item.push(("counterparty_name", shown(&line.counterparty_name)));
                item.push((
                    "counterparty_iban_masked",
                    opt(&line.counterparty_iban_masked),
                ));
                item.push(("account_masked", opt(&line.account_masked)));
                item.push(("bank_reference", shown(&line.bank_reference)));
            }
            dict(item)
        })
        .collect())
}
