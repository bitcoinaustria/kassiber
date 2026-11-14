use crate::{boundary, loader, Computed, Fusion, JournalState, Refusal};
use kassiber_custody::{
    codec,
    snapshot::{BookSnapshot, Snapshot},
};
use kassiber_pydecimal::{Context, Decimal, DecimalError};
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

/// The text-fusion journal build of a v1 snapshot, kept for the frozen
/// review-build comparisons. Review itself builds through
/// [`load_review_state`]; see TODO.md (R4) for its retirement.
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
        state.quarantines = review_quarantines(conn, &self.profile, &state.quarantines)?;
        Ok(state)
    }
}

/// Review's journal state of `profile` on `conn`'s current read snapshot,
/// handed to `then`. The book is loaded as of `as_of` and rendered by the
/// custody, fusion, engine and render steps `journals process` stores from
/// (`process::render_loaded`), so review and processing cannot compute
/// different journals for any tax country.
pub fn load_review_state<R>(
    conn: &Connection,
    profile: Py,
    as_of: &str,
    then: impl FnOnce(JournalState<'_>) -> R,
) -> Result<R, Refusal> {
    let loaded = loader::load_read_snapshot(conn, profile, as_of).map_err(load_error)?;
    let snapshot = loaded.into_custody().map_err(load_error)?;
    review_state(conn, &snapshot, then)
}

/// [`load_review_state`] over a loaded snapshot.
pub fn review_state<R>(
    conn: &Connection,
    snapshot: &Snapshot,
    then: impl FnOnce(JournalState<'_>) -> R,
) -> Result<R, Refusal> {
    let profile = serde_json::to_value(&snapshot.profile)
        .map_err(|error| Refusal::new("ValueError", error.to_string()))?;
    crate::process::render_loaded(&profile, snapshot, |state| {
        let mut state = state.clone();
        state.quarantines = review_quarantines(conn, &profile, &state.quarantines)?;
        Ok(then(state))
    })
}

/// Review cases: each quarantined transaction once, under its primary
/// reason, and only the profile's live transactions.
fn review_quarantines(
    conn: &Connection,
    profile: &Value,
    quarantines: &[crate::QuarantineRow],
) -> Result<Vec<crate::QuarantineRow>, Refusal> {
    let mut statement = conn.prepare("SELECT id FROM transactions WHERE profile_id=?")?;
    let live: HashSet<String> = statement
        .query_map([crate::string(&profile["id"])], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    Ok(crate::persistence::dedupe_quarantines(quarantines)?
        .into_iter()
        .filter(|row| live.contains(&crate::string(&row.transaction_id)))
        .collect())
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
    kassiber_pytext::strip(&text)
        .parse()
        .map_err(|error: kassiber_pydecimal::DecimalError| {
            // `raise AppError(...) from exc`: the InvalidOperation is the cause.
            let mut refusal = Refusal::new("AppError", format!("Invalid decimal value: {text}"));
            refusal.code = Some("app_error".into());
            let mut cause = Refusal::new(
                "InvalidOperation",
                format!("[<class 'decimal.{}'>]", error.name()),
            );
            cause.retryable = None;
            refusal.cause = Some(Box::new(cause));
            refusal
        })
}

/// A stored price as `dec()` reads it: Python's Decimal also holds NaN and
/// the infinities, which this decimal type cannot, so they keep their
/// canonical text (`NaN12`, `-Infinity`, `sNaN`).
enum Price {
    Finite(Decimal),
    Special(String),
}

fn price(value: &Value) -> Result<Price, Refusal> {
    match decimal(value) {
        Ok(decimal) => Ok(Price::Finite(decimal)),
        Err(refusal) => {
            let text = codec::pystr(value);
            let normalized: String = kassiber_pytext::strip(&text)
                .chars()
                .filter(|&c| c != '_')
                .collect();
            let sign = if normalized.starts_with('-') { "-" } else { "" };
            let unsigned = normalized.trim_start_matches(['+', '-']).to_lowercase();
            let payload = |rest: &str| {
                rest.bytes()
                    .all(|b| b.is_ascii_digit())
                    .then(|| rest.trim_start_matches('0').to_owned())
            };
            let special = if let Some(digits) = unsigned.strip_prefix("snan").and_then(payload) {
                format!("{sign}sNaN{digits}")
            } else if let Some(digits) = unsigned.strip_prefix("nan").and_then(payload) {
                format!("{sign}NaN{digits}")
            } else if matches!(unsigned.as_str(), "inf" | "infinity") {
                format!("{sign}Infinity")
            } else {
                return Err(refusal);
            };
            Ok(Price::Special(special))
        }
    }
}

/// `rates_from_transaction_rows`: each asset's latest rate, else its latest
/// value over `msat_to_btc(amount)`, in the loader's precedence order. A
/// quiet NaN or infinity stays a rate, as Python's Decimal arithmetic keeps
/// it; a signalling NaN raises InvalidOperation there.
pub(crate) fn rates(rows: &[kassiber_custody::snapshot::Record]) -> Result<Value, Refusal> {
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
                .map(price)
                .transpose()
        };
        let rate = source("fiat_rate_exact", "fiat_rate")?;
        let value = source("fiat_value_exact", "fiat_value")?;
        let rate = match rate {
            Some(rate) => Some(rate),
            None if codec::truth(&row["amount"]) => value
                .map(|value| {
                    let amount: Decimal = codec::integer(&row["amount"])?.to_string().parse()?;
                    let quantity = amount.div(&Decimal::from(100_000_000_000i64), &Context::RP2)?;
                    Ok::<_, Refusal>(match value {
                        Price::Finite(value) => Price::Finite(value.div(&quantity, &Context::RP2)?),
                        Price::Special(text)
                            if text.trim_start_matches('-').starts_with("sNaN") =>
                        {
                            return Err(DecimalError::InvalidOperation.into())
                        }
                        // A quiet NaN keeps its sign and payload.
                        Price::Special(text) if text.contains("NaN") => Price::Special(text),
                        Price::Special(text) => {
                            let negative = text.starts_with('-') != quantity.is_negative();
                            Price::Special(if negative { "-Infinity" } else { "Infinity" }.into())
                        }
                    })
                })
                .transpose()?,
            None => None,
        };
        let text = match rate {
            Some(Price::Finite(rate)) => rate.to_string(),
            Some(Price::Special(text)) => text,
            None => continue,
        };
        rates.insert(asset, json!({ "$decimal": text }));
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
    fn rates_keep_python_nan_and_infinities() {
        // rates_from_transaction_rows of the same rows, under CPython.
        let rows: Vec<kassiber_custody::snapshot::Record> = serde_json::from_value(json!([
            {"asset":"A", "fiat_value_exact":"NaN0012", "amount":2},
            {"asset":"B", "fiat_rate_exact":"-inf", "amount":1},
            {"asset":"C", "fiat_value_exact":"Infinity", "amount":-3},
            {"asset":"D", "fiat_value_exact":"-NaN", "amount":5}
        ]))
        .unwrap();
        assert_eq!(
            rates(&rows).unwrap(),
            json!({
                "A":{"$decimal":"NaN12"},
                "B":{"$decimal":"-Infinity"},
                "C":{"$decimal":"-Infinity"},
                "D":{"$decimal":"-NaN"}
            })
        );
        let signalling: Vec<kassiber_custody::snapshot::Record> =
            serde_json::from_value(json!([{"asset":"E", "fiat_value_exact":"sNaN", "amount":1}]))
                .unwrap();
        assert_eq!(rates(&signalling).unwrap_err().family, "InvalidOperation");
    }

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
