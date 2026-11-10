//! Profile-wide commercial evidence matching and excluded full-import rows.
//! This coordinator cannot establish chain ownership or tax input authority.
use super::*;

pub(super) struct Columns(Vec<(String, Json)>);
impl Columns {
    pub fn new(row: &csv::Row) -> Self {
        Self::collect(row, true)
    }
    pub fn unstripped(row: &csv::Row) -> Self {
        Self::collect(row, false)
    }
    fn collect(row: &csv::Row, strip: bool) -> Self {
        let mut entries: Vec<(String, Json)> = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for key in &row.headers {
            if !seen.insert(key) {
                continue;
            }
            let clean = if strip {
                kassiber_pytext::strip(key).to_owned()
            } else {
                key.clone()
            };
            let value = row.data[key].clone();
            if let Some((_, old)) = entries.iter_mut().find(|(k, _)| *k == clean) {
                *old = value;
            } else {
                entries.push((clean, value));
            }
        }
        Self(entries)
    }
    pub fn get(&self, names: &[&str]) -> Option<String> {
        names.iter().find_map(|name| {
            self.0
                .iter()
                .rev()
                .find(|(k, _)| river::column(k) == river::column(name))
                .and_then(|(_, v)| v.as_str())
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
        })
    }
    pub fn opt(&self, names: &[&str]) -> Option<String> {
        names.iter().find_map(|name| {
            self.get(&[name])
                .map(|s| kassiber_pytext::strip(&s).to_owned())
                .filter(|v| !v.is_empty())
        })
    }
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(|(key, _)| key.as_str())
    }
    pub fn json(&self) -> Json {
        Json::Object(self.0.iter().cloned().collect())
    }
}
pub(super) fn headers(
    rows: &[csv::Row],
    required: &[&str],
    provider: &str,
) -> Result<(), AppError> {
    let Some(first) = rows.first() else {
        return Ok(());
    };
    let missing = required
        .iter()
        .filter(|k| {
            !first
                .data
                .as_object()
                .unwrap()
                .keys()
                .any(|h| river::column(h) == river::column(k))
        })
        .copied()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(AppError::new(
            "app_error",
            format!(
                "{provider} CSV is missing required columns: {}",
                missing.join(", ")
            ),
        ))
    }
}
pub(super) fn bitcoin(currency: Option<&str>) -> Option<&'static str> {
    match currency? {
        "BTC" | "XBT" | "XXBT" => Some("BTC"),
        "LBTC" | "L-BTC" => Some("LBTC"),
        _ => None,
    }
}
pub(super) fn clear_legacy_flags(
    conn: &Connection,
    request: &Request,
    wallet: &crate::Wallet,
    records: &[Record],
    result: &mut Vec<(&str, Py)>,
) -> Result<(), AppError> {
    let mut tags = conn.prepare("SELECT id FROM tags WHERE profile_id=? AND code IN ('21bitcoin-matched','21bitcoin-wallet-gap','21bitcoin-ambiguous')").map_err(database_error)?;
    let tags = tags
        .query_map([&request.profile_id], |r| r.get::<_, String>(0))
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)?;
    if tags.is_empty() {
        return Ok(());
    }
    let mut cleared = 0;
    let mut reactivated = 0;
    for record in records {
        let imported = find(conn, &wallet.id, record, &fingerprint(&wallet.id, record))?;
        let imported = if imported.is_some() {
            imported
        } else {
            conn.query_row("SELECT id,excluded FROM transactions WHERE wallet_id=? AND pricing_external_ref=? AND direction=? AND asset=? AND amount=? ORDER BY created_at DESC LIMIT 1", params![wallet.id, record.fields["pricing_external_ref"],record.fields["direction"],record.fields["asset"],record.fields["amount"]], |r| Ok(BTreeMap::from([("id".into(),r.get(0)?),("excluded".into(),r.get(1)?)]))).optional().map_err(database_error)?
        };
        let Some(imported) = imported else {
            continue;
        };
        let id = string(&imported, "id");
        let mut tagged = false;
        for tag in &tags {
            tagged |= conn
                .execute(
                    "DELETE FROM transaction_tags WHERE transaction_id=? AND tag_id=?",
                    params![id, tag],
                )
                .map_err(database_error)?
                > 0;
        }
        if tagged {
            cleared += 1;
            reactivated += conn
                .execute(
                    "UPDATE transactions SET excluded=0 WHERE id=? AND excluded!=0",
                    [&id],
                )
                .map_err(database_error)?;
        }
    }
    if cleared > 0 {
        result.push(("reconciliation_flags_cleared", Py::from(cleared)));
    }
    if reactivated > 0 {
        result.push(("reactivated", Py::from(reactivated as i64)));
    }
    Ok(())
}
pub(super) fn mode(value: Option<&str>) -> Result<&'static str, AppError> {
    match kassiber_pytext::lower(kassiber_pytext::strip(value.filter(|v|!v.is_empty()).unwrap_or("relevant"))).as_str() {
        "full" => Ok("full"),
        "relevant" | "relevant-only" | "relevant_only" | "match" | "match-only" | "match_only" => Ok("relevant"),
        _ => Err(AppError::new("validation",format!("Unsupported exchange-evidence import mode '{}'",value.unwrap_or_default())).with_hint("Choose 'relevant' to enrich matching book transactions, or 'full' to import all provider rows with reconciliation flags.")),
    }
}
pub(super) fn wallet(
    conn: &Connection,
    request: &Request,
    ids: &mut Ids<'_>,
) -> Result<crate::Wallet, AppError> {
    if !request.wallet.is_empty() {
        return resolve(conn, &request.profile_id, &request.wallet);
    }
    let (label, kind, _) = request.format.provider();
    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM wallets WHERE profile_id=? AND lower(label)=lower(?) LIMIT 1",
            [&request.profile_id, label],
            |r| r.get(0),
        )
        .optional()
        .map_err(database_error)?;
    if let Some(id) = existing {
        return resolve(conn, &request.profile_id, &id);
    }
    let workspace: String = conn
        .query_row(
            "SELECT workspace_id FROM profiles WHERE id=?",
            [&request.profile_id],
            |r| r.get(0),
        )
        .map_err(database_error)?;
    let mut id = None;
    lifecycle::execute_with_creation_source(
        conn,
        lifecycle::Operation::Create {
            workspace_id: workspace,
            profile_id: request.profile_id.clone(),
            label: label.into(),
            kind: kind.into(),
            account: None,
            config: json!({"source_format":request.format.name()}),
        },
        &mut |conn| {
            let value = lifecycle::CreationIdentity {
                id: ids.next(conn)?,
                created_at: ids.time(conn)?,
            };
            id = Some(value.id.clone());
            Ok(value)
        },
    )?;
    resolve(conn, &request.profile_id, &id.expect("created wallet"))
}

