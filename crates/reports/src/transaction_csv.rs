//! The report's observed transaction sheet; journal facts are joined explicitly.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

pub(super) const HEADERS: &[&str] = &[
    "occurred_at",
    "wallet",
    "transaction_id",
    "direction",
    "asset",
    "amount",
    "amount_msat",
    "fee",
    "fee_msat",
    "fiat_currency",
    "fiat_price",
    "fiat_value",
    "fmv",
    "fiat_fee",
    "cost_basis",
    "gain_loss",
    "description",
    "note",
    "counterparty",
    "transfer",
    "tags",
    "references",
    "attachments",
];

pub(super) fn optional(row: &Value, exact: &str, legacy: &str) -> Result<Option<Decimal>> {
    let raw = value(row, exact);
    let raw = if matches!(raw, Value::Null) || raw == &s("") {
        value(row, legacy)
    } else {
        raw
    };
    if matches!(raw, Value::Null) || raw == &s("") {
        Ok(None)
    } else {
        match kassiber_pydecimal::finite_price(&py_str(raw)) {
            Ok(value) => Ok(Some(value)),
            Err(kassiber_pydecimal::PriceError::Invalid) => decimal(raw).map(Some),
            Err(_) => Ok(None),
        }
    }
}
fn optional_float(v: Option<&Decimal>) -> Result<Value> {
    v.map(float).transpose().map(|v| v.unwrap_or(Value::Null))
}

fn transfer_labels(conn: &Connection, profile: &str) -> Result<BTreeMap<String, BTreeSet<String>>> {
    let mut labels: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let entries = rows(conn, "SELECT je.transaction_id AS tx_id,je.entry_type,je.occurred_at,COALESCE(je.description,'') AS description,je.asset,ABS(je.quantity) AS quantity_msat,w.label AS wallet_label FROM journal_entries je JOIN wallets w ON w.id=je.wallet_id WHERE je.profile_id=? AND je.entry_type IN ('transfer_out','transfer_in')", [profile])?;
    let mut groups: BTreeMap<(String, String, String), Vec<Value>> = BTreeMap::new();
    for row in entries {
        groups
            .entry((
                text(&row, "occurred_at"),
                text(&row, "description"),
                text(&row, "asset"),
            ))
            .or_default()
            .push(row);
    }
    let mut link = |out: &Value, incoming: &Value| {
        labels
            .entry(text(out, "tx_id"))
            .or_default()
            .insert(text(incoming, "wallet_label"));
        labels
            .entry(text(incoming, "tx_id"))
            .or_default()
            .insert(text(out, "wallet_label"));
    };
    for group in groups.values() {
        let mut outs: Vec<_> = group
            .iter()
            .filter(|r| text(r, "entry_type") == "transfer_out")
            .collect();
        let mut ins: Vec<_> = group
            .iter()
            .filter(|r| text(r, "entry_type") == "transfer_in")
            .collect();
        let key = |r: &&Value| (int(r, "quantity_msat"), text(r, "tx_id"));
        outs.sort_by_key(key);
        ins.sort_by_key(key);
        let mut fee_bearing = Vec::new();
        for out in outs {
            if let Some(index) = ins
                .iter()
                .position(|r| int(r, "quantity_msat") == int(out, "quantity_msat"))
            {
                link(out, ins.remove(index));
            } else {
                fee_bearing.push(out);
            }
        }
        for out in fee_bearing {
            if let Some((_, index)) = ins
                .iter()
                .enumerate()
                .filter_map(|(i, r)| {
                    let delta = int(out, "quantity_msat") - int(r, "quantity_msat");
                    (delta >= 0).then_some((delta, i))
                })
                .min()
            {
                link(out, ins.remove(index));
            }
        }
    }
    let conversions = rows(conn,"SELECT tout.id AS source_id,tin.id AS target_id,wout.label AS source_label,win.label AS target_label FROM journal_custody_projection_relations p JOIN transactions tout ON tout.id=p.out_transaction_id JOIN transactions tin ON tin.id=p.in_transaction_id JOIN wallets wout ON wout.id=tout.wallet_id JOIN wallets win ON win.id=tin.wallet_id WHERE p.profile_id=? AND p.relation_kind='conversion' AND p.policy='carrying-value' AND p.basis_state='eligible' AND tout.excluded=0 AND tin.excluded=0 AND tout.asset!=tin.asset AND NOT EXISTS(SELECT 1 FROM journal_quarantines q WHERE q.profile_id=p.profile_id AND q.transaction_id IN (tout.id,tin.id))",[profile])?;
    for row in conversions {
        labels
            .entry(text(&row, "source_id"))
            .or_default()
            .insert(text(&row, "target_label"));
        labels
            .entry(text(&row, "target_id"))
            .or_default()
            .insert(text(&row, "source_label"));
    }
    Ok(labels)
}

