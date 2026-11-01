use super::super::*;
use kassiber_wallets::psbt_privacy::{inventory, Baseline};

/// Admit source absence and capture inventory under the enclosing report read
/// savepoint. Unknown/missing schemas fail closed rather than imply no history.
pub(super) fn capture(conn: &Connection, profile: &str) -> Result<Baseline> {
    for (table, scoped) in [
        ("transaction_graph_cache", false),
        ("chain_analysis_observations", true),
        ("chain_analysis_reference_assertions", true),
        ("chain_analysis_datasets", true),
        ("book_network_bindings", true),
    ] {
        let query = format!(
            "SELECT EXISTS(SELECT 1 FROM {table}{})",
            if scoped { " WHERE profile_id=?" } else { "" }
        );
        let present: bool = if scoped {
            conn.query_row(&query, [profile], |r| r.get(0))
        } else {
            conn.query_row(&query, [], |r| r.get(0))
        }
        .map_err(sql_error)?;
        if present {
            return Err(inventory::unavailable());
        }
    }
    // Excluded observations still affect the public graph. Records without
    // external/raw evidence cannot create physical Bitcoin nodes or anchors.
    let history: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM transactions WHERE profile_id=? AND (COALESCE(external_id,'')!='' OR COALESCE(raw_json,'') NOT IN ('','{}')))",
        [profile], |r| r.get(0),
    ).map_err(sql_error)?;
    if history {
        return Err(inventory::unavailable());
    }
    let query = format!(
        "SELECT * FROM wallet_utxos WHERE profile_id=? LIMIT {}",
        inventory::MAX_ROWS + 1
    );
    let rows = rows(conn, &query, [profile])?;
    let rows = rows
        .iter()
        .map(|row| {
            let text = kassiber_pyjson::dumps(row, &kassiber_pyjson::Options::new().compact())
                .map_err(|_| inventory::unavailable())?;
            serde_json::from_str(&text).map_err(|_| inventory::unavailable())
        })
        .collect::<Result<Vec<_>>>()?;
    inventory::baseline(&rows)
}
