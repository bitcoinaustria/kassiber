use super::*;
use kassiber_wallets::psbt_privacy::{Baseline, Proposal};

mod inventory;

/// Native inventory-only checkpoint. Product routes remain gated until the
/// shared historical graph can be admitted, rather than silently omitted.
pub fn report_inventory(conn: &Connection, args: &Value, psbt_text: &str) -> Result<Value> {
    report_with_baseline(conn, args, psbt_text, inventory::capture)
}

pub fn report_with_baseline(
    conn: &Connection,
    args: &Value,
    psbt_text: &str,
    build_baseline: impl FnOnce(&Connection, &str) -> Result<Baseline>,
) -> Result<Value> {
    conn.execute_batch("SAVEPOINT psbt_privacy_read")
        .map_err(sql_error)?;
    let result = (|| {
        let (workspace, profile) = kassiber_setup::references::scope(conn, args)?;
        let proposal = Proposal::decode(psbt_text)?;
        let baseline = build_baseline(conn, &text(&profile, "id"))?;
        let mut payload = proposal.analyze(&baseline)?;
        payload["scope"] = serde_json::json!({"workspace":"active","profile":"active",
            "workspace_bound":truthy(value(&workspace,"id")),"profile_bound":truthy(value(&profile,"id"))});
        kassiber_pytext::loads(&payload.to_string())
            .map_err(|_| AppError::new("internal_error", "Invalid PSBT report projection"))
    })();
    conn.execute_batch("RELEASE SAVEPOINT psbt_privacy_read")
        .map_err(sql_error)?;
    result
}

pub fn table_rows(payload: &Value) -> Value {
    let findings = match value(payload, "findings") {
        Value::List(rows) => rows.as_slice(),
        _ => &[],
    };
    let rows: Vec<_> = findings
        .iter()
        .filter(|r| matches!(r, Value::Dict(_)))
        .map(|r| {
            dict([
                ("severity", value(r, "severity").clone()),
                ("kind", value(r, "kind").clone()),
                ("result", value(r, "id").clone()),
                ("evidence_level", value(r, "evidence_level").clone()),
                ("detail", value(r, "detail").clone()),
            ])
        })
        .collect();
    if !rows.is_empty() {
        return Value::List(rows);
    }
    let summary = value(payload, "summary");
    Value::List(vec![dict([
        ("severity",s("info")),("kind",s("psbt_privacy")),("result",s("summary")),
        ("evidence_level",value(summary,"evidence_level").clone()),
        ("detail",s(format!("Decoded PSBT privacy analysis produced no finding rows; cluster_merge_delta={}, unknown_input_count={}.",
            py_str(get(summary,"cluster_merge_delta").unwrap_or(&Value::Int(0.into()))),
            py_str(get(summary,"unknown_input_count").unwrap_or(&Value::Int(0.into())))))),
    ])])
}
