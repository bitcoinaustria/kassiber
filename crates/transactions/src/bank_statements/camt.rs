//! ISO 20022 camt.053 (bank-to-customer statement), versions .02 to .08.
//! Elements are matched by local name; the namespace only gates the version.
use super::{
    account_id, collapse, fiat, invalid, iso_date, Money, ParsedStatement, Result, StatementLine,
};
use kassiber_envelope::{dict, s};
use roxmltree::{Document, Node, ParsingOptions};

const NAMESPACE: &str = "urn:iso:std:iso:20022:tech:xsd:camt.";

fn child<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    node.children()
        .find(|c| c.is_element() && c.tag_name().name() == name)
}

fn children<'a, 'i>(node: Node<'a, 'i>, name: &'a str) -> impl Iterator<Item = Node<'a, 'i>> + 'a {
    node.children()
        .filter(move |c| c.is_element() && c.tag_name().name() == name)
}

fn text(node: Node<'_, '_>, names: &[&str]) -> Option<String> {
    let found = names.iter().try_fold(node, |n, name| child(n, name))?;
    found.text().and_then(collapse)
}

/// `Dt` or the date part of `DtTm`; `Some(None)` when absent.
fn date(node: Node<'_, '_>, name: &str) -> Option<Option<String>> {
    let Some(holder) = child(node, name) else {
        return Some(None);
    };
    let raw = text(holder, &["Dt"]).or_else(|| text(holder, &["DtTm"]))?;
    let p: Vec<i64> = raw
        .get(..10)?
        .split('-')
        .map(|v| v.parse().ok())
        .collect::<Option<_>>()?;
    match (raw.as_bytes().get(4), p.as_slice()) {
        (Some(b'-'), [y, m, d]) => iso_date(*y as i32, *m as u32, *d as u32).map(Some),
        _ => None,
    }
}

fn non_placeholder(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.eq_ignore_ascii_case("NOTPROVIDED") && !v.eq_ignore_ascii_case("NONREF"))
}

fn remittance(node: Node<'_, '_>) -> Option<String> {
    let info = child(node, "RmtInf")?;
    let mut parts: Vec<String> = children(info, "Ustrd")
        .filter_map(|n| n.text().and_then(collapse))
        .collect();
    if parts.is_empty() {
        for structured in children(info, "Strd") {
            parts.extend(text(structured, &["CdtrRefInf", "Ref"]));
            parts.extend(
                children(structured, "AddtlRmtInf").filter_map(|n| n.text().and_then(collapse)),
            );
        }
    }
    collapse(&parts.join(" "))
}

/// What kind of XML error and where, never the file's own text.
fn xml_error(error: &roxmltree::Error) -> kassiber_envelope::AppError {
    let kind = match error {
        roxmltree::Error::DtdDetected => "a DTD is not allowed",
        roxmltree::Error::NodesLimitReached => "the file is too large",
        _ => "the XML is malformed",
    };
    let pos = error.pos();
    invalid(format!(
        "The statement is not readable XML: {kind} (line {}, column {})",
        pos.row, pos.col
    ))
    .with_details(dict([("reason", s("bank_camt_xml"))]))
}

