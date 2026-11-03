//! `_rates_rebuild_payload`: the desktop "rebuild rates" action. The request is
//! the explicit, exact-action consent for its provider fetch; live sources also
//! need the book's market-data opt-in, and the shared offline gate still applies.
use kassiber_envelope::redaction::{redact_error_details, redact_secret_text};
use kassiber_envelope::{dict, get, py_str, s, truthy, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;

use super::{error, map_sqlite_error, OptionalExtension, RealBook};

fn validation(message: &str) -> AppError {
    AppError::new("validation", message)
}
fn optional_text(args: &Value, key: &str) -> Result<Option<String>, AppError> {
    match get(args, key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Str(_)) => Ok(Some(py_str(get(args, key).unwrap()))),
        Some(_) => Err(validation(&format!(
            "ui.rates.rebuild {key} must be a string"
        ))),
    }
}
/// `int(args.get("days") or 30)`.
fn days(args: &Value) -> Result<i64, AppError> {
    let invalid = || validation("ui.rates.rebuild days must be a positive integer");
    let raw = get(args, "days").filter(|v| truthy(v));
    let days = match raw {
        None => 30,
        Some(Value::Bool(b)) => i64::from(*b),
        Some(Value::Int(n)) => i64::try_from(n.clone()).map_err(|_| invalid())?,
        Some(Value::Float(n)) if n.is_nan() => return Err(invalid()),
        Some(Value::Float(n)) if n.is_infinite() => {
            return Err(AppError::new(
                "internal_error",
                "cannot convert float infinity to integer",
            ))
        }
        Some(Value::Float(n)) => n.trunc() as i64,
        Some(Value::Str(_)) => kassiber_pytext::parse_int(&py_str(raw.unwrap()))
            .ok()
            .and_then(|n| i64::try_from(n).ok())
            .ok_or_else(invalid)?,
        Some(_) => return Err(invalid()),
    };
    if days <= 0 {
        return Err(invalid());
    }
    Ok(days)
}
/// `_app_error_payload`.
fn error_payload(error: &AppError) -> Value {
    dict([
        ("code", s(error.code.as_str())),
        ("message", s(redact_secret_text(&error.message))),
        (
            "hint",
            error
                .hint
                .as_deref()
                .filter(|h| !h.is_empty())
                .map_or(Value::Null, |h| s(redact_secret_text(h))),
        ),
        (
            "details",
            error
                .details
                .as_ref()
                .map_or(Value::Null, redact_error_details),
        ),
        ("retryable", Value::Bool(error.retryable)),
    ])
}