pub(super) fn report_rows(
    conn: &Connection,
    profile: &Value,
    wallet: Option<&str>,
) -> Result<Vec<Value>> {
    report_rows_current(conn, profile, wallet, true)
}

pub(super) fn report_rows_current(
    conn: &Connection,
    profile: &Value,
    wallet: Option<&str>,
    current: bool,
) -> Result<Vec<Value>> {
    let id = text(profile, "id");
    let mut transactions = rows(conn,"SELECT t.*,w.label AS wallet,COALESCE(t.external_id,'') AS transaction_id FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE t.profile_id=? AND t.excluded=0 ORDER BY t.occurred_at,t.created_at,t.id",[&id])?;
    if let Some(wallet) = wallet {
        transactions.retain(|r| text(r, "wallet_id") == wallet);
    }
    let basis = if current {
        rows(conn,"SELECT je.transaction_id AS tx_id,SUM(COALESCE(je.cost_basis,0)) AS cost_basis,SUM(COALESCE(je.gain_loss,0)) AS gain_loss,SUM(CASE WHEN je.cost_basis IS NOT NULL THEN 1 ELSE 0 END) AS basis_entries,SUM(CASE WHEN je.gain_loss IS NOT NULL THEN 1 ELSE 0 END) AS gain_entries FROM journal_entries je LEFT JOIN transactions t ON t.id=je.transaction_id WHERE je.profile_id=? AND (?2 IS NULL OR je.wallet_id=?2) AND je.entry_type IN ('disposal','income') AND COALESCE(t.taxability_override,1)!=0 AND COALESCE(je.at_category,'')!='neu_swap' GROUP BY je.transaction_id",kassiber_store::rusqlite::params![id,wallet])?
    } else {
        Vec::new()
    };
    let basis: BTreeMap<_, _> = basis.into_iter().map(|r| (text(&r, "tx_id"), r)).collect();
    let tags = rows(conn,"SELECT tt.transaction_id AS tx_id,tg.code FROM transaction_tags tt JOIN tags tg ON tg.id=tt.tag_id JOIN transactions t ON t.id=tt.transaction_id WHERE t.profile_id=? AND t.excluded=0 ORDER BY tg.code",[&id])?;
    let mut tag_map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in tags {
        tag_map
            .entry(text(&row, "tx_id"))
            .or_default()
            .push(text(&row, "code"));
    }
    let attachments = rows(conn,"SELECT a.transaction_id AS tx_id,a.attachment_type AS kind,COALESCE(a.label,'') AS label,COALESCE(a.original_filename,'') AS filename,COALESCE(a.source_url,'') AS url FROM attachments a JOIN transactions t ON t.id=a.transaction_id WHERE t.profile_id=? AND t.excluded=0 ORDER BY a.created_at",[&id])?;
    let mut attachment_map: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for row in attachments {
        let label = text(&row, "label");
        let label = kassiber_pytext::strip(&label);
        let url = text(&row, "url");
        let url = kassiber_pytext::strip(&url);
        let filename = text(&row, "filename");
        let filename = kassiber_pytext::strip(&filename);
        let is_url = text(&row, "kind").to_lowercase() == "url";
        let display = if is_url {
            if !label.is_empty() {
                label
            } else if !url.is_empty() {
                url
            } else {
                "(link)"
            }
        } else if !filename.is_empty() {
            filename
        } else if !label.is_empty() {
            label
        } else {
            "(file)"
        };
        attachment_map
            .entry(text(&row, "tx_id"))
            .or_default()
            .push((
                display.into(),
                if is_url { url.into() } else { String::new() },
            ));
    }
    let transfers = if current {
        transfer_labels(conn, &id)?
    } else {
        BTreeMap::new()
    };
    transactions
        .into_iter()
        .map(|row| {
            let amount = int(&row, "amount");
            let fee = int(&row, "fee");
            let amount_btc = btc(amount)?;
            let fee_btc = btc(fee)?;
            let cash = optional(&row, "fiat_value_exact", "fiat_value")?;
            let mut price = optional(&row, "fiat_rate_exact", "fiat_rate")?;
            if price.is_none() {
                if let Some(cash) = &cash {
                    let denominator = if amount != 0 {
                        Some(&amount_btc)
                    } else if fee != 0 {
                        Some(&fee_btc)
                    } else {
                        None
                    };
                    if let Some(denominator) = denominator {
                        price = Some(cash.div(denominator, &DECIMAL).map_err(arithmetic)?);
                    }
                }
            }
            let fmv = price
                .as_ref()
                .map(|p| amount_btc.mul(p, &DECIMAL).map_err(arithmetic))
                .transpose()?
                .or_else(|| cash.clone());
            let fiat_fee = price
                .as_ref()
                .map(|p| fee_btc.mul(p, &DECIMAL).map_err(arithmetic))
                .transpose()?;
            let mut result = dict(HEADERS.iter().map(|h| (*h, value(&row, h).clone())));
            for key in ["description", "note", "counterparty", "fiat_currency"] {
                if !truthy(value(&result, key)) {
                    replace(&mut result, key, s(""));
                }
            }
            for (key, v) in [
                ("amount", float(&amount_btc)?),
                ("amount_msat", Value::Int(amount.into())),
                ("fee", float(&fee_btc)?),
                ("fee_msat", Value::Int(fee.into())),
                ("fiat_price", optional_float(price.as_ref())?),
                ("fiat_value", optional_float(cash.as_ref())?),
                ("fmv", optional_float(fmv.as_ref())?),
                ("fiat_fee", optional_float(fiat_fee.as_ref())?),
            ] {
                replace(&mut result, key, v);
            }
            // The machine CSV writes these exactly.
            let exact = |d: Option<&Decimal>| {
                d.map(|d| s(crate::report_design::plain(d)))
                    .unwrap_or(Value::Null)
            };
            for (key, v) in [
                (
                    "amount_exact",
                    s(crate::report_design::btc_text(&amount_btc)),
                ),
                ("fee_exact", s(crate::report_design::btc_text(&fee_btc))),
                ("fiat_price_exact", exact(price.as_ref())),
                ("fiat_value_exact", exact(cash.as_ref())),
                ("fmv_exact", exact(fmv.as_ref())),
                ("fiat_fee_exact", exact(fiat_fee.as_ref())),
            ] {
                replace(&mut result, key, v);
            }
            let tx = text(&row, "id");
            let journal = basis.get(&tx).unwrap_or(&Value::Null);
            for (key, count) in [
                ("cost_basis", "basis_entries"),
                ("gain_loss", "gain_entries"),
            ] {
                let total = if int(journal, count) > 0 {
                    Some(decimal(value(journal, key))?)
                } else {
                    None
                };
                replace(
                    &mut result,
                    key,
                    match &total {
                        Some(d) => float(d)?,
                        None => Value::Null,
                    },
                );
                replace(&mut result, &format!("{key}_exact"), exact(total.as_ref()));
            }
            replace(
                &mut result,
                "transfer",
                s(transfers
                    .get(&tx)
                    .map(|v| v.iter().cloned().collect::<Vec<_>>().join(", "))
                    .unwrap_or_default()),
            );
            replace(
                &mut result,
                "tags",
                s(tag_map.get(&tx).map(|v| v.join(", ")).unwrap_or_default()),
            );
            let attachments = attachment_map
                .get(&tx)
                .map(Vec::as_slice)
                .unwrap_or_default();
            replace(
                &mut result,
                "references",
                s(attachments
                    .iter()
                    .filter(|(_, url)| !url.is_empty())
                    .map(|(_, url)| url.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")),
            );
            replace(
                &mut result,
                "attachments",
                s(attachments
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")),
            );
            // Rendering needs link/name pairing; the CSV header projection
            // continues to emit only the existing flattened text fields.
            replace(
                &mut result,
                "attachments_list",
                Value::List(
                    attachments
                        .iter()
                        .map(|(name, url)| {
                            dict([("display_name", s(name.as_str())), ("url", s(url.as_str()))])
                        })
                        .collect(),
                ),
            );
            Ok(result)
        })
        .collect()
}
