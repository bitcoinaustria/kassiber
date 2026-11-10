use kassiber_envelope::{build_error_envelope, success};
use kassiber_net::{Gate, Ledger, Offline};
use kassiber_pyjson::Value as Py;
use kassiber_store::rusqlite::Connection;
use kassiber_wallets::{
    publication::wallet::Sources,
    scan::Cancellation,
    sync::{Request, Sync, SyncConsent},
};
use serde_json::Value;
use std::{
    io::Read,
    time::{Duration, Instant},
};

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
    let mut conn = Connection::open(input["book"].as_str().unwrap()).unwrap();
    if let Some(key) = input["key"].as_str() {
        // Fixed test passphrase of a disposable SQLCipher book.
        conn.pragma_update(None, "key", key).unwrap();
    }
    conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    let gate = Gate::new(|| Offline::Online, Ledger::default());
    let result = (|| {
        let action = Sync::new(
            &conn,
            &Request {
                profile_id: input["profile_id"].as_str().unwrap(),
                wallet: input["wallet"].as_str(),
                all: input["all"] == true,
                force_full: input["force_full"] == true,
                version: input["version"].as_str().unwrap(),
            },
            |_, _| Ok(input["backend"].clone()),
        )?;
        let proof = SyncConsent::explicit_trigger(&action);
        if input["revoke"] == true {
            proof.revoke();
        }
        let cancel = Cancellation::default();
        if input["cancel"] == true {
            cancel.cancel();
        }
        action.run(
            &mut conn,
            &gate,
            (input["consent"] != false).then_some(&proof),
            &cancel,
            Instant::now() + Duration::from_secs(60),
            input["start_time"].as_u64().unwrap_or(1_700_000_000),
            &mut Fixed {
                prefix: input["prefix"].as_str().unwrap_or("id").into(),
                ids: 0,
                now: input["now"].as_i64().unwrap_or(0),
            },
            |_, _| Ok(input["backend"].clone()),
            None,
        )
    })();
    let envelope = match result {
        Ok(results) => success("wallets.sync", Py::List(results)),
        Err(e) => build_error_envelope(
            &e.code,
            &e.message,
            e.details,
            e.hint.as_deref(),
            e.retryable,
            None,
        ),
    };
    println!(
        "{}",
        kassiber_pyjson::dumps(&envelope, &kassiber_pyjson::Options::new().sort_keys(true))
            .unwrap()
    );
    if input["consent"] == false
        || input["cancel"] == true
        || input["revoke"] == true
        || std::env::var("KASSIBER_NO_EGRESS").as_deref() == Ok("1")
    {
        assert!(gate.attempts().records.is_empty());
        assert!(gate.ledger().snapshot(0, 100).records.is_empty());
    }
}
