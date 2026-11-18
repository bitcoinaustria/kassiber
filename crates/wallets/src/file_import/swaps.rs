//! Pairs the two legs of each swap an import stored
//! (docs/plan/24-crypto-assets.md). A provider that reports a
//! crypto-to-crypto trade gives both legs the same `swap_ref`; once both rows
//! are stored, the import authors the pair through reviewed pair authoring,
//! in the same write. The pair's policy is the one the profile's country
//! recommends (carrying value where basis carries across assets, as in
//! Austria; taxable elsewhere), so the tax engine decides the treatment.
use super::*;

/// Authors one pair per `swap_ref` among `rows` (in step with `records` and
/// `selected`, the transaction each record is now) and returns how many it
/// created, or `None` when no row names a swap. A swap whose legs are
/// already paired keeps its pair, so a re-run creates none. The legs are the
/// transactions this import stored or matched, never another row that
/// shares a leg's id (a corrected leg imported as a new row).
pub(super) fn pair(
    conn: &Connection,
    profile: &str,
    rows: &[csv::Row],
    records: &[Record],
    selected: &[Option<String>],
) -> Result<Option<i64>, AppError> {
    // swap_ref -> (outbound transactions, inbound transactions)
    type Legs = (Vec<Option<String>>, Vec<Option<String>>);
    let mut legs: BTreeMap<String, Legs> = BTreeMap::new();
    for ((row, record), transaction) in rows.iter().zip(records).zip(selected) {
        let reference = text(&row.data, "swap_ref");
        if reference.is_empty() {
            continue;
        }
        let entry = legs.entry(reference).or_default();
        match record.fields.get("direction") {
            Some(Sql::Text(direction)) if direction == "outbound" => {
                entry.0.push(transaction.clone())
            }
            _ => entry.1.push(transaction.clone()),
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
        // A leg the import left out (a match-only row) pairs nothing.
        let (Some(out), Some(into)) = (out, into) else {
            continue;
        };
        if paired.contains(out) || paired.contains(into) {
            continue;
        }
        kassiber_transfers::pair_provider_swap(
            conn,
            &scope,
            &dict([
                ("tx_out", s(out.as_str())),
                ("tx_in", s(into.as_str())),
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
