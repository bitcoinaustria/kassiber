//! Append resolutions only after the journal's existing report barriers clear.
//! Summary arithmetic and chain authority reuse the already ported services.
use super::*;
use bitcoin_hashes::{sha1, Hash};
use kassiber_custody::codec;
use kassiber_store::rusqlite::types::ValueRef;

fn error(e: impl std::fmt::Display) -> Refusal {
    Refusal::new("RuntimeError", e.to_string())
}
fn rows(c: &Connection, sql: &str, params: impl rusqlite::Params) -> Result<Vec<Value>, Refusal> {
    let mut statement = c.prepare(sql)?;
    let names = statement
        .column_names()
        .iter()
        .map(|s| (*s).to_owned())
        .collect::<Vec<_>>();
    let result = statement
        .query_map(params, |row| {
            let mut out = serde_json::Map::new();
            for (i, name) in names.iter().enumerate() {
                let v = match row.get_ref(i)? {
                    ValueRef::Null => Value::Null,
                    ValueRef::Integer(n) => json!(n),
                    ValueRef::Real(f) => json!(f),
                    ValueRef::Text(s) => {
                        json!(std::str::from_utf8(s).map_err(|_| rusqlite::Error::InvalidQuery)?)
                    }
                    ValueRef::Blob(_) => return Err(rusqlite::Error::InvalidQuery),
                };
                out.insert(name.clone(), v);
            }
            Ok(Value::Object(out))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(result)
}
fn object(v: &Value) -> Value {
    v.as_str()
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}))
}
fn summaries(
    c: &Connection,
    pid: &str,
    start: &Value,
    end: &Value,
    scope: &Value,
) -> Result<(Value, Value), Refusal> {
    let scope = codec::from_json(scope)?;
    let (class, gain) = kassiber_reports::export_completion::summaries(
        c,
        pid,
        start.as_i64().ok_or_else(|| error("invalid report year"))?,
        end.as_i64().ok_or_else(|| error("invalid report year"))?,
        &scope,
    )
    .map_err(error)?;
    Ok((codec::to_json(&class), codec::to_json(&gain)))
}
fn compare(
    before_class: Value,
    before_gain: Value,
    state: &Value,
    after_class: Value,
    after_gain: Value,
) -> Value {
    let cc = before_class != after_class;
    let gc = before_gain != after_gain;
    json!({"after_classification_summary":after_class,"after_gain_summary":after_gain,
        "classification_changed":cc,"gain_changed":gc,"amendment_status":if !cc&&!gc{"no_change"}else if state=="filed"{"review_required"}else{"saved_report_changed"}})
}
fn uuid5(impact: &str) -> String {
    let mut preimage = vec![
        0x6b, 0xa7, 0xb8, 0x11, 0x9d, 0xad, 0x11, 0xd1, 0x80, 0xb4, 0x00, 0xc0, 0x4f, 0xd4, 0x30,
        0xc8,
    ];
    preimage.extend(format!("kassiber:custody-filed-impact-resolution:{impact}").as_bytes());
    let mut h = sha1::Hash::hash(&preimage).to_byte_array();
    h[6] = (h[6] & 15) | 80;
    h[8] = (h[8] & 63) | 128;
    let h = h[..16]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    )
}
fn chain_ready(latest: &Value, current: &Value) -> bool {
    latest["status"] != "unavailable"
        && (latest["status"] == "retracted"
            || (current["status"] == "observed" && truth(&current["confirmed"])))
}

