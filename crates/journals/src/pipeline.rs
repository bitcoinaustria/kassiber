use crate::{boundary, loader, Computed, Fusion, JournalState, Refusal};
use kassiber_custody::{codec, snapshot::BookSnapshot};
use kassiber_pydecimal::{Context, Decimal};
use kassiber_pyjson::Value as Py;
use kassiber_store::rusqlite::Connection;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};

pub fn memory_snapshot(
    conn: &Connection,
    mut ephemeral_key: impl FnMut() -> Result<String, Refusal>,
) -> Result<Connection, Refusal> {
    use kassiber_store::rusqlite::{backup::StepResult, TransactionState};
    let active_write = || {
        let mut error = Refusal::new(
            "AppError",
            "Finish the current write transaction before planning a review",
        );
        error.code = Some("review_transaction_active".into());
        error
    };
    if conn.transaction_state(Some("main"))? == TransactionState::Write {
        return Err(active_write());
    }
    let mut clone = Connection::open_in_memory()?;
    let encrypted: String = conn.pragma_query_value(None, "cipher_status", |row| row.get(0))?;
    if encrypted == "1" {
        clone.pragma_update(None, "key", ephemeral_key()?)?;
    }
    clone.execute_batch("PRAGMA temp_store=MEMORY")?;
    {
        let backup = kassiber_store::rusqlite::backup::Backup::new(conn, &mut clone)?;
        loop {
            match backup.step(128)? {
                StepResult::Done => break,
                StepResult::More => (),
                StepResult::Busy | StepResult::Locked => return Err(active_write()),
                _ => return Err(Refusal::new("RuntimeError", "review backup failed")),
            }
        }
    }
    clone.execute_batch("PRAGMA foreign_keys=ON")?;
    Ok(clone)
}

pub struct Prepared {
    response:
        std::cell::OnceCell<Result<kassiber_tax_core::ledger::response::LedgerResponse, Refusal>>,
    pub computed: Computed,
    pub custody: Value,
    pub wallets: Vec<Value>,
    pub profile: Value,
}

fn stage_error(error: kassiber_custody::Refusal) -> Refusal {
    let family = match error.code.as_str() {
        "ValueError" | "TypeError" | "KeyError" | "AttributeError" => error.code.as_str(),
        _ => "AppError",
    };
    let mut refusal = Refusal::new(family, error.message);
    if family == "AppError" {
        // The custody AppError reaches the caller whole: its hint (the
        // network review's "Review the active book's network in Settings")
        // and details too.
        refusal.code = Some(error.code);
        refusal.hint = error.hint;
        refusal.details = error.details;
    }
    refusal
}

impl Prepared {
    pub fn from_snapshot(snapshot: BookSnapshot) -> Result<Self, Refusal> {
        let computed = crate::compute_snapshot(snapshot, true).map_err(stage_error)?;
        Self::from_computed(computed)
    }

    fn from_computed(computed: Computed) -> Result<Self, Refusal> {
        let (mut custody, wallets) = kassiber_custody::journal_support::journal_operands(
            &computed.snapshot,
            &computed.stages,
        )
        .map_err(|message| Refusal::new("ValueError", message))?;
        let profile = serde_json::to_value(&computed.snapshot.profile)
            .map_err(|error| Refusal::new("ValueError", error.to_string()))?;
        custody["rates"] = rates(&computed.snapshot.rate_rows)?;
        custody["warnings"] = warnings(&wallets, &computed.snapshot.ownership_warnings);
        Ok(Self {
            response: std::cell::OnceCell::new(),
            computed,
            custody,
            wallets,
            profile,
        })
    }

    pub fn load(conn: &Connection, profile: Py, as_of: &str) -> Result<Self, Refusal> {
        let loaded = loader::load_read_snapshot(conn, profile, as_of).map_err(load_error)?;
        let snapshot = loaded.into_custody().map_err(load_error)?;
        Self::from_computed(crate::compute_loaded(snapshot, true).map_err(stage_error)?)
    }

    pub fn render(&self, ids: impl FnMut() -> String) -> Result<JournalState<'_>, Refusal> {
        let response = self
            .response
            .get_or_init(|| {
                let response = match &self.computed.fusion {
                    Fusion::Empty => boundary::empty_response(),
                    Fusion::Ledger { response, .. } => {
                        let response = serde_json::from_str(response)
                            .map_err(|error| Refusal::new("ValueError", error.to_string()))?;
                        boundary::ledger_response(&response)?
                    }
                    Fusion::Refused(error) => {
                        let mut refusal = Refusal::new(
                            error["family"].as_str().unwrap_or("RuntimeError"),
                            crate::string(&error["message"]),
                        );
                        refusal.code = error["code"].as_str().map(str::to_owned);
                        refusal.hint = error["hint"].as_str().map(str::to_owned);
                        refusal.details = error["details"].clone();
                        refusal.invariant = error["invariant"].as_str().map(str::to_owned);
                        refusal.error_class = error["error_class"].as_str().map(str::to_owned);
                        return Err(refusal);
                    }
                    Fusion::Skipped => {
                        return Err(Refusal::new(
                            "RuntimeError",
                            "journal build requires finalized custody projection",
                        ));
                    }
                };
                Ok(response)
            })
            .as_ref()
            .map_err(Clone::clone)?;
        boundary::build(&self.profile, &self.custody, &self.wallets, response, ids)
    }

    pub fn review_state(
        &self,
        conn: &Connection,
        ids: impl FnMut() -> String,
    ) -> Result<JournalState<'_>, Refusal> {
        let mut state = self.render(ids)?;
        let mut statement = conn.prepare("SELECT id FROM transactions WHERE profile_id=?")?;
        let live: HashSet<String> = statement
            .query_map([crate::string(&self.profile["id"])], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        state.quarantines = crate::persistence::dedupe_quarantines(&state.quarantines)?
            .into_iter()
            .filter(|row| live.contains(&crate::string(&row.transaction_id)))
            .collect();
        Ok(state)
    }
}

