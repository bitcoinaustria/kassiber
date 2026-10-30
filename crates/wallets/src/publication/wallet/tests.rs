use super::*;
use crate::publication::testbook::{load, snapshot, text};

struct Fixed {
    prefix: String,
    ids: u64,
    now: i64,
}

impl Sources for Fixed {
    fn id(&mut self) -> String {
        self.ids += 1;
        format!("{}-{:04}", self.prefix, self.ids)
    }
    fn now(&mut self) -> String {
        self.now += 1;
        kassiber_chain::format_unix_time(1_893_456_000 + self.now).unwrap()
    }
}

fn vectors() -> Json {
    serde_json::from_str(include_str!(
        "../../../tests/vectors/python_wallet_publication.json"
    ))
    .unwrap()
}

fn replay(case: &Json, fail: Option<&str>) -> (Value, Connection, Value) {
    let mut conn = load(&py(&case["before"]).unwrap());
    let before = snapshot(&conn);
    let mut sources = Fixed {
        prefix: case["prefix"].as_str().unwrap().into(),
        ids: 0,
        now: case["now"].as_i64().unwrap(),
    };
    let result = apply(
        &mut conn,
        &Request {
            profile_id: "p",
            wallet_id: "a",
            captured_config: case["captured_config"].as_str().unwrap(),
            fetch: &case["fetch"],
        },
        &mut sources,
        |stage| match fail {
            Some(target) if target == stage => Err(AppError::new("injected_failure", stage)),
            _ => Ok(()),
        },
    );
    let outcome = match result {
        Ok(mut results) => {
            for result in &mut results {
                if let Value::Dict(pairs) = result {
                    pairs.retain(
                        |(key, _)| !matches!(key, Key::Str(k) if k.as_str() == Some("elapsed_ms")),
                    );
                }
            }
            dict([("result", Value::List(results))])
        }
        Err(error) => dict([(
            "error",
            dict([("code", s(error.code)), ("message", s(error.message))]),
        )]),
    };
    (outcome, conn, before)
}

#[test]
fn publication_books_match_python_fixture_and_sequence_rounds() {
    let vectors = vectors();
    let cases = vectors["cases"].as_array().unwrap();
    assert!(cases.len() >= 60, "{}", cases.len());
    let mut published = 0;
    for case in cases.iter().filter(|c| c.get("before").is_some()) {
        let name = case["name"].as_str().unwrap();
        let (outcome, conn, _) = replay(case, None);
        let expected = py(&case["outcome"]).unwrap();
        if get(&expected, "error").is_some() {
            assert_eq!(
                get(get(&outcome, "error").expect(name), "code"),
                get(get(&expected, "error").unwrap(), "code"),
                "{name}"
            );
        } else {
            assert_eq!(text(&outcome), text(&expected), "{name} outcome");
            published += 1;
        }
        assert_eq!(
            text(&snapshot(&conn)),
            text(&py(&case["after"]).unwrap()),
            "{name} rows"
        );
    }
    assert!(published >= 60, "{published}");
}

#[test]
fn every_stage_failure_and_cancellation_restores_the_book() {
    let vectors = vectors();
    let mut checked = 0;
    for case in vectors["cases"].as_array().unwrap().iter().filter(|c| {
        c.get("before").is_some()
            && matches!(
                c["name"].as_str(),
                Some(
                    "receive-change/electrum/0"
                        | "receive-change/esplora/0"
                        | "reorg-drop/electrum/1"
                        | "replacement/esplora/1"
                )
            )
    }) {
        for stage in [
            "begin",
            "observer_persistence",
            "retractions",
            "transaction_insertion",
            "output_inventory",
            "derivation_coverage",
            "freshness_checkpoint",
            "commit",
        ] {
            let (outcome, conn, before) = replay(case, Some(stage));
            assert_eq!(
                get(get(&outcome, "error").unwrap(), "code"),
                Some(&s("injected_failure")),
                "{stage}"
            );
            assert_eq!(
                text(&snapshot(&conn)),
                text(&before),
                "{} {stage}",
                case["name"]
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 32);
}

#[test]
fn unsupported_shapes_and_stale_preparations_refuse_without_writes() {
    let vectors = vectors();
    let case = vectors["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "receive-change/electrum/0")
        .unwrap();
    for (sql, code) in [
        ("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('b','w','p','Other','descriptor','{}','fixed')", "native_fallback_required"),
        ("UPDATE wallets SET kind='lnd'", "native_fallback_required"),
        ("UPDATE profiles SET id=id", ""),
    ] {
        let mut conn = load(&py(&case["before"]).unwrap());
        conn.execute_batch(sql).unwrap();
        let before = snapshot(&conn);
        let mut sources = Fixed { prefix: "x".into(), ids: 0, now: 0 };
        let result = apply(
            &mut conn,
            &Request {
                profile_id: "p",
                wallet_id: "a",
                captured_config: case["captured_config"].as_str().unwrap(),
                fetch: &case["fetch"],
            },
            &mut sources,
            |_| Ok(()),
        );
        if code.is_empty() {
            assert!(result.is_ok());
            continue;
        }
        assert_eq!(result.unwrap_err().code, code, "{sql}");
        assert_eq!(text(&snapshot(&conn)), text(&before), "{sql}");
    }
    let mut conn = load(&py(&case["before"]).unwrap());
    let before = snapshot(&conn);
    let mut stale = case["fetch"].clone();
    stale["book_environment_id"] = Json::from("other-environment");
    for (fetch, captured, code) in [
        (&case["fetch"], "{}", "stale_context"),
        (
            &stale,
            case["captured_config"].as_str().unwrap(),
            "stale_context",
        ),
    ] {
        let mut sources = Fixed {
            prefix: "x".into(),
            ids: 0,
            now: 0,
        };
        let error = apply(
            &mut conn,
            &Request {
                profile_id: "p",
                wallet_id: "a",
                captured_config: captured,
                fetch,
            },
            &mut sources,
            |_| Ok(()),
        )
        .unwrap_err();
        assert_eq!(error.code, code);
        assert_eq!(text(&snapshot(&conn)), text(&before));
    }
}

#[test]
fn nested_publication_releases_only_its_savepoint() {
    let vectors = vectors();
    let case = vectors["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "receive-change/esplora/0")
        .unwrap();
    let mut conn = load(&py(&case["before"]).unwrap());
    conn.execute_batch("BEGIN").unwrap();
    let mut sources = Fixed {
        prefix: case["prefix"].as_str().unwrap().into(),
        ids: 0,
        now: 0,
    };
    apply(
        &mut conn,
        &Request {
            profile_id: "p",
            wallet_id: "a",
            captured_config: case["captured_config"].as_str().unwrap(),
            fetch: &case["fetch"],
        },
        &mut sources,
        |_| Ok(()),
    )
    .unwrap();
    assert!(!conn.is_autocommit(), "the caller's transaction stays open");
    conn.execute_batch("ROLLBACK").unwrap();
    assert_eq!(
        text(&snapshot(&conn)),
        text(&snapshot(&load(&py(&case["before"]).unwrap())))
    );
}