pub(super) fn resolve<O: storage::WriteObserver>(
    w: &mut Writer<'_, O>,
    wid: &str,
    pid: &str,
    at: &str,
    clock: &mut impl FnMut() -> Result<String, Refusal>,
) -> Result<usize, Refusal> {
    let custody_at = clock()?;
    let pending=rows(w.conn,"SELECT impact.*,snapshot.report_state,snapshot.report_scope_json FROM custody_filed_report_impacts impact JOIN filed_report_snapshots snapshot ON snapshot.id=impact.filed_report_snapshot_id LEFT JOIN custody_filed_report_impact_resolutions resolution ON resolution.impact_id=impact.id WHERE impact.profile_id=? AND resolution.id IS NULL ORDER BY impact.affected_period_start_year,impact.created_at,impact.id",[pid])?;
    let mut resolutions = Vec::new();
    for r in &pending {
        let (class, gain) = summaries(
            w.conn,
            pid,
            &r["affected_period_start_year"],
            &r["affected_period_end_year"],
            &object(&r["report_scope_json"]),
        )?;
        let result = compare(
            object(&r["before_classification_summary_json"]),
            object(&r["before_gain_summary_json"]),
            &r["report_state"],
            class,
            gain,
        );
        resolutions.push(vec![
            t(uuid5(&string(&r["id"]))),
            t(wid),
            t(pid),
            bind(&r["id"])?,
            t(at),
            t(pyjson(&result["after_classification_summary"], true)?),
            t(pyjson(&result["after_gain_summary"], true)?),
            bind(&result["classification_changed"])?,
            bind(&result["gain_changed"])?,
            bind(&result["amendment_status"])?,
            t(&custody_at),
        ]);
    }
    let mut count = resolutions.len();
    w.insert(
        "custody_filed_report_impact_resolutions",
        &[
            "id",
            "workspace_id",
            "profile_id",
            "impact_id",
            "rebuilt_at",
            "after_classification_summary_json",
            "after_gain_summary_json",
            "classification_changed",
            "gain_changed",
            "amendment_status",
            "created_at",
        ],
        resolutions,
    )?;
    let pending=rows(w.conn,"SELECT impact.*,inbox.observation_json FROM filed_report_chain_impacts impact JOIN chain_analysis_watch_inbox inbox ON inbox.id=impact.inbox_id LEFT JOIN filed_report_chain_resolutions resolution ON resolution.impact_id=impact.id WHERE impact.profile_id=? AND resolution.impact_id IS NULL",[pid])?;
    let mut resolutions = Vec::new();
    for r in &pending {
        let dependency=rows(w.conn,"SELECT * FROM filed_report_chain_dependencies WHERE snapshot_id=? AND transaction_id=?",params![string(&r["snapshot_id"]),string(&r["transaction_id"])])?;
        let Some(dep) = dependency.first() else {
            continue;
        };
        let dep = codec::from_json(dep)?;
        let latest = codec::to_json(
            &kassiber_wallets::publication::reports::last_observation(w.conn, &dep)
                .map_err(error)?,
        );
        let current = codec::to_json(
            &kassiber_wallets::publication::reports::current(w.conn, &dep).map_err(error)?,
        );
        if !chain_ready(&latest, &current) {
            continue;
        }
        let snapshots = rows(
            w.conn,
            "SELECT * FROM filed_report_snapshots WHERE id=? AND profile_id=?",
            params![string(&r["snapshot_id"]), pid],
        )?;
        let s = snapshots
            .first()
            .ok_or_else(|| error("filed report snapshot not found"))?;
        let (class, gain) = summaries(
            w.conn,
            pid,
            &s["period_start_year"],
            &s["period_end_year"],
            &object(&s["report_scope_json"]),
        )?;
        let result = compare(
            object(&s["classification_summary_json"]),
            object(&s["gain_summary_json"]),
            &s["report_state"],
            class,
            gain,
        );
        resolutions.push(vec![
            bind(&r["id"])?,
            t(at),
            t(pyjson(&result, true)?),
            t(clock()?),
        ]);
    }
    count += resolutions.len();
    w.insert(
        "filed_report_chain_resolutions",
        &["impact_id", "rebuilt_at", "summary_json", "created_at"],
        resolutions,
    )?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn filed_chain_resolution_requires_authoritative_retraction_or_confirmed_observation() {
        for (latest, current, confirmed, ready) in [
            ("unavailable", "observed", true, false),
            ("unavailable", "unavailable", false, false),
            ("retracted", "unavailable", false, true),
            ("retracted", "observed", false, true),
            ("observed", "unavailable", true, false),
            ("observed", "observed", false, false),
            ("observed", "observed", true, true),
        ] {
            assert_eq!(
                chain_ready(
                    &json!({"status":latest}),
                    &json!({"status":current,"confirmed":confirmed})
                ),
                ready
            );
        }
    }
    #[test]
    fn filed_summary_parsing_preserves_safe_fallbacks_and_scalar_refusals() {
        for value in [json!(null), json!("bad"), json!("[]"), json!("null")] {
            assert_eq!(object(&value), json!({}));
        }
        assert_eq!(object(&json!("{\"a\":1}")), json!({"a":1}));
        let c = Connection::open_in_memory().unwrap();
        let values = rows(&c, "SELECT NULL AS n, 1 AS i, 1.5 AS f, 'é' AS s", []).unwrap();
        assert_eq!(values, [json!({"n":null,"i":1,"f":1.5,"s":"é"})]);
        assert!(rows(&c, "SELECT x'00' AS blob", []).is_err());
        assert!(summaries(&c, "p", &Value::Null, &json!(2024), &json!({})).is_err());
        for (start, end) in [(Value::Null, json!(2024)), (json!(2024), Value::Null)] {
            let refusal = summaries(&c, "p", &start, &end, &json!({})).unwrap_err();
            assert_eq!(refusal.family, "RuntimeError");
            assert_eq!(refusal.message, "invalid report year");
        }
        for (query, family) in [
            ("SELECT x'00' AS blob", "DatabaseError"),
            ("SELECT CAST(x'ff' AS TEXT) AS invalid", "DatabaseError"),
            ("SELECT absent FROM missing", "OperationalError"),
        ] {
            assert_eq!(rows(&c, query, []).unwrap_err().family, family);
        }
        assert_eq!(
            rows(&c, "SELECT ? AS absent", []).unwrap_err().family,
            "DatabaseError"
        );
        // Valid years reach the delegated summary service: its driver refusal
        // remains distinct from malformed-year validation above.
        let refusal = summaries(&c, "p", &json!(2024), &json!(2024), &json!({})).unwrap_err();
        assert_eq!(refusal.family, "RuntimeError");
        assert_ne!(refusal.message, "invalid report year");
    }
}