fn load_error(error: loader::Error) -> Refusal {
    match error {
        loader::Error::Reference(error) => error,
        loader::Error::Database(error) => error.into(),
        error => Refusal::new("RuntimeError", error.to_string()),
    }
}

fn decimal(value: &Value) -> Result<Decimal, Refusal> {
    let text = if value.is_null() || value == "" {
        "0".to_owned()
    } else {
        codec::pystr(value)
    };
    kassiber_pytext::strip(&text).parse().map_err(|_| {
        let mut refusal = Refusal::new("AppError", format!("Invalid decimal value: {text}"));
        refusal.code = Some("app_error".into());
        refusal
    })
}

fn rates(rows: &[kassiber_custody::snapshot::Record]) -> Result<Value, Refusal> {
    let mut rates = serde_json::Map::new();
    for row in rows {
        let asset = codec::pystr(&row["asset"]);
        if rates.contains_key(&asset) {
            continue;
        }
        let source = |exact: &str, legacy: &str| {
            row.get(exact)
                .filter(|value| !value.is_null())
                .or_else(|| row.get(legacy).filter(|value| !value.is_null()))
                .map(decimal)
                .transpose()
        };
        let rate = source("fiat_rate_exact", "fiat_rate")?;
        let value = source("fiat_value_exact", "fiat_value")?;
        let rate = match rate {
            Some(rate) => Some(rate),
            None if codec::truth(&row["amount"]) => value
                .map(|value| {
                    let quantity = decimal(&row["amount"])?
                        .div(&Decimal::from(100_000_000_000i64), &Context::RP2)?;
                    Ok::<_, Refusal>(value.div(&quantity, &Context::RP2)?)
                })
                .transpose()?,
            None => None,
        };
        if let Some(rate) = rate {
            rates.insert(asset, json!({"$decimal": rate.to_string()}));
        }
    }
    Ok(Value::Object(rates))
}

fn warnings(wallets: &[Value], ownership: &[String]) -> Value {
    let mut labels = BTreeMap::<String, Vec<String>>::new();
    for wallet in wallets {
        if codec::truth(&wallet["label"]) {
            labels
                .entry(codec::pystr(&wallet["label"]))
                .or_default()
                .push(codec::pystr(&wallet["id"]));
        }
    }
    let mut warnings: Vec<Value> = labels.into_iter().filter(|(_, ids)| ids.len() > 1).map(|(label, mut ids)| {
        ids.sort();
        json!({"code": "duplicate_wallet_label", "label": label, "wallet_ids": ids,
            "message": format!("{} wallets share the label '{label}'. Reports key holdings by wallet label, so their balances merge and a derived self-transfer can be attributed to the wrong wallet. Rename them to be unique.", ids.len())})
    }).collect();
    warnings.extend(
        ownership
            .iter()
            .map(|message| json!({"code": "ownership_index", "message": message})),
    );
    json!(warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_rates_and_warnings_preserve_reference_precedence_and_exactness() {
        let rows: Vec<kassiber_custody::snapshot::Record> = serde_json::from_value(json!([
            {"asset":"BTC", "fiat_rate": null, "fiat_value": null, "amount": 0},
            {"asset":"BTC", "fiat_rate_exact":"12345.678900", "fiat_rate":1.5, "amount":2},
            {"asset":"BTC", "fiat_rate_exact":"99", "amount":2},
            {"asset":"LBTC", "fiat_value_exact":"1.2300", "fiat_value":99, "amount":3},
            {"asset":"LN", "fiat_rate_exact":"", "fiat_rate":10, "amount":1},
            {"asset":"ZERO", "fiat_value_exact":"123", "amount":0},
            {"asset":"LEGACY", "fiat_value": 1.25, "amount":200000000000i64}
        ]))
        .unwrap();
        assert_eq!(
            rates(&rows).unwrap(),
            json!({
                "BTC":{"$decimal":"12345.678900"},
                "LBTC":{"$decimal":"4.100E+10"},
                "LN":{"$decimal":"0"},
                "LEGACY":{"$decimal":"0.625"}
            })
        );
        let wallets = vec![
            json!({"id":"z", "label":"钱"}),
            json!({"id":"b", "label":"A"}),
            json!({"id":"a", "label":"A"}),
            json!({"id":"c", "label":"钱"}),
            json!({"id":"unlabelled", "label":null}),
        ];
        assert_eq!(
            warnings(&wallets, &["retired policy warning".into()]),
            json!([
                {"code":"duplicate_wallet_label","label":"A","wallet_ids":["a","b"],
                 "message":"2 wallets share the label 'A'. Reports key holdings by wallet label, so their balances merge and a derived self-transfer can be attributed to the wrong wallet. Rename them to be unique."},
                {"code":"duplicate_wallet_label","label":"钱","wallet_ids":["c","z"],
                 "message":"2 wallets share the label '钱'. Reports key holdings by wallet label, so their balances merge and a derived self-transfer can be attributed to the wrong wallet. Rename them to be unique."},
                {"code":"ownership_index","message":"retired policy warning"}
            ])
        );
    }
}
