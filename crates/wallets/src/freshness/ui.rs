use super::{field, or_text, redact};
use kassiber_envelope::{dict, get, redaction::redact_operational_text, s, truthy};
use kassiber_pyjson::{Key, PyStr, Value};
use regex::Regex;
use std::sync::OnceLock;

pub fn text(value: &str) -> String {
    static URL: OnceLock<Regex> = OnceLock::new();
    static HOST: OnceLock<Regex> = OnceLock::new();
    let url = URL.get_or_init(|| {
        Regex::new(r#"\b[a-zA-Z][a-zA-Z0-9+.-]*://(?:\[[^\]\s]+\][^\s,;)"'\]]*|[^\s,;)"'\]]+)"#)
            .unwrap()
    });
    let host =
        HOST.get_or_init(|| Regex::new(r#"(?i)\bhost\s*=\s*['"]?[^\s,;)'"]+['"]?"#).unwrap());
    let scrubbed = url.replace_all(value, |m: &regex::Captures<'_>| {
        let raw = m.get(0).unwrap().as_str();
        let trimmed = raw.trim_end_matches([':', '.', '!', '?']);
        format!("<backend-url>{}", &raw[trimmed.len()..])
    });
    redact_operational_text(&host.replace_all(&scrubbed, "host=<backend-host>"))
}

fn sync(value: &Value) -> Value {
    match value {
        Value::Dict(pairs) => {
            let mut result = Value::Dict(
                pairs
                    .iter()
                    .map(|(key, item)| {
                        if key == &Key::from("backend_url") {
                            (Key::from("has_backend_url"), Value::Bool(truthy(item)))
                        } else {
                            (key.clone(), sync(item))
                        }
                    })
                    .collect(),
            );
            if get(&result, "results").is_some() && get(&result, "ok").is_none() {
                let failed = error_rows(field(&result, "results"));
                super::put(&mut result, "ok", Value::Bool(!failed));
            }
            result
        }
        Value::List(items) => Value::List(items.iter().map(sync).collect()),
        Value::Str(v) => Value::Str(py_text(v)),
        _ => value.clone(),
    }
}

fn py_text(value: &PyStr) -> PyStr {
    if let Some(value) = value.as_str() {
        return text(value).into();
    }
    let present: std::collections::HashSet<u32> = value.code_points().collect();
    let mut available = (0xf0000..=0x10fffd).filter(|point| !present.contains(point));
    let mut originals = std::collections::HashMap::new();
    let mut masked = String::new();
    for point in value.code_points() {
        if let Some(c) = char::from_u32(point) {
            masked.push(c);
        } else {
            let Some(c) = available.next().and_then(char::from_u32) else {
                return "[redacted]".into();
            };
            originals.insert(c, point);
            masked.push(c);
        }
    }
    PyStr::from_code_points(
        text(&masked)
            .chars()
            .map(|c| originals.get(&c).copied().unwrap_or(c as u32)),
    )
    .unwrap_or_else(|_| "[redacted]".into())
}

pub fn error_rows(value: &Value) -> bool {
    matches!(value, Value::List(items) if items.iter().any(|row| {
        matches!(row, Value::Dict(_)) && or_text(field(row,"status")).to_lowercase() == "error"
    }))
}

pub fn payload(value: &Value) -> Value {
    sync(&redact(value))
}

pub fn journal_step_summary(enqueued: &[Value], completed: &[Value]) -> Value {
    let mut result = dict([
        ("status", s("skipped")),
        ("rebuilt", Value::Bool(false)),
        ("quarantines", Value::Null),
        ("auto_pair_applied", Value::from(0)),
        ("error", Value::Null),
    ]);
    let journal = |job: &&Value| field(job, "job_type") == &s("journal_refresh");
    let Some(job) = completed.iter().rev().find(journal) else {
        if enqueued
            .iter()
            .any(|job| field(job, "job_type") == &s("journal_refresh"))
        {
            super::put(&mut result, "status", s("deferred"));
        }
        return result;
    };
    let status = field(job, "status");
    if status == &s("done") {
        let data = field(job, "result");
        let pairs = field(data, "auto_pair");
        let rebuilt = get(data, "rebuilt").is_none_or(truthy);
        for (key, value) in [
            ("status", s(if rebuilt { "processed" } else { "current" })),
            ("rebuilt", Value::Bool(rebuilt)),
            (
                "quarantines",
                match field(data, "quarantined") {
                    value @ Value::Int(_) => value.clone(),
                    Value::Bool(value) => Value::from(i64::from(*value)),
                    _ => Value::Null,
                },
            ),
            (
                "auto_pair_applied",
                Value::from(super::integer(field(pairs, "applied")).unwrap_or(0)),
            ),
        ] {
            super::put(&mut result, key, value);
        }
        if truthy(field(pairs, "skipped")) {
            super::put(&mut result, "auto_pair_skipped", Value::Bool(true));
            let error = field(pairs, "error");
            super::put(
                &mut result,
                "error",
                dict([
                    (
                        "code",
                        if truthy(field(error, "code")) {
                            field(error, "code").clone()
                        } else {
                            s("auto_pair_failed")
                        },
                    ),
                    (
                        "message",
                        if truthy(field(error, "message")) {
                            field(error, "message").clone()
                        } else {
                            s("Automatic pairing was skipped; journals were still processed.")
                        },
                    ),
                ]),
            );
        }
    } else if [s("queued"), s("rate_limited")].contains(status) {
        super::put(&mut result, "status", s("deferred"));
    } else {
        super::put(
            &mut result,
            "status",
            s(if status == &s("cancelled") {
                "cancelled"
            } else {
                "failed"
            }),
        );
        let error = field(job, "error");
        super::put(
            &mut result,
            "error",
            dict([
                (
                    "code",
                    if truthy(field(error, "code")) {
                        field(error, "code").clone()
                    } else if truthy(status) {
                        status.clone()
                    } else {
                        s("journal_refresh_failed")
                    },
                ),
                (
                    "message",
                    if truthy(field(error, "message")) {
                        field(error, "message").clone()
                    } else {
                        s("Journal processing did not finish.")
                    },
                ),
            ]),
        );
    }
    result
}

pub fn results(jobs: &[Value]) -> Vec<Value> {
    jobs.iter()
        .filter_map(|job| {
            if ![
                "onchain_wallet_history",
                "btcpay_wallet_source",
                "btcpay_provenance",
            ]
            .contains(&or_text(field(job, "job_type")).as_str())
            {
                return None;
            }
            if field(job, "status") == &s("done") {
                return matches!(field(job, "result"), Value::Dict(_))
                    .then(|| field(job, "result").clone());
            }
            let error = field(job, "error");
            let choose = |values: &[&Value]| {
                values
                    .iter()
                    .find(|v| truthy(v))
                    .map_or(Value::Null, |v| (*v).clone())
            };
            Some(kassiber_envelope::dict([
                (
                    "wallet",
                    choose(&[
                        field(field(job, "payload"), "wallet_label"),
                        field(job, "source_label"),
                        &s("Source"),
                    ]),
                ),
                ("status", s("error")),
                (
                    "code",
                    choose(&[field(error, "code"), field(job, "status")]),
                ),
                (
                    "message",
                    choose(&[field(error, "message"), &s("Refresh did not finish.")]),
                ),
                ("hint", field(error, "hint").clone()),
                ("details", field(error, "details").clone()),
                ("retryable", Value::Bool(truthy(field(error, "retryable")))),
            ]))
        })
        .collect()
}