pub(super) fn parse(input: &str) -> Result<ParsedStatement> {
    let options = ParsingOptions {
        allow_dtd: false,
        nodes_limit: 4_000_000,
        ..ParsingOptions::default()
    };
    let document = Document::parse_with_options(input, options).map_err(|e| xml_error(&e))?;
    let root = document.root_element();
    let namespace = root.tag_name().namespace().unwrap_or_default();
    let version = namespace
        .strip_prefix(NAMESPACE)
        .and_then(|rest| rest.strip_prefix("053.001."))
        .and_then(|v| v.parse::<u32>().ok());
    if !namespace.is_empty() && !version.is_some_and(|v| (2..=8).contains(&v)) {
        // Names the expected namespaces only, never the file's.
        return Err(invalid(
            "This XML file is not a camt.053 account statement: it needs the namespace urn:iso:std:iso:20022:tech:xsd:camt.053.001.02 to .08",
        )
        .with_hint("Download the end-of-day account statement (camt.053) from your bank."));
    }
    let statements = child(root, "BkToCstmrStmt")
        .ok_or_else(|| invalid("The XML file has no BkToCstmrStmt statement"))?;
    let mut parsed = ParsedStatement::default();
    let mut position = 0;
    for statement in children(statements, "Stmt") {
        let account = child(statement, "Acct");
        let own = account
            .and_then(|a| text(a, &["Id", "IBAN"]).or_else(|| text(a, &["Id", "Othr", "Id"])))
            .and_then(|v| account_id(&v));
        if let Some(account) = account {
            if parsed.currency.is_none() {
                parsed.currency = text(account, &["Ccy"])
                    .map(|c| fiat(&c, "The statement account"))
                    .transpose()?;
            }
            let institution = child(account, "Svcr").and_then(|n| child(n, "FinInstnId"));
            if let Some(institution) = institution {
                parsed.bank_name = parsed
                    .bank_name
                    .take()
                    .or_else(|| text(institution, &["Nm"]));
                parsed.bank_bic = parsed.bank_bic.take().or_else(|| {
                    text(institution, &["BIC"]).or_else(|| text(institution, &["BICFI"]))
                });
            }
        }
        if parsed.statement_id.is_none() {
            parsed.statement_id = text(statement, &["Id"]);
        }
        for entry in children(statement, "Ntry") {
            position += 1;
            // `Sts` is the code itself (camt.053.001.02), `Sts/Cd` or `Sts/Prtry` (later
            // versions). Only an ISO BOOK books: a proprietary status means what the bank says,
            // and a status that does not read is not booked either.
            let status =
                child(entry, "Sts").map(|sts| match (text(sts, &["Cd"]), text(sts, &["Prtry"])) {
                    (Some(code), _) => code,
                    (None, Some(proprietary)) if proprietary != "BOOK" => proprietary,
                    (None, Some(_)) => String::new(),
                    (None, None) => text(sts, &[]).unwrap_or_default(),
                });
            if let Some(status) = status.filter(|s| s != "BOOK") {
                let known = ["PDNG", "INFO", "FUTR"].into_iter().find(|k| *k == status);
                parsed.skipped.push((
                    position,
                    match known {
                        Some(known) => format!("entry status {known} is not booked"),
                        None => "entry status is not booked".to_owned(),
                    },
                ));
                continue;
            }
            let fail = |what: &str| invalid(format!("Entry {position}: {what}"));
            let amt = child(entry, "Amt").ok_or_else(|| fail("no Amt"))?;
            let value = Money::parse(amt.text().unwrap_or_default().trim())
                .filter(|v| !v.is_negative())
                .ok_or_else(|| fail("the amount is not a plain decimal"))?;
            let currency = match amt.attribute("Ccy") {
                Some(code) => fiat(code, &format!("Entry {position}"))?,
                None => return Err(fail("amount has no valid Ccy currency")),
            };
            let debit = match text(entry, &["CdtDbtInd"]).as_deref() {
                Some("DBIT") => true,
                Some("CRDT") => false,
                _ => return Err(fail("CdtDbtInd must be CRDT or DBIT")),
            };
            let booking = date(entry, "BookgDt").ok_or_else(|| fail("invalid booking date"))?;
            let value_date = date(entry, "ValDt").ok_or_else(|| fail("invalid value date"))?;
            let booking = booking
                .or_else(|| value_date.clone())
                .ok_or_else(|| fail("no booking or value date"))?;
            let details: Vec<Node> = children(entry, "NtryDtls")
                .flat_map(|d| children(d, "TxDtls"))
                .collect();
            // A batch (several details) is one line at entry level: its parts
            // are never split out or guessed.
            let detail = match details.as_slice() {
                [one] => Some(*one),
                _ => None,
            };
            let role = if debit { "Cdtr" } else { "Dbtr" };
            let party = detail.and_then(|d| child(d, "RltdPties"));
            let joined: Vec<String> = details.iter().filter_map(|d| remittance(*d)).collect();
            parsed.lines.push(StatementLine {
                position,
                account: own.clone(),
                booking_date: booking,
                value_date,
                amount: if debit { value.negated() } else { value },
                currency,
                counterparty_name: party
                    .and_then(|p| child(p, role))
                    .and_then(|p| text(p, &["Nm"]).or_else(|| text(p, &["Pty", "Nm"]))),
                counterparty_iban: party
                    .and_then(|p| child(p, &format!("{role}Acct")))
                    .and_then(|a| {
                        text(a, &["Id", "IBAN"]).or_else(|| text(a, &["Id", "Othr", "Id"]))
                    })
                    .and_then(|v| account_id(&v)),
                remittance: collapse(&joined.join(" "))
                    .or_else(|| remittance(entry))
                    .or_else(|| text(entry, &["AddtlNtryInf"])),
                bank_reference: detail
                    .and_then(|d| non_placeholder(text(d, &["Refs", "AcctSvcrRef"])))
                    .or_else(|| non_placeholder(text(entry, &["AcctSvcrRef"]))),
                end_to_end_id: detail
                    .and_then(|d| non_placeholder(text(d, &["Refs", "EndToEndId"]))),
            });
        }
    }
    if position == 0 {
        return Err(invalid("The camt.053 file contains no Stmt/Ntry entries"));
    }
    Ok(parsed)
}
