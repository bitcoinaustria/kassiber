//! Reviewed commercial evidence, separate from tax and journal readiness.
use super::*;
use std::path::Path;

const HEADERS: &[&str] = &[
    "occurred_at",
    "wallet",
    "external_id",
    "direction",
    "asset",
    "amount",
    "fee_msat",
    "fiat_currency",
    "fiat_value_exact",
    "pricing_source_kind",
    "commercial_kind",
    "invoice_id",
    "payment_id",
    "order_id",
    "payment_request_id",
    "origin_kind",
    "origin_label",
    "document_label",
    "document_external_ref",
    "reconciliation_state",
];

pub(super) fn report(conn: &Connection, profile: &Value) -> Result<Value> {
    let entries = rows(conn, "SELECT cl.commercial_kind,cl.reconciliation_state,
        t.id AS transaction_id,t.external_id,t.occurred_at,t.direction,t.asset,t.amount,t.fee,
        t.fiat_currency,t.fiat_value_exact,t.pricing_source_kind,w.label AS wallet,
        p.invoice_id,p.payment_id,p.order_id,p.payment_request_id,p.origin_kind,p.origin_label,p.stable_key,
        d.id AS document_id,d.label AS document_label,d.external_ref AS document_external_ref
        FROM commercial_links cl JOIN transactions t ON t.id=cl.transaction_id
        JOIN wallets w ON w.id=t.wallet_id
        LEFT JOIN btcpay_provenance_records p ON p.id=cl.btcpay_record_id
        LEFT JOIN external_documents d ON d.id=cl.document_id
        WHERE cl.profile_id=? AND cl.state='reviewed'
        ORDER BY t.occurred_at ASC,cl.created_at ASC", [text(profile, "id")])?;
    let mut result = Vec::new();
    for entry in entries {
        let mut row = dict([
            ("transaction_id", value(&entry, "transaction_id").clone()),
            ("external_id", value(&entry, "external_id").clone()),
            ("occurred_at", value(&entry, "occurred_at").clone()),
            ("wallet", value(&entry, "wallet").clone()),
            ("direction", value(&entry, "direction").clone()),
            ("asset", value(&entry, "asset").clone()),
            ("amount_msat", value(&entry, "amount").clone()),
            ("amount", float(&btc(int(&entry, "amount"))?)?),
            ("fee_msat", Value::Int(int(&entry, "fee").into())),
            ("fiat_currency", value(&entry, "fiat_currency").clone()),
            (
                "fiat_value_exact",
                value(&entry, "fiat_value_exact").clone(),
            ),
            (
                "pricing_source_kind",
                value(&entry, "pricing_source_kind").clone(),
            ),
            ("commercial_kind", value(&entry, "commercial_kind").clone()),
            (
                "reconciliation_state",
                value(&entry, "reconciliation_state").clone(),
            ),
            ("invoice_id", value(&entry, "invoice_id").clone()),
            ("payment_id", value(&entry, "payment_id").clone()),
            ("order_id", value(&entry, "order_id").clone()),
            (
                "payment_request_id",
                value(&entry, "payment_request_id").clone(),
            ),
            ("origin_kind", value(&entry, "origin_kind").clone()),
            ("origin_label", value(&entry, "origin_label").clone()),
            ("btcpay_ref", value(&entry, "stable_key").clone()),
            ("document_id", value(&entry, "document_id").clone()),
            ("document_label", value(&entry, "document_label").clone()),
            (
                "document_external_ref",
                value(&entry, "document_external_ref").clone(),
            ),
        ]);
        for key in [
            "external_id",
            "fiat_currency",
            "fiat_value_exact",
            "pricing_source_kind",
            "commercial_kind",
            "invoice_id",
            "payment_id",
            "order_id",
            "payment_request_id",
            "origin_kind",
            "origin_label",
            "btcpay_ref",
            "document_id",
            "document_label",
            "document_external_ref",
        ] {
            if !truthy(value(&row, key)) {
                replace(&mut row, key, s(""));
            }
        }
        result.push(row);
    }
    Ok(Value::List(result))
}

pub(super) fn export(conn: &Connection, args: &Value) -> Result<Value> {
    let Value::List(rows) = super::execute(conn, "reports.commercial-subledger", args)? else {
        unreachable!()
    };
    let mut csv = vec![HEADERS.iter().map(|h| s(*h)).collect()];
    csv.extend(
        rows.iter()
            .map(|row| HEADERS.iter().map(|h| value(row, h).clone()).collect()),
    );
    let path = text(args, "file");
    let path = Path::new(&path);
    super::export::create_directory(
        path.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )?;
    // This reference exporter uses an ordinary text file, with no saved report
    // marker or staging file. Preserve its path spelling, including relatives.
    super::export::write_synced(path, &artifact_csv::encode(&csv))
        .map_err(|e| super::export::io_error(e, Some(path)))?;
    Ok(dict([
        ("file", s(path.to_string_lossy().into_owned())),
        ("rows", Value::Int(rows.len().into())),
    ]))
}
