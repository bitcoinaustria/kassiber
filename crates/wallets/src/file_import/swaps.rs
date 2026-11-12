//! Pairs the two legs of each swap an import stored
//! (docs/plan/23-crypto-assets.md). A provider that reports a
//! crypto-to-crypto trade gives both legs the same `swap_ref`; once both rows
//! are stored, the import authors the pair through reviewed pair authoring,
//! in the same write. The pair's policy is the one the profile's country
//! recommends (carrying value where basis carries across assets, as in
//! Austria; taxable elsewhere), so the tax engine decides the treatment.
use super::*;

/// Authors one pair per `swap_ref` among `rows` (in step with `records`) and
/// returns how many it created, or `None` when no row names a swap. A swap
/// whose legs are already paired keeps its pair, so a re-run creates none.
pub(super) fn pair(
    conn: &Connection,
    profile: &str,
    wallet: &str,
    rows: &[csv::Row],
    records: &[Record],
) -> Result<Option<i64>, AppError> {
    // swap_ref -> (outbound external ids, inbound external ids)
    let mut legs: BTreeMap<String, (Vec<String>, Vec<String>)> = BTreeMap::new();
    for (row, record) in rows.iter().zip(records) {
        let reference = text(&row.data, "swap_ref");
        if reference.is_empty() {
            continue;
        }
        let field = |key: &str| match record.fields.get(key) {
            Some(Sql::Text(value)) => value.clone(),
            _ => String::new(),
        };
        let entry = legs.entry(reference).or_default();
        match field("direction").as_str() {
            "outbound" => entry.0.push(field("external_id")),
            _ => entry.1.push(field("external_id")),
        }
    }
    if legs.is_empty() {
        return Ok(None);
    }
    let workspace: String = conn
        .query_row(
            "SELECT workspace_id FROM profiles WHERE id=?",
            [profile],
            |r| r.get(0),
        )
        .map_err(database_error)?;
    let scope = kassiber_transfers::Scope {
        workspace_id: &workspace,
        profile_id: profile,
    };
    let paired = kassiber_transfers::paired_transaction_ids(conn, &scope)?;
    let mut created = 0;
    for (reference, (outbound, inbound)) in legs {
        let ([out], [into]) = (outbound.as_slice(), inbound.as_slice()) else {
            return Err(AppError::new(
                "protocol_error",
                format!(
                    "Swap '{reference}' has {} outgoing and {} incoming legs; a swap needs exactly one of each",
                    outbound.len(),
                    inbound.len()
                ),
            ));
        };
        let (out, into) = (stored(conn, wallet, out)?, stored(conn, wallet, into)?);
        if paired.contains(&out) || paired.contains(&into) {
            continue;
        }
        kassiber_transfers::execute(
            conn,
            &scope,
            "transfers.pair",
            &dict([
                ("tx_out", s(out)),
                ("tx_in", s(into)),
                ("kind", s("manual")),
                ("pair_source", s("provider")),
                (
                    "notes",
                    s(format!("Swap reported by the provider ({reference})")),
                ),
            ]),
        )?;
        created += 1;
    }
    Ok(Some(created))
}

/// The id of the wallet's stored row with this external id.
fn stored(conn: &Connection, wallet: &str, external_id: &str) -> Result<String, AppError> {
    conn.query_row(
        "SELECT id FROM transactions WHERE wallet_id=? AND external_id=?",
        [wallet, external_id],
        |r| r.get(0),
    )
    .map_err(database_error)
}