pub(super) fn find_profile(
    conn: &Connection,
    profile: &str,
    r: &Record,
    exclude: Option<&str>,
) -> Result<(Option<Fields>, &'static str), AppError> {
    let external = string(&r.fields, "external_id");
    if r.economics {
        return find_economics(conn, profile, r, exclude);
    }
    if external.is_empty() {
        return Ok((None, "unmatched"));
    }
    // `btc_to_msat(normalized["fee"])`: a non-finite fee refuses here.
    if let Some(error) = &r.deferred {
        return Err(error.clone());
    }
    let query = |exact: bool| -> Result<Vec<Fields>, AppError> {
        let mut stmt=conn.prepare("SELECT t.*,w.label AS wallet_label,(SELECT authority_version FROM chain_observation_provenance WHERE transaction_id=t.id) AS observation_authority_version,(SELECT graph_hash FROM chain_observation_provenance WHERE transaction_id=t.id) AS observation_graph_hash,(SELECT quantity_hash FROM chain_observation_provenance WHERE transaction_id=t.id) AS observation_quantity_hash FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE t.profile_id=? AND external_id=? AND direction=? AND asset=? AND amount=? AND (NOT ? OR fee=?) AND (? IS NULL OR wallet_id!=?) ORDER BY t.created_at DESC LIMIT 2").map_err(database_error)?;
        let names = stmt
            .column_names()
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
        let rows = stmt
            .query_map(
                params![
                    profile,
                    external,
                    r.fields["direction"],
                    r.fields["asset"],
                    r.fields["amount"],
                    exact,
                    r.fields["fee"],
                    exclude,
                    exclude
                ],
                |row| {
                    names
                        .iter()
                        .enumerate()
                        .map(|(i, k)| Ok((k.clone(), row.get(i)?)))
                        .collect()
                },
            )
            .map_err(database_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error)?;
        Ok(rows)
    };
    let mut rows = query(true)?;
    if rows.is_empty() && string(&r.fields, "pricing_source_kind") == "exchange_execution" {
        rows = query(false)?;
    }
    let status = match rows.len() {
        0 => "unmatched",
        1 => "matched",
        _ => "ambiguous",
    };
    Ok((if rows.len() == 1 { rows.pop() } else { None }, status))
}
fn find_economics(
    conn: &Connection,
    profile: &str,
    r: &Record,
    exclude: Option<&str>,
) -> Result<(Option<Fields>, &'static str), AppError> {
    let occurred = string(&r.fields, "occurred_at");
    let start = crate::time::shifted(&occurred, -r.tolerance.max(0))?;
    let end = crate::time::shifted(&occurred, r.tolerance.max(0))?;
    let mut stmt = conn.prepare("SELECT w.label AS wallet_label,t.*,(SELECT authority_version FROM chain_observation_provenance WHERE transaction_id=t.id) AS observation_authority_version,(SELECT graph_hash FROM chain_observation_provenance WHERE transaction_id=t.id) AS observation_graph_hash,(SELECT quantity_hash FROM chain_observation_provenance WHERE transaction_id=t.id) AS observation_quantity_hash FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE t.profile_id=? AND direction=? AND asset=? AND amount=? AND (? IS NULL OR wallet_id!=?) AND NOT (excluded=1 AND pricing_method=?) AND occurred_at BETWEEN ? AND ? ORDER BY t.created_at DESC LIMIT 2").map_err(database_error)?;
    let names = stmt
        .column_names()
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();
    let mut rows = stmt
        .query_map(
            params![
                profile,
                r.fields["direction"],
                r.fields["asset"],
                r.fields["amount"],
                exclude,
                exclude,
                r.fields["pricing_method"],
                start,
                end
            ],
            |row| {
                names
                    .iter()
                    .enumerate()
                    .map(|(i, k)| Ok((k.clone(), row.get(i)?)))
                    .collect()
            },
        )
        .map_err(database_error)?
        .collect::<Result<Vec<Fields>, _>>()
        .map_err(database_error)?;
    let status = if rows.is_empty() {
        "unmatched"
    } else if rows.len() == 1 {
        "matched"
    } else {
        "ambiguous"
    };
    Ok((if rows.len() == 1 { rows.pop() } else { None }, status))
}
fn validate_price(currency: &str, r: &Record) -> Result<(), AppError> {
    let fiat = string(&r.fields, "fiat_currency");
    if !fiat.is_empty()
        && fiat != kassiber_pytext::upper(currency)
        && ["fiat_rate", "fiat_value"]
            .iter()
            .any(|k| r.fields.get(*k).is_some_and(|v| *v != Sql::Null))
    {
        return Err(AppError::new("validation",format!("Imported price currency {fiat} does not match profile fiat currency {}",kassiber_pytext::upper(currency))).with_hint("Import the file into a profile with the same fiat currency, or remove the imported fiat price values and price the transactions separately."));
    }
    Ok(())
}
pub(super) fn enrich(
    conn: &Connection,
    request: &Request,
    rows: &[csv::Row],
) -> Result<Py, AppError> {
    let currency: String = conn
        .query_row(
            "SELECT fiat_currency FROM profiles WHERE id=?",
            [&request.profile_id],
            |r| r.get(0),
        )
        .map_err(database_error)?;
    let (mut matched, mut unchanged, mut ambiguous, mut unmatched) = (0, 0, 0, 0);
    let mut updated = Vec::new();
    for row in rows {
        let r = normalize(row)?;
        let (existing, status) = find_profile(conn, &request.profile_id, &r, None)?;
        if let Some(existing) = existing {
            matched += 1;
            validate_price(&currency, &r)?;
            let changes = merge(&existing, &r, &string(&existing, "fingerprint"), false)?;
            if changes.is_empty() {
                unchanged += 1;
            } else {
                let id = string(&existing, "id");
                update(conn, "transactions", &id, &changes)?;
                updated.push(change(
                    &id,
                    &string(&existing, "wallet_label"),
                    &r,
                    changes.keys().cloned().collect(),
                ));
            }
        } else if status == "ambiguous" {
            ambiguous += 1;
        } else {
            unmatched += 1;
        }
    }
    if !updated.is_empty() {
        invalidate_journals(conn, &request.profile_id)?;
    }
    let mut result = vec![
        ("scope", s("book")),
        ("source", s(format!("file:{}", request.format.name()))),
        ("imported", Py::from(0)),
        ("skipped", Py::from(rows.len() as i64)),
        ("matched", Py::from(matched)),
        ("unchanged", Py::from(unchanged)),
        ("journal_invalidated", Py::Bool(!updated.is_empty())),
        ("skipped_unmatched", Py::from(unmatched)),
        ("skipped_ambiguous", Py::from(ambiguous)),
    ];
    let updated_count = updated.len() as i64;
    result.push(("updated_records", Py::List(updated)));
    if updated_count > 0 {
        result.push(("updated", Py::from(updated_count)));
    }
    result.extend([
        ("mode", s("relevant")),
        (request.format.provider().2, Py::from(rows.len() as i64)),
        ("input_format", s(request.format.name())),
        (
            "file",
            s(kassiber_paths::abspath(std::path::Path::new(&request.file))
                .map_err(|e| AppError::new("internal_error", e.to_string()))?
                .to_string_lossy()
                .into_owned()),
        ),
    ]);
    Ok(dict(result))
}
fn set(value: &mut Py, key: &str, item: Py) {
    if let Py::Dict(entries) = value {
        if let Some((_, v)) = entries
            .iter_mut()
            .find(|(k, _)| k == &kassiber_pyjson::Key::from(key))
        {
            *v = item;
        } else {
            entries.push((kassiber_pyjson::Key::from(key), item));
        }
    }
}
pub(super) fn reconcile(
    conn: &Connection,
    request: &Request,
    wallet: &crate::Wallet,
    records: &[Record],
    matches: &[(Option<Fields>, &str)],
    ids: &mut Ids<'_>,
    result: &mut Vec<(&str, Py)>,
) -> Result<(), AppError> {
    let workspace: String = conn
        .query_row(
            "SELECT workspace_id FROM profiles WHERE id=?",
            [&request.profile_id],
            |r| r.get(0),
        )
        .map_err(database_error)?;
    let mut reconciled = Vec::new();
    let (provider, slug, _) = request.format.provider();
    let mut excluded = 0;
    for (r, (existing, status)) in records.iter().zip(matches) {
        let Some(imported) = find(conn, &wallet.id, r, &fingerprint(&wallet.id, r))? else {
            continue;
        };
        let id = string(&imported, "id");
        for (tag_status, suffix, label) in [
            ("matched", "matched", "matched"),
            ("unmatched", "wallet-gap", "wallet gap"),
            ("ambiguous", "ambiguous", "ambiguous"),
        ] {
            let code = format!("{slug}-{suffix}");
            let found: Option<String> = conn
                .query_row(
                    "SELECT id FROM tags WHERE profile_id=? AND code=?",
                    [&request.profile_id, &code],
                    |r| r.get(0),
                )
                .optional()
                .map_err(database_error)?;
            let tag = if let Some(id) = found {
                id
            } else {
                let id = ids.next(conn)?;
                conn.execute("INSERT INTO tags(id,workspace_id,profile_id,code,label,created_at) VALUES(?,?,?,?,?,?)",params![id,workspace,request.profile_id,code,format!("{provider} {label}"),ids.time(conn)?]).map_err(database_error)?;
                id
            };
            conn.execute(
                "DELETE FROM transaction_tags WHERE transaction_id=? AND tag_id=?",
                [&id, &tag],
            )
            .map_err(database_error)?;
            if status == &tag_status {
                conn.execute(
                    "INSERT OR IGNORE INTO transaction_tags(transaction_id,tag_id) VALUES(?,?)",
                    [&id, &tag],
                )
                .map_err(database_error)?;
            }
        }
        conn.execute("UPDATE transactions SET excluded=1 WHERE id=?", [&id])
            .map_err(database_error)?;
        excluded += 1;
        let mut record = change(&id, &wallet.label, r, vec!["reconciliation".into()]);
        set(&mut record, "status", s(*status));
        if let Some(existing) = existing {
            set(
                &mut record,
                "matched_transaction_id",
                s(string(existing, "id")),
            );
            set(
                &mut record,
                "matched_wallet",
                s(string(existing, "wallet_label")),
            );
        }
        let mut changed = false;
        for (bucket, values) in result
            .iter_mut()
            .filter(|(k, _)| ["inserted_records", "updated_records"].contains(k))
        {
            let _ = bucket;
            if let Py::List(values) = values {
                for row in values.iter_mut().filter(|row| {
                    kassiber_envelope::get(row, "transaction_id") == Some(&s(id.as_str()))
                }) {
                    changed = true;
                    set(row, "status", s(*status));
                    set(row, "excluded", Py::Bool(true));
                    set(
                        row,
                        "matched_wallet",
                        existing
                            .as_ref()
                            .map_or(Py::Null, |e| s(string(e, "wallet_label"))),
                    );
                    set(
                        row,
                        "matched_transaction_id",
                        existing.as_ref().map_or(Py::Null, |e| s(string(e, "id"))),
                    );
                }
            }
        }
        if changed {
            set(&mut record, "changed", Py::Bool(true));
        }
        reconciled.push(record);
    }
    invalidate_journals(conn, &request.profile_id)?;
    result.extend([
        ("scope", s("book")),
        ("mode", s("full")),
        (
            "matched",
            Py::from(matches.iter().filter(|(_, s)| *s == "matched").count() as i64),
        ),
        (
            "unmatched",
            Py::from(matches.iter().filter(|(_, s)| *s == "unmatched").count() as i64),
        ),
        (
            "ambiguous",
            Py::from(matches.iter().filter(|(_, s)| *s == "ambiguous").count() as i64),
        ),
        ("excluded", Py::from(excluded)),
        ("reconciliation_records", Py::List(reconciled)),
    ]);
    Ok(())
}
