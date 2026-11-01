use super::*;
use sha2::{Digest, Sha256};

pub(super) fn pack(row: &Value) -> Result<Value> {
    let mut value = row.clone();
    remove(&mut value, "profile_id");
    remove(&mut value, "manifest_json");
    let manifest = kassiber_pytext::loads(text(row, "manifest_json"))
        .map_err(|_| AppError::new("internal_error", "Invalid stored dataset manifest"))?;
    set(&mut value, "manifest", manifest);
    set(&mut value, "accounting_authority", Value::Bool(false));
    Ok(value)
}
pub(super) fn get_pack(conn: &Connection, profile: &str, id: &Value) -> Result<Value> {
    let id = match id {
        Value::Null => Sql::Null,
        Value::Str(v) => Sql::Text(v.as_str().unwrap_or("").to_owned()),
        Value::Int(n) => {
            Sql::Integer(i64::try_from(n.clone()).map_err(|_| invalid("Invalid dataset identity"))?)
        }
        Value::Bool(v) => Sql::Integer(i64::from(*v)),
        Value::Float(v) => Sql::Real(*v),
        _ => return Err(invalid("Invalid dataset identity")),
    };
    let found = rows(
        conn,
        "SELECT * FROM chain_analysis_datasets WHERE profile_id=? AND id=?",
        [Sql::Text(profile.to_owned()), id],
    )?;
    pack(
        found
            .first()
            .ok_or_else(|| AppError::new("not_found", "Dataset not found in this book"))?,
    )
}
pub(super) fn query(conn: &Connection, profile: &str, args: &Value) -> Result<Value> {
    arguments(
        args,
        &[
            "subject",
            "chain",
            "network",
            "label",
            "dataset_id",
            "limit",
            "cursor",
            "observer",
        ],
        &[],
    )
    .map_err(|_| invalid("Unsupported attribution query arguments"))?;
    let observer = get(args, "observer").cloned().unwrap_or_else(|| s("owner"));
    let size = limit(
        args,
        "limit",
        50,
        100,
        "Invalid attribution query observer or limit",
    )?;
    if ![s("public"), s("owner"), s("disclosed")].contains(&observer) {
        return Err(invalid("Invalid attribution query observer or limit"));
    }
    let has_subject = get(args, "subject").is_some();
    let has_label = get(args, "label").is_some();
    if has_subject == has_label
        || (has_label
            && (!truthy(field(args, "dataset_id"))
                || get(args, "chain").is_some()
                || get(args, "network").is_some()))
    {
        return Err(invalid(
            "Query an exact subject with chain/network, or an exact label with dataset_id",
        ));
    }
    let (mut query, column, index, value) = if has_subject {
        let domain = engine::datasets::canonical_subject(
            field(args, "chain"),
            field(args, "network"),
            field(args, "subject"),
        )?;
        let v = list(&domain);
        (
            dict([
                ("chain", v[0].clone()),
                ("network", v[1].clone()),
                ("subject", v[2].clone()),
            ]),
            "subject",
            "idx_chain_analysis_dataset_subject",
            v[2].clone(),
        )
    } else {
        let label = s(text_value(field(args, "label"), "label", 256)?);
        (
            dict([("label", label.clone())]),
            "label",
            "idx_chain_analysis_dataset_entity",
            label,
        )
    };
    set(&mut query, "dataset_id", field(args, "dataset_id").clone());
    set(&mut query, "observer", observer.clone());
    atomic(conn, || {
        let mut packs = if truthy(field(args, "dataset_id")) {
            text_value(field(args, "dataset_id"), "dataset_id", 64)?;
            let pack = get_pack(
                conn,
                profile,
                &s(text_value(field(args, "dataset_id"), "dataset_id", 64)?),
            )?;
            if observer == s("public") && field(&pack, "visibility") != &s("public") {
                return Err(AppError::new("not_found", "Dataset not found in this view"));
            }
            if ["staging", "failed"].contains(&text(&pack, "status")) {
                return Err(invalid("An incomplete dataset cannot be queried"));
            }
            if !has_subject
                || (field(&pack, "chain") == field(&query, "chain")
                    && field(&pack, "network") == field(&query, "network"))
            {
                vec![pack]
            } else {
                vec![]
            }
        } else {
            let visible = if observer == s("public") {
                " AND visibility='public'"
            } else {
                ""
            };
            rows(conn, &format!("SELECT * FROM chain_analysis_datasets WHERE profile_id=? AND chain=? AND network=? AND status='active'{visible} ORDER BY id"), [profile, text(&query, "chain"), text(&query, "network")])?
                .iter().map(pack).collect::<Result<Vec<_>>>()?
        };
        packs.sort_by(|a, b| text(a, "id").cmp(text(b, "id")));
        let encoded = dumps(
            &Value::List(vec![s(profile), query.clone(), Value::List(packs.clone())]),
            &Options::new()
                .sort_keys(true)
                .compact()
                .ensure_ascii(false)
                .allow_nan(false),
        )
        .map_err(|_| invalid("Invalid dataset query state"))?;
        let binding: String = Sha256::digest(encoded.as_bytes())
            .iter()
            .map(|v| format!("{v:02x}"))
            .collect();
        let (mut after_pack, mut after_record) = (String::new(), 0i64);
        if truthy(field(args, "cursor")) {
            let fail = || {
                AppError::new(
                    "chain_analysis_stale",
                    "Attribution cursor changed or belongs to another query",
                )
                .retryable(true)
            };
            let raw = text_value(field(args, "cursor"), "cursor", 2048).map_err(|_| fail())?;
            let decoded = cursor::decode(raw).ok_or_else(fail)?;
            let v = list(&decoded);
            if v.len() != 3 || v[0] != s(binding.as_str()) || v[1].as_str().is_none() {
                return Err(fail());
            }
            let Value::Int(n) = &v[2] else {
                return Err(fail());
            };
            after_record = i64::try_from(n.clone()).map_err(|_| fail())?;
            if after_record < 0 {
                return Err(fail());
            }
            after_pack = v[1].as_str().unwrap().to_owned();
        }
        let mut items = Vec::new();
        let mut positions = Vec::new();
        for pack in &packs {
            let id = text(pack, "id");
            if id < after_pack.as_str() {
                continue;
            }
            let record = if id == after_pack { after_record } else { 0 };
            for row in rows(conn, &format!("SELECT * FROM chain_analysis_dataset_claims INDEXED BY {index} WHERE dataset_id=? AND {column}=? AND record_number>? ORDER BY record_number LIMIT ?"),
                [Sql::Text(id.to_owned()), Sql::Text(value.as_str().unwrap().to_owned()), Sql::Integer(record), Sql::Integer(size + 1 - items.len() as i64)])? {
                items.push(engine::datasets::matched(pack, &row)?);
                positions.push((s(id), field(&row,"record_number").clone()));
            }
            if items.len() > size as usize {
                break;
            }
        }
        let next = if items.len() > size as usize {
            let (id, record) = &positions[size as usize - 1];
            cursor::encode(
                &Value::List(vec![s(binding.as_str()), id.clone(), record.clone()]),
                false,
            )?
        } else {
            Value::Null
        };
        items.truncate(size as usize);
        Ok(dict([
            ("items", Value::List(items)),
            ("datasets", Value::List(packs)),
            ("query", query.clone()),
            ("next_cursor", next),
            ("dataset_state_digest", s(binding)),
            (
                "coverage",
                dict([
                    ("scope", s("exact_local_attribution_claims")),
                    ("historical_inspection", Value::Bool(true)),
                    ("accounting_authority", Value::Bool(false)),
                ]),
            ),
        ]))
    })
}
