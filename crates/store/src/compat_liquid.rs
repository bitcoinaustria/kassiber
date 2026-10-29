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
    let mut stmt = tx.prepare(&format!("SELECT t.id,t.profile_id,t.external_id,t.raw_json,w.config_json FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE upper(replace(t.asset,'-',''))='LBTC' OR lower(t.asset) IN ({marks})")).map_err(map_sqlite_error)?;
    let rows = stmt
        .query_map(params_from_iter(&assets), |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        })
        .map_err(map_sqlite_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_sqlite_error)?;
    drop(stmt);
    for (id, profile, external, raw, config) in rows {
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
        let expected = [
            ("txid", txid),
            ("chain", "liquid".into()),
            ("network", network),
            ("asset_id", asset),
            ("asset", "LBTC".into()),
        ];
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
