use super::*;
const SELECT: &str = "SELECT t.id,t.profile_id,t.wallet_id,w.label AS wallet_label,w.kind AS wallet_kind,t.occurred_at,t.direction,t.asset,t.amount,t.fee,t.amount_includes_fee,t.excluded,t.kind,t.kind_override,t.privacy_boundary,t.external_id,t.raw_json FROM transactions t JOIN wallets w ON w.id=t.wallet_id ";
pub(super) fn load(
    conn: &Connection,
    profile_id: &str,
    configs: &mut HashMap<String, (String, bool)>,
) -> Result<Value> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM transactions WHERE profile_id=? AND excluded=0",
        [profile_id],
        |r| r.get(0),
    )?;
    let wallet_configs = query(
        conn,
        "SELECT id,config_json FROM wallets WHERE profile_id=?",
        [profile_id],
    )?
    .into_iter()
    .map(|row| (text(get(&row, "id")), get(&row, "config_json").clone()))
    .collect::<HashMap<_, _>>();
    let mapped = |mut rows: Vec<Value>| {
        for row in &mut rows {
            let config = wallet_configs
                .get(&text(get(row, "wallet_id")))
                .cloned()
                .unwrap_or(Value::from("{}"));
            set(row, "wallet_config_json", config);
        }
        rows
    };
    let large = count > 50_000;
    let rows = if !large {
        mapped(query(conn,&format!("{SELECT} WHERE t.profile_id=? AND t.excluded=0 ORDER BY t.occurred_at,t.created_at,t.id"),[profile_id])?)
    } else {
        let structured="LOWER(COALESCE(t.privacy_boundary,'')) IN ('coinjoin','payjoin','whirlpool') OR LOWER(COALESCE(w.kind,'')) IN ('samourai','samourai-whirlpool','whirlpool') OR LOWER(COALESCE(t.kind,'')) IN ('badbank','coinjoin','postmix','premix','samourai_badbank','samourai_deposit','samourai_postmix','samourai_premix','samourai_tx0','whirlpool_mix','whirlpool_tx0') OR LOWER(COALESCE(w.config_json,'')) LIKE '%\"samourai\"%'";
        let mut sources=mapped(query(conn,&format!("{SELECT} WHERE t.profile_id=? AND t.excluded=0 AND t.direction='outbound' AND ({structured}) ORDER BY t.occurred_at,t.created_at,t.id LIMIT 256"),[profile_id])?);
        let budget = 64_usize.min(256 - sources.len());
        if budget > 0 {
            let mut ordinary = vec![];
            for asset in query(conn,"SELECT DISTINCT t.asset FROM transactions t WHERE t.profile_id=? AND t.excluded=0 AND t.direction='outbound' ORDER BY t.asset",[profile_id])? {
                let params=vec![rusqlite::types::Value::Text(profile_id.into()),sql_parameter(get(&asset,"asset"))?,rusqlite::types::Value::Integer(budget as i64)];
                ordinary.extend(mapped(query(conn,&format!("{SELECT} WHERE t.profile_id=? AND t.excluded=0 AND t.direction='outbound' AND t.asset=? AND NOT ({structured}) ORDER BY t.amount DESC,t.occurred_at DESC,t.created_at DESC,t.id DESC LIMIT ?"),rusqlite::params_from_iter(params))?));
            }
            let mut keyed = ordinary
                .into_iter()
                .map(|row| {
                    Ok((
                        (
                            -ownership::integer(get(&row, "amount")).unwrap_or(0.into()),
                            string_or_empty(get(&row, "occurred_at")),
                            string_or_empty(get(&row, "id")),
                        ),
                        row,
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            keyed.sort_by(|a, b| a.0.cmp(&b.0));
            sources.extend(keyed.into_iter().take(budget).map(|(_, row)| row));
        }
        let native = sources
            .iter()
            .map(kassiber_custody::codec::to_json)
            .collect::<Vec<_>>();
        let ranges =
            kassiber_custody::gaps::worklist_amount_ranges(&native).map_err(Error::Invalid)?;
        let mut returns = vec![];
        if !ranges.is_empty() {
            let range_sql =
                vec!["(t.asset=? AND t.amount BETWEEN ? AND ?)"; ranges.len()].join(" OR ");
            let mut parameters = vec![rusqlite::types::Value::Text(profile_id.into())];
            for (asset, low, high) in ranges {
                parameters.extend([
                    rusqlite::types::Value::Text(asset),
                    rusqlite::types::Value::Integer(i64::try_from(low).map_err(|_| {
                        Error::Invalid("gap lower bound exceeds SQLite INTEGER".into())
                    })?),
                    rusqlite::types::Value::Integer(i64::try_from(high).map_err(|_| {
                        Error::Invalid("gap upper bound exceeds SQLite INTEGER".into())
                    })?),
                ]);
            }
            returns=mapped(query(conn,&format!("{SELECT} WHERE t.profile_id=? AND t.excluded=0 AND t.direction='inbound' AND ({range_sql}) ORDER BY t.occurred_at,t.created_at,t.id LIMIT 512"),rusqlite::params_from_iter(parameters))?);
        }
        sources.extend(returns);
        sources.sort_by_key(|row| {
            (
                string_or_empty(get(row, "occurred_at")),
                string_or_empty(get(row, "id")),
            )
        });
        sources
    };
    Ok(dict([
        ("large_book", Value::Bool(large)),
        ("rows", list(safe_rows(rows, configs)?)),
    ]))
}
fn string_or_empty(value: &Value) -> String {
    if truth(value) {
        text(value)
    } else {
        String::new()
    }
}
fn sql_parameter(value: &Value) -> Result<rusqlite::types::Value> {
    Ok(match value {
        Value::Null => rusqlite::types::Value::Null,
        Value::Int(value) => rusqlite::types::Value::Integer(
            i64::try_from(value).map_err(|_| Error::Invalid("SQL INTEGER".into()))?,
        ),
        Value::Float(value) => rusqlite::types::Value::Real(*value),
        Value::Str(value) => rusqlite::types::Value::Text(
            value
                .as_str()
                .ok_or(Error::Unsupported("surrogate SQL text"))?
                .into(),
        ),
        _ => return Err(Error::Invalid("SQL parameter".into())),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_loader_worklist_parameters_keep_sqlite_scalar_types() {
        use rusqlite::types::Value as S;
        for (value, expected) in [
            (Value::Null, S::Null),
            (Value::from(-1), S::Integer(-1)),
            (Value::Float(1.5), S::Real(1.5)),
            (Value::from("BTC"), S::Text("BTC".into())),
        ] {
            assert_eq!(sql_parameter(&value).unwrap(), expected);
        }
        for value in [
            Value::Bool(true),
            list(vec![]),
            Value::from(u64::MAX),
            Value::Str(kassiber_pyjson::PyStr::from_code_points([0xd800]).unwrap()),
        ] {
            assert!(sql_parameter(&value).is_err());
        }
        assert_eq!(string_or_empty(&Value::Null), "");
        assert_eq!(string_or_empty(&Value::from("date")), "date");
    }
}
