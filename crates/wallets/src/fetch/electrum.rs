//! Python's compatibility Electrum observer for Bitcoin
//! (`compatibility_electrum_sync_adapter`): records from the changed histories
//! a cached acquisition returned, and the outputs it listed. Pure: the
//! acquisition made every request.
use super::*;
use crate::bitcoin_records::{self, UNKNOWN};

/// Microseconds from 0001-01-01 to the Unix epoch, `kassiber_chain`'s base.
const UNIX_US: i64 = 62_135_596_800_000_000;

fn invalid() -> AppError {
    AppError::new("protocol_error", "Invalid Electrum observation")
}

/// `_positive_electrum_height`.
pub(super) fn positive_height(value: &Value) -> Option<i64> {
    crate::scan::integer(value)
        .and_then(|n| i64::try_from(n).ok())
        .filter(|h| *h > 0)
}

/// `timestamp_to_iso(value, default=None)` for a header timestamp.
pub(super) fn header_time(value: &Value) -> Result<Option<String>> {
    bitcoin_records::timestamp(value)
}

/// `_history_explicit_time_iso`: a history row's own time, when it has one.
pub(super) fn explicit_time(history: &Value) -> Option<String> {
    for key in ["confirmed_at", "timestamp", "time", "blocktime", "height"] {
        let value = &history[key];
        if value.is_null() || value.as_str() == Some("") {
            continue;
        }
        // `parse_iso_datetime_or_none`, truncated to whole seconds.
        if let Some(us) = kassiber_chain::parse_iso_time(&descriptors::text(value)) {
            return kassiber_chain::format_unix_time((us - UNIX_US).div_euclid(1_000_000));
        }
        let zero =
            value.as_str() == Some("0") || value.as_f64() == Some(0.0) || value == &json!(false);
        if key != "height" && !zero {
            if let Some(time) = crate::scan::integer(value)
                .and_then(|n| i64::try_from(n).ok())
                .and_then(kassiber_chain::format_unix_time)
            {
                return Some(time);
            }
        }
    }
    None
}

/// `_history_sort_key`: confirmed rows by height, then the rest by their
/// explicit time; the txid breaks ties.
pub(super) fn sort_key(txid: &str, history: &Value) -> (u8, i64, String, String) {
    match positive_height(&history["height"]) {
        Some(height) => (0, height, String::new(), txid.into()),
        None => (
            1,
            0,
            explicit_time(history).unwrap_or_else(|| UNKNOWN.into()),
            txid.into(),
        ),
    }
}

/// The record loop of `compatibility_electrum_records_for_wallet`: every
/// changed history in Python's order, dated by its block header.
pub(super) fn records(
    acquired: &Value,
    tracked: &BTreeSet<String>,
    backend: &str,
    deadline: Instant,
    control: &dyn Control,
) -> Result<Vec<Value>> {
    let histories = acquired["histories"].as_object().ok_or_else(invalid)?;
    let headers = &acquired["freshness_checkpoint"]["electrum_headers"];
    let mut ordered: Vec<(&String, &Value)> = histories.iter().collect();
    ordered.sort_by_cached_key(|(txid, history)| sort_key(txid, history));
    let total = ordered.len();
    let mut records = vec![];
    for (index, (txid, history)) in ordered.into_iter().enumerate() {
        control.check(deadline)?;
        // `_history_occurred_at`.
        let occurred = match positive_height(&history["height"]) {
            Some(height) => {
                header_time(&headers[height.to_string()])?.unwrap_or_else(|| UNKNOWN.into())
            }
            None => explicit_time(history).unwrap_or_else(|| UNKNOWN.into()),
        };
        let tx = acquired["transactions"]
            .get(txid.as_str())
            .ok_or_else(invalid)?;
        if let Some(record) = bitcoin_records::from_electrum(txid, tx, &occurred, tracked, backend)?
        {
            records.push(record);
        }
        if (index + 1).is_multiple_of(100) || index + 1 == total {
            control.progress(json!({"phase":"decode_enrich","transactions_seen":index+1,
                "transactions_total":total,"records":records.len(),"processed":index+1,"total":total}))?;
        }
    }
    Ok(records)
}

