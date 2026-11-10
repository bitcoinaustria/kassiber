//! Guarded driver for disposable plaintext test books. Replays one prepared
//! fetch, or independently prepares it against an explicitly supplied owned
//! fixture backend with an exact-action grant. No product routing.
use kassiber_wallets::publication::wallet::{apply, Request, Sources};
use serde_json::{json, Value};
use std::io::Read;

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

fn main() {
    let mut raw = String::new();
    std::io::stdin()
        .take(64 * 1024 * 1024)
        .read_to_string(&mut raw)
        .unwrap();
    let input: Value = serde_json::from_str(&raw).unwrap();
    let mut conn =
        kassiber_store::rusqlite::Connection::open(input["book"].as_str().unwrap()).unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
    let mut sources = Fixed {
        prefix: input["prefix"].as_str().unwrap_or("id").into(),
        ids: input["ids"].as_u64().unwrap_or(0),
        now: input["now"].as_i64().unwrap_or(0),
    };
    let fail = input["fail_stage"].as_str().map(str::to_owned);
    let (fetch, captured_config) = if input["native_backend"].is_object() {
        use kassiber_net::{Gate, Ledger, Offline};
        use kassiber_wallets::{
            fetch::{Fetch, FetchConsent},
            scan::Cancellation,
        };
        use std::time::{Duration, Instant};
        let gate = Gate::new(|| Offline::Online, Ledger::default());
        let action = Fetch::new(
            &conn,
            input["profile_id"].as_str().unwrap(),
            input["wallet_id"].as_str().unwrap(),
            &input["native_backend"],
            input["force_full"] == true,
            input["version"].as_str().unwrap(),
        )
        .unwrap();
        let consent = FetchConsent::explicit_trigger(&action);
        let fetch = action
            .run(
                &gate,
                action.scope(),
                Some(&consent),
                Instant::now() + Duration::from_secs(60),
                &Cancellation::default(),
                input["start_time"].as_u64().unwrap_or(1_700_000_000),
            )
            .unwrap();
        (fetch, action.captured_config().to_owned())
    } else {
        (
            input["fetch"].clone(),
            input["captured_config"].as_str().unwrap().to_owned(),
        )
    };
    let mut stages = vec![];
    let result = apply(
        &mut conn,
        &Request {
            profile_id: input["profile_id"].as_str().unwrap(),
            wallet_id: input["wallet_id"].as_str().unwrap(),
            captured_config: &captured_config,
            fetch: &fetch,
            btcpay: None,
        },
        &mut sources,
        |stage| {
            stages.push(stage.to_owned());
            if fail.as_deref() == Some(stage) {
                return Err(kassiber_envelope::AppError::new("injected_failure", stage));
            }
            Ok(())
        },
    );
    let text = |value: kassiber_pyjson::Value| {
        kassiber_pyjson::dumps(&value, &kassiber_pyjson::Options::new().sort_keys(true)).unwrap()
    };
    println!(
        "{}",
        match result {
            Ok(results) =>
                json!({"result": serde_json::from_str::<Value>(&text(kassiber_pyjson::Value::List(results))).unwrap(), "stages": stages, "fetch": fetch}),
            Err(e) =>
                json!({"error": {"code": e.code, "message": e.message, "details": e.details.map(|d| serde_json::from_str::<Value>(&text(d)).unwrap())}, "stages": stages}),
        }
    );
}