impl RealBook {
    fn profile_row(&self, id: &str) -> Result<(String, i64), AppError> {
        self.conn()?
            .query_row(
                "SELECT COALESCE(fiat_currency,''),COALESCE(journal_input_version,0) FROM profiles WHERE id=?",
                [id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()
            .map_err(map_sqlite_error)
            .map_err(error)?
            .ok_or_else(|| AppError::new("not_found", "profile not found"))
    }

    pub(super) fn rates_rebuild(&mut self, args: &Value) -> Result<Value, AppError> {
        let conn = self.conn()?;
        let source = match get(args, "source").filter(|v| truthy(v)) {
            Some(value) => py_str(value),
            None => kassiber_rates::market_provider(conn)?,
        };
        let source = kassiber_pytext::strip(&source).to_lowercase();
        let pair = optional_text(args, "pair")?
            .map(|p| kassiber_pytext::strip(&p).to_owned())
            .filter(|p| !p.is_empty());
        let path = optional_text(args, "path")?;
        let days = days(args)?;
        let reprice = get(args, "reprice_transactions").is_none_or(truthy);
        let mut profile = None;
        let pair = match pair {
            Some(pair) => pair,
            None => {
                let (_, id) = self.resolve_scope(None, None)?;
                let (fiat, _) = self.profile_row(&id)?;
                profile = Some(id);
                kassiber_rates::transaction_rate_pair("BTC", &fiat).ok_or_else(|| {
                    validation(
                        "Active profile fiat currency is not supported for automatic BTC rate rebuild",
                    )
                    .with_details(dict([("fiat_currency", s(fiat))]))
                })?
            }
        };
        if kassiber_rates::LIVE_SOURCES.contains(&source.as_str()) {
            let id = match &profile {
                Some(id) => id.clone(),
                None => self.resolve_scope(None, None)?.1,
            };
            kassiber_rates::require_live_enabled(self.conn()?, &id)?;
            profile = Some(id);
        }
        let mut before = None;
        if reprice {
            let id = match &profile {
                Some(id) => id.clone(),
                None => self.resolve_scope(None, None)?.1,
            };
            before = Some(self.profile_row(&id)?.1);
            profile = Some(id);
        }
        let path = path
            .map(|p| kassiber_pytext::strip(&p).to_owned())
            .filter(|p| !p.is_empty());
        let mut request = dict([
            ("source", s(source.as_str())),
            ("pair", s(pair.as_str())),
            ("days", Value::from(days)),
            ("reprice_transactions", Value::Bool(reprice)),
        ]);
        if let Some(path) = &path {
            kassiber_envelope::set(&mut request, "path", s(path.as_str()));
        }
        if reprice {
            kassiber_envelope::set(
                &mut request,
                "profile_id",
                s(profile.as_deref().unwrap_or_default()),
            );
        }
        let conn = self.conn()?;
        // Invalid sources and pairs are refused by the rebuild itself, in Python's order.
        let consent = kassiber_rates::FetchConsent::explicit(
            conn,
            kassiber_rates::FetchAction::Rebuild,
            &source,
            Some(&pair),
        )
        .ok();
        let rebuilt =
            kassiber_rates::command(conn, "rebuild", &request, &self.rates, consent.as_ref())?;
        let mut payload = rebuilt;
        let mut reprice_payload = Value::Null;
        let mut journals = Value::Null;
        if reprice {
            let (_, id) = self.resolve_scope(None, None)?;
            let (fiat, after) = self.profile_row(&id)?;
            let conn = self.conn()?;
            conn.execute_batch("BEGIN IMMEDIATE")
                .map_err(map_sqlite_error)
                .map_err(error)?;
            let priced = (|| {
                let priced = kassiber_journals::service::auto_price(conn, &id, &fiat)?;
                if priced > 0 && Some(after) == before {
                    conn.execute(
                        "UPDATE profiles SET last_processed_at=NULL,last_processed_tx_count=0,journal_input_version=journal_input_version+1,ownership_review_counts_json=NULL WHERE id=?",
                        [&id],
                    )
                    .map_err(map_sqlite_error)
                    .map_err(error)?;
                }
                Ok::<_, AppError>(priced)
            })();
            match priced {
                Ok(priced) => {
                    conn.execute_batch("COMMIT")
                        .map_err(map_sqlite_error)
                        .map_err(error)?;
                    reprice_payload = dict([("auto_priced", Value::from(priced as i64))]);
                }
                Err(failure) => {
                    let _ = conn.execute_batch("ROLLBACK");
                    return Err(failure);
                }
            }
            journals = match self.process_journals(None, None, false, &mut |_| {}) {
                Ok(reply) => {
                    let result = reply.payload().clone();
                    reply.after_reply()?;
                    dict([("ok", Value::Bool(true)), ("result", result)])
                }
                Err(failure) if failure.error.code != "internal_error" => dict([
                    ("ok", Value::Bool(false)),
                    ("error", error_payload(&failure.error)),
                ]),
                Err(failure) => return Err(*failure.error),
            };
        }
        kassiber_envelope::set(&mut payload, "reprice", reprice_payload);
        kassiber_envelope::set(&mut payload, "journals", journals);
        Ok(payload)
    }
}