/// `_target_output_metadata`.
pub(super) fn target_metadata(target: &Value) -> serde_json::Map<String, Value> {
    let label = if descriptors::truthy(&target["branch_label"]) {
        target["branch_label"].clone()
    } else {
        json!("address")
    };
    let index = target.get("address_index").cloned().unwrap_or(Value::Null);
    let address_label = if index.is_null() {
        descriptors::text(&label)
    } else {
        format!(
            "{} #{}",
            descriptors::text(&label),
            descriptors::text(&index)
        )
    };
    let address = ["address", "unconfidential_address"]
        .iter()
        .map(|key| &target[*key])
        .find(|v| descriptors::truthy(v))
        .cloned()
        .unwrap_or(json!(""));
    let script = if descriptors::truthy(&target["script_pubkey"]) {
        target["script_pubkey"].clone()
    } else {
        json!("")
    };
    let mut fields = serde_json::Map::new();
    fields.insert("address".into(), address);
    fields.insert("script_pubkey".into(), script);
    fields.insert("address_label".into(), json!(address_label));
    fields.insert("branch_label".into(), label);
    fields.insert(
        "branch_index".into(),
        target.get("branch_index").cloned().unwrap_or(Value::Null),
    );
    fields.insert("address_index".into(), index);
    fields
}

/// `_electrum_bitcoin_utxo_record` for every listed output, in target order:
/// depth from the second connection's tip, time from the output's header.
pub(super) fn outputs(acquired: &Value, chain: &str, network: &str) -> Result<Vec<Value>> {
    let tip = acquired["tip_height"].as_i64();
    let headers = &acquired["utxo_headers"];
    let scripts = acquired["scripts"].as_array().ok_or_else(invalid)?;
    let mut outputs = vec![];
    for script in scripts {
        let listed = match &script["utxos"] {
            Value::Null => continue,
            Value::Array(listed) => listed,
            _ => return Err(invalid()),
        };
        for raw in listed {
            if !raw.is_object() {
                return Err(invalid());
            }
            // `_electrum_utxo_status`.
            let height = positive_height(&raw["height"]);
            let (confirmed, block_time) = match height {
                Some(height) => (true, header_time(&headers[height.to_string()])?),
                None => {
                    let time = explicit_time(raw);
                    (time.is_some(), time)
                }
            };
            let amount = if descriptors::truthy(&raw["value"]) {
                crate::scan::integer(&raw["value"]).ok_or_else(invalid)?
            } else {
                0.into()
            };
            let confirmations = height
                .zip(tip)
                .filter(|(h, t)| *t >= *h)
                .map(|(h, t)| t - h + 1);
            let mut output = json!({"txid":raw["tx_hash"],"vout":raw["tx_pos"],"asset":"BTC",
                "amount_sats":serde_json::from_str::<Value>(&amount.to_string()).map_err(|_| invalid())?,
                "confirmation_status":if confirmed {"confirmed"} else {"mempool"},
                "confirmations":confirmations,"block_height":height,"block_time":block_time,
                "chain":chain,"network":network});
            let fields = output.as_object_mut().expect("object");
            fields.extend(target_metadata(&script["target"]));
            fields.insert(
                "raw".into(),
                json!({"source":"electrum_scripthash_listunspent","confirmed":confirmed}),
            );
            outputs.push(output);
        }
    }
    Ok(outputs)
}

