//! Liquid identity repair, with the reference Decimal representation retained
//! in fingerprints. No current observation is upgraded to ownership evidence.
use super::{
    backfills, data,
    ownership::{json, normalize, text, unsupported},
    python_json as py,
};
use crate::{map_sqlite_error, Result};
use kassiber_pydecimal::{Context, Decimal, DecimalError, Rounding};
use kassiber_pyjson::{Key, Value};
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, str::FromStr};

fn normalized_asset(value: Option<&Value>) -> Result<String> {
    Ok(backfills::asset(&text(value)?))
}
fn decimal(raw: &Value, key: &str, msat: i64) -> Result<String> {
    let scale = Decimal::from_str("100000000000").expect("msat scale");
    if let Some(value) =
        py::get(raw, key).filter(|v| !matches!(v, Value::Null) && *v != &Value::from(""))
    {
        // Decimal(str(value)); booleans and collections are invalid decimals.
        let rendered = match value {
            Value::Str(v) => v.as_str().ok_or_else(unsupported)?.to_owned(),
            Value::Int(v) => v.to_string(),
            Value::Float(v) => kassiber_pyjson::float_repr(*v),
            _ => String::new(),
        };
        let parsed = Decimal::from_str(rendered.trim_matches(backfills::whitespace));
        if matches!(parsed, Err(DecimalError::NonFinite)) {
            return Err(unsupported());
        }
        if let Ok(value) = parsed {
            let value = value.abs(&Context::RP2).map_err(|_| unsupported())?;
            let scaled = value
                .mul(&scale, &Context::RP2)
                .map_err(|_| unsupported())?;
            // quantize is sufficient for i64-bound stored quantities; reject a
            // value outside that context rather than losing decimal digits.
            if let Ok(rounded) = scaled.quantize(0, &Context::new(32, Rounding::HalfUp)) {
                if rounded.to_string().parse::<i64>().ok() == Some(msat) {
                    return Ok(value.to_string());
                }
            }
        }
    }
    Decimal::from_str(&msat.to_string())
        .expect("i64 decimal")
        .div(&scale, &Context::RP2)
        .map(|v| v.to_string())
        .map_err(|_| unsupported())
}
/// Python's `canonical_graph_hash` for an already decoded stored payload.
pub(crate) fn graph_hash(raw: &Value) -> Result<String> {
    Ok(Sha256::digest(py::dumps(raw, false, true)?.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// `_observed_liquid_graph`: the payload without the backfilled identity
/// fields its provenance lacks. Only fields holding exactly the backfilled
/// value are candidates, smallest removal first in `itertools.combinations`
/// order, and only a payload hashing to the committed graph is returned.
pub(crate) fn observed_graph(
    fields: &[(Key, Value)],
    expected: &[(&str, String); 5],
    observed: &str,
) -> Result<Option<Value>> {
    let candidates: Vec<Key> = expected
        .iter()
        .filter(|(key, value)| {
            fields
                .iter()
                .any(|(k, v)| k == &Key::from(*key) && v == &Value::from(value.clone()))
        })
        .map(|(key, _)| Key::from(*key))
        .collect();
    for size in 1..=candidates.len() {
        let mut indices: Vec<usize> = (0..size).collect();
        loop {
            let removed: Vec<&Key> = indices.iter().map(|&i| &candidates[i]).collect();
            let restored = Value::Dict(
                fields
                    .iter()
                    .filter(|(key, _)| !removed.contains(&key))
                    .cloned()
                    .collect(),
            );
            if graph_hash(&restored)? == observed {
                return Ok(Some(restored));
            }
            // Advance to the next lexicographic index combination.
            let Some(slot) = (0..size)
                .rev()
                .find(|&i| indices[i] != i + candidates.len() - size)
            else {
                break;
            };
            indices[slot] += 1;
            for next in slot + 1..size {
                indices[next] = indices[next - 1] + 1;
            }
        }
    }
    Ok(None)
}

/// How the Liquid backfill treats a row carrying observer provenance.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Observed {
    /// The stored graph is the committed one; the backfill leaves it alone.
    Current,
    /// The backfill would restore the committed graph.
    Restorable,
    /// No provenance, or a mismatch no backfilled field explains.
    Other,
}

/// The backfill's provenance decision for one stored row, for read-only
/// verification of books another writer may still have to repair.
pub(crate) fn observed_state(
    raw_json: Option<&str>,
    expected: &[(&str, String); 5],
    observed: Option<&str>,
) -> Result<Observed> {
    let Some(observed) = observed.filter(|v| !v.is_empty()) else {
        return Ok(Observed::Other);
    };
    let raw = json(raw_json)?;
    let Value::Dict(fields) = &raw else {
        return Ok(Observed::Other);
    };
    if graph_hash(&raw)? == observed {
        return Ok(Observed::Current);
    }
    Ok(match observed_graph(fields, expected, observed)? {
        Some(_) => Observed::Restorable,
        None => Observed::Other,
    })
}

pub(crate) fn liquid(
    conn: &mut Connection,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    let assets: BTreeSet<String> = data()["normalization"]["policy_assets"]
        .as_object()
        .expect("policy assets")
        .values()
        .filter_map(|v| v.as_str())
        .filter(|v| !v.is_empty())
        .map(str::to_lowercase)
        .collect();
    if assets.is_empty() {
        return Ok(());
    }
    let marks = vec!["?"; assets.len()].join(",");
    let tx = conn.savepoint().map_err(map_sqlite_error)?;
    // Evaluate the identical asset predicate over distinct assets first. SQLite
    // can use an existing covering index instead of scanning wide observation
    // rows twice. Materialization keeps normalization after DISTINCT.
    let pending: bool = tx.query_row(
        &format!("WITH assets AS MATERIALIZED (SELECT DISTINCT asset FROM transactions) SELECT EXISTS(SELECT 1 FROM assets WHERE upper(replace(asset,'-',''))='LBTC' OR lower(asset) IN ({marks}))"),
        params_from_iter(&assets), |r| r.get(0),
    ).map_err(map_sqlite_error)?;
    if !pending {
        observer("liquid.before_commit")?;
        tx.commit().map_err(map_sqlite_error)?;
        return observer("liquid.after_commit");
    }
    let mut stmt = tx.prepare(&format!("SELECT id,profile_id,wallet_id,external_id,occurred_at,direction,amount,fee,raw_json FROM transactions WHERE lower(asset) IN ({marks})")).map_err(map_sqlite_error)?;
    let rows = stmt
        .query_map(params_from_iter(&assets), |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, Option<i64>>(7)?,
                r.get::<_, Option<String>>(8)?,
            ))
        })
        .map_err(map_sqlite_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_sqlite_error)?;
    drop(stmt);
    let mut profiles = BTreeSet::new();
    for (id, profile, wallet, external, occurred, direction, amount, fee, raw) in rows {
        let raw = json(raw.as_deref())?;
        let fields = Value::Dict(vec![
            (Key::from("wallet_id"), wallet.into()),
            (
                Key::from("external_id"),
                external.unwrap_or_default().into(),
            ),
            (Key::from("occurred_at"), occurred.into()),
            (Key::from("direction"), direction.into()),
            (Key::from("asset"), "LBTC".into()),
            (Key::from("amount"), decimal(&raw, "amount", amount)?.into()),
            (
                Key::from("fee"),
                decimal(&raw, "fee", fee.unwrap_or(0))?.into(),
            ),
        ]);
        let fingerprint: String = Sha256::digest(py::dumps(&fields, true, false)?.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let collision = tx
            .query_row(
                "SELECT 1 FROM transactions WHERE fingerprint=? AND id!=? LIMIT 1",
                params![fingerprint, id],
                |_| Ok(()),
            )
            .optional()
            .map_err(map_sqlite_error)?
            .is_some();
        if collision {
            tx.execute("UPDATE transactions SET asset='LBTC' WHERE id=?", [&id])
                .map_err(map_sqlite_error)?;
        } else {
            tx.execute(
                "UPDATE transactions SET asset='LBTC',fingerprint=? WHERE id=?",
                params![fingerprint, id],
            )
            .map_err(map_sqlite_error)?;
        }
        profiles.insert(profile);
        observer("liquid.after_asset")?;
    }
    let mut stmt = tx.prepare(&format!("SELECT t.id,t.profile_id,t.external_id,t.raw_json,w.config_json,observation.graph_hash FROM transactions t JOIN wallets w ON w.id=t.wallet_id LEFT JOIN chain_observation_provenance observation ON observation.transaction_id=t.id WHERE upper(replace(t.asset,'-',''))='LBTC' OR lower(t.asset) IN ({marks})")).map_err(map_sqlite_error)?;
    let rows = stmt
        .query_map(params_from_iter(&assets), |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
            ))
        })
        .map_err(map_sqlite_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_sqlite_error)?;
    drop(stmt);
    for (id, profile, external, raw, config, observed) in rows {
        let (raw, config) = (json(raw.as_deref())?, json(config.as_deref())?);
        let Value::Dict(mut fields) = raw.clone() else {
            continue;
        };
        if !matches!(config, Value::Dict(_))
            || normalize("chain", py::get(&config, "chain"), "bitcoin")?.as_deref()
                != Some("liquid")
        {
            continue;
        }
        let Some(network) = normalize("liquid_network", py::get(&config, "network"), "liquidv1")?
        else {
            continue;
        };
        let configured_asset = normalized_asset(py::get(&config, "policy_asset"))?;
        let asset = if configured_asset.is_empty() || configured_asset == "LBTC" {
            data()["normalization"]["policy_assets"][&network]
                .as_str()
                .expect("policy asset")
                .to_owned()
        } else {
            configured_asset
        };
        if asset.len() != 64 {
            continue;
        }
        let txid = text(py::get(&raw, "txid"))?;
        let txid = if txid.is_empty() {
            external.unwrap_or_default()
        } else {
            txid
        };
        let txid = txid.trim_matches(backfills::whitespace).to_lowercase();
        if !crate::valid_hex(&txid, 64) {
            continue;
        }
        let expected = [
            ("txid", txid),
            ("chain", "liquid".to_owned()),
            ("network", network.clone()),
            ("asset_id", asset.clone()),
            ("asset", "LBTC".to_owned()),
        ];
        if let Some(observed) = observed.filter(|v| !v.is_empty()) {
            // A current observer graph is not a legacy observation: its
            // provenance commits to the exact payload, so filling identity
            // fields here would silently revoke its authority. Rows an earlier
            // run of this backfill already rewrote are restored when the
            // committed hash proves which added fields to drop.
            if graph_hash(&raw)? == observed {
                continue;
            }
            if let Some(restored) = observed_graph(&fields, &expected, &observed)? {
                tx.execute(
                    "UPDATE transactions SET raw_json=? WHERE id=?",
                    params![py::dumps(&restored, true, false)?, id],
                )
                .map_err(map_sqlite_error)?;
                profiles.insert(profile);
                observer("liquid.after_identity")?;
                continue;
            }
        }
        let old_chain = text(py::get(&raw, "chain"))?;
        let old_network = text(py::get(&raw, "network"))?;
        let old_asset_id = normalized_asset(py::get(&raw, "asset_id"))?;
        let old_asset = normalized_asset(py::get(&raw, "asset"))?;
        if (!old_chain.trim_matches(backfills::whitespace).is_empty()
            && normalize("chain", py::get(&raw, "chain"), "bitcoin")?.as_deref() != Some("liquid"))
            || (!old_network.trim_matches(backfills::whitespace).is_empty()
                && normalize("liquid_network", py::get(&raw, "network"), "liquidv1")?.as_deref()
                    != Some(&network))
            || (!old_asset_id.is_empty() && old_asset_id != asset)
            || (!old_asset.is_empty() && old_asset != "LBTC" && old_asset != asset)
        {
            continue;
        }
        let mut changed = false;
        for (key, value) in expected {
            let value = Value::from(value);
            if let Some((_, old)) = fields.iter_mut().find(|(k, _)| k == &Key::from(key)) {
                if old != &value {
                    changed = true;
                    *old = value;
                }
            } else {
                changed = true;
                fields.push((Key::from(key), value));
            }
        }
        if !changed {
            continue;
        }
        tx.execute(
            "UPDATE transactions SET raw_json=? WHERE id=?",
            params![py::dumps(&Value::Dict(fields), true, false)?, id],
        )
        .map_err(map_sqlite_error)?;
        profiles.insert(profile);
        observer("liquid.after_identity")?;
    }
    for profile in profiles {
        tx.execute("UPDATE profiles SET last_processed_at=NULL,last_processed_tx_count=0,ownership_review_counts_json=NULL,journal_input_version=journal_input_version+1 WHERE id=?",[profile]).map_err(map_sqlite_error)?;
    }
    observer("liquid.before_commit")?;
    tx.commit().map_err(map_sqlite_error)?;
    observer("liquid.after_commit")
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    const TXID: &str = "2222222222222222222222222222222222222222222222222222222222222222";

    /// A Bull Bitcoin Liquid import as the native importer writes it: the
    /// wallet names its chain, the row carries no Liquid identity yet.
    fn seed(conn: &Connection, raw: &str) {
        conn.execute_batch(&format!(
            "INSERT INTO workspaces(id,label,created_at) VALUES('w','Main','2026');
             INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('p','w','Book','2026');
             INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at)
               VALUES('l','w','p','L','bullbitcoin','{{\"chain\": \"liquid\", \"bullbitcoin_wallet_network\": \"liquid\"}}','2026');
             INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,occurred_at,direction,asset,amount,fee,raw_json,created_at)
               VALUES('t','w','p','l','fp','{TXID}','2026-01-03T10:00:00Z','outbound','LBTC',500000,1000,'{raw}','2026');"
        ))
        .unwrap();
    }

    #[test]
    fn verify_refuses_pending_liquid_identity() {
        let root = tempfile::tempdir().unwrap();
        let book = crate::Book::create(root.path(), None).unwrap();
        seed(
            &book.connection,
            &format!("{{\"network\": \"\", \"txid\": \"{TXID}\"}}"),
        );
        let error = crate::schema::verify(&book.connection).unwrap_err();
        assert_eq!(
            error.kind,
            crate::ErrorKind::RequiresPythonMigration,
            "{error:?}"
        );
    }

    const POLICY: &str = "b2e15d0d7a0c94e4e2ce0fe6e8691b9e451377f6e46e8045a86f7c4b5d4f0f23";

    /// What current Liquid sync stores: identity only inside `component`.
    fn synced() -> String {
        format!(
            "{{\"chain\": \"liquid\", \"component\": {{\"asset\": \"LBTC\", \"asset_id\": \"{POLICY}\", \"fee_sats\": 0, \"net_sats\": 5000}}, \"network\": \"elementsregtest\", \"ownership_graph_version\": 1, \"txid\": \"{TXID}\", \"vin\": [], \"vout\": []}}"
        )
    }

    fn graph(raw: &str) -> String {
        super::graph_hash(&super::json(Some(raw)).unwrap()).unwrap()
    }

    /// A synced Liquid row whose observer committed to the `committed` graph.
    fn observed(conn: &Connection, raw: &str, committed: &str) {
        conn.execute_batch(&format!(
            "INSERT INTO workspaces(id,label,created_at) VALUES('w','Main','2026');
             INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('p','w','Book','2026');
             INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at)
               VALUES('l','w','p','L','descriptor','{{\"chain\": \"liquid\", \"network\": \"elementsregtest\", \"policy_asset\": \"{POLICY}\"}}','2026');
             INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,occurred_at,direction,asset,amount,fee,raw_json,created_at)
               VALUES('t','w','p','l','fp','{TXID}','2026-01-03T10:00:00Z','inbound','LBTC',5000000,0,'{raw}','2026');
             INSERT INTO chain_observation_provenance(transaction_id,workspace_id,profile_id,wallet_id,authority_version,observer_ids_json,observer_kinds_json,chain,network,application_revision,graph_hash,quantity_hash,fee_attribution,observed_at,updated_at)
               VALUES('t','w','p','l',1,'[]','[\"electrum\"]','liquid','elementsregtest','r','{}','q','unknown','2026','2026');",
            graph(committed)
        ))
        .unwrap();
    }

    fn stored(conn: &Connection) -> String {
        conn.query_row("SELECT raw_json FROM transactions WHERE id='t'", [], |r| {
            r.get(0)
        })
        .unwrap()
    }

    fn input_version(conn: &Connection) -> i64 {
        conn.query_row(
            "SELECT journal_input_version FROM profiles WHERE id='p'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    }

    fn backfill(conn: &mut Connection) {
        super::liquid(conn, &mut |_| Ok(())).unwrap();
    }

    #[test]
    fn committed_liquid_observation_keeps_its_graph() {
        let root = tempfile::tempdir().unwrap();
        let mut book = crate::Book::create(root.path(), None).unwrap();
        observed(&book.connection, &synced(), &synced());
        crate::schema::verify(&book.connection).unwrap();
        let version = input_version(&book.connection);
        backfill(&mut book.connection);
        assert_eq!(stored(&book.connection), synced());
        assert_eq!(input_version(&book.connection), version);
        crate::schema::verify(&book.connection).unwrap();
    }

    #[test]
    fn rewritten_liquid_observation_is_restored_to_its_committed_graph() {
        let root = tempfile::tempdir().unwrap();
        let mut book = crate::Book::create(root.path(), None).unwrap();
        let rewritten = synced().replacen(
            "{\"chain\"",
            &format!("{{\"asset\": \"LBTC\", \"asset_id\": \"{POLICY}\", \"chain\""),
            1,
        );
        observed(&book.connection, &rewritten, &synced());
        let error = crate::schema::verify(&book.connection).unwrap_err();
        assert_eq!(error.kind, crate::ErrorKind::RequiresPythonMigration);
        let version = input_version(&book.connection);
        backfill(&mut book.connection);
        assert_eq!(graph(&stored(&book.connection)), graph(&synced()));
        assert_eq!(input_version(&book.connection), version + 1);
        crate::schema::verify(&book.connection).unwrap();
    }

    #[test]
    fn unexplained_liquid_graph_change_keeps_the_legacy_fill() {
        let root = tempfile::tempdir().unwrap();
        let mut book = crate::Book::create(root.path(), None).unwrap();
        let forged = synced().replace("\"vin\": []", "\"vin\": [{\"forged\": true}]");
        observed(&book.connection, &forged, &synced());
        backfill(&mut book.connection);
        let after = stored(&book.connection);
        assert!(
            after.contains("\"forged\": true") && after.contains("\"asset_id\""),
            "{after}"
        );
        crate::schema::verify(&book.connection).unwrap();
    }
}
