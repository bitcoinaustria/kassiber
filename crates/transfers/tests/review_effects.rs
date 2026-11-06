#![cfg(feature = "test-harness")]
use kassiber_envelope::{dict, get, s};
use kassiber_journals::review_effects;
use kassiber_journals::{
    pipeline::Prepared, Holding, Holdings, JournalDecimal, JournalEntryRow, JournalState,
    QuarantineRow, TaxSummaryRow,
};
use kassiber_pyjson::Value as Py;
use kassiber_store::rusqlite::{types::Value as Sql, Connection};
use kassiber_transfers::{review_operations::canonical, Scope};
use serde_json::{json, Value};

fn py(value: &Value) -> Py {
    kassiber_custody::codec::from_json(value).unwrap()
}
fn decimal(value: &Value) -> JournalDecimal {
    JournalDecimal(value["$decimal"].as_str().unwrap().parse().unwrap())
}
fn optional(value: &Value) -> Option<JournalDecimal> {
    (!value.is_null()).then(|| decimal(value))
}
fn float(value: &Value) -> f64 {
    value
        .as_f64()
        .unwrap_or_else(|| value["$float"].as_str().unwrap().parse().unwrap())
}
fn string(value: &Value) -> &'static str {
    match value.as_str().unwrap() {
        "acquisition" => "acquisition",
        "income" => "income",
        "disposal" => "disposal",
        "fee" => "fee",
        "transfer_fee" => "transfer_fee",
        "transfer_in" => "transfer_in",
        "transfer_out" => "transfer_out",
        "long_term" => "long_term",
        "short_term" => "short_term",
        value => panic!("test vocabulary {value}"),
    }
}
fn entries(state: &Value) -> Vec<JournalEntryRow<'_>> {
    state["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| JournalEntryRow {
            id: row["id"].as_str().unwrap().into(),
            workspace_id: &row["workspace_id"],
            profile_id: &row["profile_id"],
            transaction_id: row["transaction_id"].clone(),
            wallet_id: &row["wallet_id"],
            account_id: &row["account_id"],
            occurred_at: row["occurred_at"].clone(),
            entry_type: string(&row["entry_type"]),
            asset: row["asset"].as_str().unwrap().into(),
            quantity: decimal(&row["quantity"]).into(),
            fiat_value: decimal(&row["fiat_value"]),
            unit_cost: decimal(&row["unit_cost"]),
            cost_basis: optional(&row["cost_basis"]),
            proceeds: optional(&row["proceeds"]),
            gain_loss: optional(&row["gain_loss"]),
            description: row["description"].as_str().unwrap().into(),
            at_category: row["at_category"].as_str().map(str::to_owned),
            at_kennzahl: row
                .get("at_kennzahl")
                .map(|v| v.as_i64().map(|v| i32::try_from(v).unwrap())),
            capital_gains_type: row["capital_gains_type"]
                .as_str()
                .map(|_| string(&row["capital_gains_type"])),
            calculation: None,
        })
        .collect()
}
fn holdings(value: &Value) -> Holdings {
    Holdings {
        pairs: value["$pairs"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .map(|row| {
                        (
                            row[0].as_array().unwrap().clone(),
                            Holding {
                                quantity: decimal(&row[1]["quantity"]),
                                cost_basis: decimal(&row[1]["cost_basis"]),
                            },
                        )
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}
fn state(value: &Value) -> JournalState<'_> {
    JournalState {
        entries: entries(value),
        quarantines: value["quarantines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| QuarantineRow {
                transaction_id: row["transaction_id"].clone(),
                workspace_id: json!("w"),
                profile_id: json!("p"),
                reason: row["reason"].as_str().unwrap().into(),
                detail_json: row["detail_json"].as_str().unwrap_or("{}").into(),
            })
            .collect(),
        wallet_holdings: holdings(&value["wallet_holdings"]),
        account_holdings: Holdings { pairs: vec![] },
        tax_summary: value["tax_summary"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| TaxSummaryRow {
                year: i32::try_from(row["year"].as_i64().unwrap()).unwrap(),
                asset: row["asset"].as_str().unwrap().into(),
                transaction_type: string(&row["transaction_type"]),
                capital_gains_type: string(&row["capital_gains_type"]),
                quantity: float(&row["quantity"]),
                quantity_msat: row["quantity_msat"].clone(),
                proceeds: float(&row["proceeds"]),
                cost_basis: float(&row["cost_basis"]),
                gain_loss: float(&row["gain_loss"]),
            })
            .collect(),
        custody_component_blockers: &value["custody_component_blockers"],
        custody_quantity: &value["custody_quantity"],
        cross_asset_pairs: &Value::Null,
        direct_swap_payouts: Value::Null,
        ownership_review_counts: Value::Null,
        ownership_review_candidates: Value::Null,
        custody_native_reconciliations: Value::Null,
        custody_transfers: &Value::Null,
        quantity_differences: Value::Null,
        latest_rates: &Value::Null,
        warnings: &Value::Null,
        custody_gap_ignored_transaction_ids: &Value::Null,
        custody_gap_accounting_ignored_transaction_ids: &Value::Null,
    }
}
fn sql(value: &Value) -> Sql {
    match value {
        Value::Null => Sql::Null,
        Value::Bool(v) => Sql::Integer(i64::from(*v)),
        Value::String(v) => Sql::Text(v.clone()),
        Value::Number(v) => v
            .as_i64()
            .map(Sql::Integer)
            .unwrap_or_else(|| Sql::Real(v.as_f64().unwrap())),
        _ => panic!("SQL test value"),
    }
}
fn connection(profile: &Value, identity: &Value, encrypted: bool) -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    if encrypted {
        conn.pragma_update(None, "key", "synthetic-review-effect-key")
            .unwrap();
    }
    conn.execute_batch("PRAGMA temp_store=MEMORY").unwrap();
    let profile = profile.as_object().unwrap();
    let columns = profile
        .keys()
        .map(|key| format!("\"{key}\""))
        .collect::<Vec<_>>()
        .join(",");
    conn.execute_batch(&format!(
        "CREATE TABLE profiles({columns}); CREATE TABLE settings(key TEXT,value)"
    ))
    .unwrap();
    conn.execute(
        &format!(
            "INSERT INTO profiles VALUES({})",
            vec!["?"; profile.len()].join(",")
        ),
        kassiber_store::rusqlite::params_from_iter(profile.values().map(sql)),
    )
    .unwrap();
    if !identity.is_null() {
        conn.execute(
            "INSERT INTO settings VALUES('database_instance_id',?)",
            [sql(identity)],
        )
        .unwrap();
    }
    conn
}
fn check(state: &JournalState<'_>, case: &Value, conn: &Connection) {
    let scope = Scope {
        workspace_id: case["profile"]["workspace_id"].as_str().unwrap(),
        profile_id: case["profile"]["id"].as_str().unwrap(),
    };
    let operations = py(&json!([{"type":"kind_override"}]));
    let changes = conn.total_changes();
    let mode = conn.is_autocommit();
    let transaction = conn.transaction_state(Some("main")).unwrap();
    let economic = review_effects::economic(state);
    if let Some(expected) = case.get("economic_error") {
        let failure = |error: kassiber_envelope::AppError| {
            dict([
                ("code", s(error.code)),
                ("message", s(error.message)),
                ("hint", error.hint.map(s).unwrap_or(Py::Null)),
                ("details", error.details.unwrap_or(Py::Null)),
            ])
        };
        assert_eq!(
            canonical(&failure(economic.unwrap_err())).unwrap(),
            canonical(&py(expected)).unwrap()
        );
        assert_eq!(
            canonical(&failure(
                review_effects::effects_without_tax(state, conn, &scope, &operations).unwrap_err()
            ))
            .unwrap(),
            canonical(&py(&case["error"])).unwrap()
        );
        assert_eq!(conn.total_changes(), changes);
        assert_eq!(conn.is_autocommit(), mode);
        return;
    }
    let economic = economic.unwrap();
    assert_eq!(
        canonical(&economic).unwrap(),
        canonical(&py(&case["economic"])).unwrap(),
        "{}",
        case["name"]
    );
    let actual = match review_effects::effects_without_tax(state, conn, &scope, &operations) {
        Ok(result) => dict([("result", result)]),
        Err(error) => dict([(
            "error",
            dict([
                ("code", s(error.code)),
                ("message", s(error.message)),
                ("hint", error.hint.map(s).unwrap_or(Py::Null)),
                ("details", error.details.unwrap_or(Py::Null)),
            ]),
        )]),
    };
    let key = if case.get("error").is_some() {
        "error"
    } else {
        "result"
    };
    assert_eq!(
        canonical(get(&actual, key).unwrap()).unwrap(),
        canonical(&py(&case[key])).unwrap(),
        "{}",
        case["name"]
    );
    assert_eq!(conn.total_changes(), changes);
    assert_eq!(conn.is_autocommit(), mode);
    assert_eq!(
        conn.transaction_state(Some("main")).unwrap()
            == kassiber_store::rusqlite::TransactionState::Write,
        transaction == kassiber_store::rusqlite::TransactionState::Write
    );
    assert_eq!(
        canonical(
            &review_effects::effects_without_tax(state, conn, &scope, &Py::List(vec![])).unwrap()
        )
        .unwrap(),
        canonical(&economic).unwrap()
    );
}

#[test]
fn review_effects_match_python_complete_builds_and_policy_context() {
    let effects: Value = serde_json::from_str(include_str!("review_effects.json")).unwrap();
    let builds: Vec<Value> =
        serde_json::from_str(include_str!("../../journals/tests/data/review-builds.json")).unwrap();
    for case in effects["rendered"].as_array().unwrap() {
        let build = &builds[case["index"].as_u64().unwrap() as usize];
        let prepared = Prepared::from_snapshot(
            kassiber_custody::parse(&build["snapshot"].to_string()).unwrap(),
        )
        .unwrap();
        for encrypted in [false, true] {
            let conn = connection(
                &case["profile"],
                &json!("0123456789abcdef0123456789abcdef"),
                encrypted,
            );
            conn.execute_batch("CREATE TABLE transactions(id TEXT,profile_id TEXT)")
                .unwrap();
            for id in build["live_ids"].as_array().unwrap() {
                conn.execute(
                    "INSERT INTO transactions VALUES(?,?)",
                    [
                        id.as_str().unwrap(),
                        case["profile"]["id"].as_str().unwrap(),
                    ],
                )
                .unwrap();
            }
            let mut ids = build["entry_ids"].as_array().unwrap().iter();
            let state = prepared
                .review_state(&conn, || ids.next().unwrap().as_str().unwrap().into())
                .unwrap();
            check(&state, case, &conn);
        }
    }
    assert_eq!(effects["rendered"].as_array().unwrap().len(), 33);
}

#[test]
fn review_effects_bind_full_output_before_bounding_and_preserve_exact_scalars() {
    let effects: Value = serde_json::from_str(include_str!("review_effects.json")).unwrap();
    for case in effects["synthetic"].as_array().unwrap() {
        let state = state(&case["state"]);
        for encrypted in [false, true] {
            for transaction in ["", "BEGIN", "BEGIN IMMEDIATE"] {
                let conn = connection(&case["profile"], &case["identity"], encrypted);
                if !transaction.is_empty() {
                    conn.execute_batch(transaction).unwrap();
                }
                check(&state, case, &conn);
            }
        }
    }
    assert!(effects["synthetic"].as_array().unwrap().len() >= 60);
}

#[test]
fn review_effects_read_pending_policy_and_need_no_database_for_unclassified_operations() {
    let effects: Value = serde_json::from_str(include_str!("review_effects.json")).unwrap();
    let cases = effects["synthetic"].as_array().unwrap();
    let base = cases
        .iter()
        .find(|case| case["name"] == "entry-types-exact-scale")
        .unwrap();
    let changed = cases
        .iter()
        .find(|case| case["name"] == "policy-tax_future_policy")
        .unwrap();
    let state = state(&base["state"]);
    let scope = Scope {
        workspace_id: "w",
        profile_id: "p",
    };
    for encrypted in [false, true] {
        let conn = connection(&base["profile"], &base["identity"], encrypted);
        conn.execute_batch("BEGIN; UPDATE profiles SET tax_future_policy='changed'")
            .unwrap();
        check(&state, changed, &conn);
        conn.execute_batch("ROLLBACK").unwrap();
        check(&state, base, &conn);
    }
    let conn = Connection::open_in_memory().unwrap();
    assert!(review_effects::effects_without_tax(
        &state,
        &conn,
        &scope,
        &py(&json!([{"type":"exclude"}]))
    )
    .is_ok());
}

#[test]
fn review_effects_join_live_native_builds_without_writes_or_transaction_changes() {
    let fixture = kassiber_pytext::loads(include_str!("review_cases.json")).unwrap();
    let effects: Value = serde_json::from_str(include_str!("review_effects.json")).unwrap();
    let field = |value: &Py, key: &str| get(value, key).unwrap().clone();
    let list = |value: Py| match value {
        Py::List(values) => values,
        _ => panic!("fixture list"),
    };
    for case in effects["live"].as_array().unwrap() {
        let scenarios = list(field(&fixture, "scenarios"));
        let scenario = scenarios
            .iter()
            .find(|row| field(row, "name") == py(&case["name"]))
            .unwrap();
        let sql = ["schema", "sql", "objects"]
            .into_iter()
            .flat_map(|key| {
                list(field(if key == "sql" { scenario } else { &fixture }, key))
                    .into_iter()
                    .map(|v| kassiber_envelope::py_str(&v))
            })
            .collect::<Vec<_>>()
            .join("\n");
        let build = field(scenario, "build");
        for encrypted in [false, true] {
            for transaction in ["", "BEGIN", "BEGIN IMMEDIATE"] {
                let conn = Connection::open_in_memory().unwrap();
                if encrypted {
                    conn.pragma_update(None, "key", "synthetic-live-effect-key")
                        .unwrap();
                }
                conn.execute_batch("PRAGMA temp_store=MEMORY; PRAGMA foreign_keys=OFF")
                    .unwrap();
                conn.execute_batch(&sql).unwrap();
                conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
                if !transaction.is_empty() {
                    conn.execute_batch(transaction).unwrap();
                }
                let changes = conn.total_changes();
                let mode = conn.is_autocommit();
                let tape = list(field(&build, "entry_ids"));
                let mut ids = tape.iter();
                let result = kassiber_journals::review_workflow::inspect_effects(
                    &conn,
                    &Scope {
                        workspace_id: "w",
                        profile_id: "p",
                    },
                    &py(&json!([{"type":"kind_override"}])),
                    &kassiber_envelope::py_str(&field(&build, "as_of")),
                    || kassiber_envelope::py_str(ids.next().unwrap()),
                )
                .unwrap();
                assert_eq!(
                    canonical(&result).unwrap(),
                    canonical(&py(&case["result"])).unwrap(),
                    "{}",
                    case["name"]
                );
                assert_eq!(conn.total_changes(), changes);
                assert_eq!(conn.is_autocommit(), mode);
            }
        }
    }
    assert_eq!(effects["live"].as_array().unwrap().len(), 14);
}