/// The adapter metadata Python's compatibility Electrum fetch returns.
pub(super) fn metadata(
    acquired: &Value,
    route: &str,
    reason: &str,
    outputs: &dyn Fn() -> Result<Vec<Value>>,
) -> Result<Value> {
    let checkpoint = &acquired["freshness_checkpoint"];
    let known: BTreeSet<&str> = checkpoint["electrum_history_entries"]
        .as_object()
        .into_iter()
        .flat_map(|entries| entries.values())
        .filter_map(Value::as_object)
        .flat_map(|entries| entries.keys().map(String::as_str))
        .collect();
    let mut meta = json!({"freshness_checkpoint":checkpoint,"scripts_changed":acquired["scripts_changed"],
        "scripts_unchanged":acquired["scripts_unchanged"],"known_txids":known.len(),
        "header_cache_hits":acquired["header_cache_hits"],"transactions_fetched":acquired["transactions_fetched"],
        "observer_route":route,"observer_compatibility_reason":reason});
    if acquired["utxos_skipped_unchanged"] == true {
        meta["utxos_skipped_unchanged"] = json!(true);
    } else {
        meta["utxos"] = json!(outputs()?);
    }
    Ok(meta)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn electrum_explicit_times_follow_python_coercions() {
        let cases = [
            (json!({"height":0}), None),
            (
                json!({"height":-1,"timestamp":0,"time":"0","blocktime":false}),
                None,
            ),
            (
                json!({"timestamp":"2024-01-02T03:04:05.987+01:00"}),
                Some("2024-01-02T02:04:05Z"),
            ),
            (
                json!({"timestamp":"bad","blocktime":1700000000}),
                Some("2023-11-14T22:13:20Z"),
            ),
            (
                json!({"confirmed_at":"1700000000"}),
                Some("2023-11-14T22:13:20Z"),
            ),
            (json!({"time":1700000000.9}), Some("2023-11-14T22:13:20Z")),
            (json!({"height":"2024-01-01"}), Some("2024-01-01T00:00:00Z")),
            (json!({"height":1700000000}), None),
            (json!({"time":"1.5"}), None),
            (json!({"time":true}), Some("1970-01-01T00:00:01Z")),
        ];
        for (history, expected) in cases {
            assert_eq!(explicit_time(&history).as_deref(), expected, "{history}");
        }
    }

    #[test]
    fn electrum_histories_sort_confirmed_by_height_then_unconfirmed_by_time() {
        let mut rows = [
            ("d", json!({"height":0})),
            ("c", json!({"height":-1,"time":1})),
            ("b", json!({"height":7})),
            ("a", json!({"height":12})),
            ("e", json!({"height":7})),
        ];
        rows.sort_by_cached_key(|(txid, history)| sort_key(txid, history));
        assert_eq!(rows.iter().map(|(t, _)| *t).collect::<String>(), "beadc");
    }

    #[test]
    fn electrum_outputs_match_python_utxo_rows() {
        let acquired = json!({"tip_height":12,"utxo_headers":{"10":1700000000},"scripts":[
            {"target":{"address":"","unconfidential_address":"bc1-u","script_pubkey":"51","branch_index":0,"address_index":2},
             "utxos":[{"tx_hash":"a","tx_pos":1,"value":2100000000000000u64,"height":10},
                      {"tx_hash":"b","tx_pos":0,"height":0},
                      {"tx_hash":"c","tx_pos":2,"value":"5","height":-1,"time":1700000000}]},
            {"target":{"address":"bc1","script_pubkey":"52","branch_label":"change","branch_index":1},"utxos":null}]});
        assert_eq!(
            outputs(&acquired, "bitcoin", "main").unwrap(),
            vec![
                json!({"txid":"a","vout":1,"asset":"BTC","amount_sats":2100000000000000u64,
                "confirmation_status":"confirmed","confirmations":3,"block_height":10,
                "block_time":"2023-11-14T22:13:20Z","chain":"bitcoin","network":"main","address":"bc1-u",
                "script_pubkey":"51","address_label":"address #2","branch_label":"address","branch_index":0,
                "address_index":2,"raw":{"source":"electrum_scripthash_listunspent","confirmed":true}}),
                json!({"txid":"b","vout":0,"asset":"BTC","amount_sats":0,"confirmation_status":"mempool",
                "confirmations":null,"block_height":null,"block_time":null,"chain":"bitcoin","network":"main",
                "address":"bc1-u","script_pubkey":"51","address_label":"address #2","branch_label":"address",
                "branch_index":0,"address_index":2,"raw":{"source":"electrum_scripthash_listunspent","confirmed":false}}),
                // A history-style explicit time confirms without a height.
                json!({"txid":"c","vout":2,"asset":"BTC","amount_sats":5,"confirmation_status":"confirmed",
                "confirmations":null,"block_height":null,"block_time":"2023-11-14T22:13:20Z","chain":"bitcoin",
                "network":"main","address":"bc1-u","script_pubkey":"51","address_label":"address #2",
                "branch_label":"address","branch_index":0,"address_index":2,
                "raw":{"source":"electrum_scripthash_listunspent","confirmed":true}}),
            ]
        );
        let mut skipped = acquired.clone();
        skipped["utxos_skipped_unchanged"] = json!(true);
        skipped["freshness_checkpoint"] =
            json!({"electrum_history_entries":{"h1":{"a":{},"b":{}},"h2":{"a":{}}}});
        let meta = metadata(&skipped, "bitcoin_script", "address_list", &|| {
            outputs(&skipped, "bitcoin", "main")
        })
        .unwrap();
        assert_eq!(meta["known_txids"], 2);
        assert_eq!(meta["utxos_skipped_unchanged"], true);
        assert!(meta.get("utxos").is_none());
        let mut bad = acquired;
        bad["scripts"][0]["utxos"] = json!({"a":1});
        assert_eq!(
            outputs(&bad, "bitcoin", "main").unwrap_err().code,
            "protocol_error"
        );
    }
}
