use kassiber_envelope::{dict, success};
use kassiber_net::{Gate, Ledger, Offline};
use kassiber_pyjson::Value as Py;
use kassiber_store::rusqlite::Connection;
use kassiber_wallets::{
    freshness::graph::{fixture_followup as followup, Grant},
    publication::wallet::Sources,
    scan::Cancellation,
};
use serde_json::Value;
use std::io::Read;

struct Fixed {
    prefix: String,
    ids: u64,
}
impl Sources for Fixed {
    fn id(&mut self) -> String {
        self.ids += 1;
        format!("{}-{:04}", self.prefix, self.ids)
    }
    fn now(&mut self) -> String {
        "2030-01-01T00:00:01Z".into()
    }
}

fn main() {
    let mut raw = String::new();
    std::io::stdin()
        .take(1024 * 1024)
        .read_to_string(&mut raw)
        .unwrap();
    let input: Value = serde_json::from_str(&raw).unwrap();
    let mut conn = Connection::open(input["book"].as_str().unwrap()).unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    let gate = Gate::new(|| Offline::Online, Ledger::default());
    let result = (|| {
        let grant = Grant::fixture(
            &conn,
            input["job"].as_str().unwrap_or("sync"),
            "a",
            &input["backend"],
        )?;
        if input["revoke"] == true {
            grant.revoke();
        }
        if let Some(sql) = input["before_sql"].as_str() {
            conn.execute_batch(sql).unwrap();
        }
        let stop = Cancellation::default();
        if input["stop"] == true {
            stop.cancel();
        }
        followup(
            &mut conn,
            grant,
            &gate,
            &stop,
            &mut Fixed {
                prefix: input["prefix"].as_str().unwrap_or("graph").into(),
                ids: 0,
            },
            "0.22.78",
            |_, _| Ok(input["backend"].clone()),
        )
    })();
    let envelope = match result {
        Ok(result) => success("graph.followup", result.unwrap_or(Py::Null)),
        Err(error) => error.envelope(None),
    };
    println!(
        "{}",
        kassiber_pyjson::dumps(&dict([("envelope", envelope)]), &Default::default()).unwrap()
    );
    if input["revoke"] == true
        || input["stop"] == true
        || input["before_sql"].is_string()
        || std::env::var("KASSIBER_NO_EGRESS").as_deref() == Ok("1")
    {
        assert!(gate.attempts().records.is_empty());
        assert!(gate.ledger().snapshot(0, 100).records.is_empty());
    }
}
