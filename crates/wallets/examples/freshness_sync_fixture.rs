use kassiber_envelope::{dict, success};
use kassiber_net::{Gate, Ledger, Offline};
use kassiber_pyjson::{Options, Value as Py};
use kassiber_store::rusqlite::Connection;
use kassiber_wallets::{
    freshness::{
        execution,
        native::Consent,
        planning::{Plan, Request, Scope},
    },
    publication::wallet::Sources,
    scan::Cancellation,
};
use serde_json::Value;
use std::{
    io::Read,
    time::{Duration, Instant},
};

struct Fixed {
    prefix: String,
    id: u64,
}
impl Sources for Fixed {
    fn id(&mut self) -> String {
        self.id += 1;
        format!("{}-{:04}", self.prefix, self.id)
    }
    fn now(&mut self) -> String {
        "2030-01-01T00:00:01Z".into()
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
    conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    let offline = input["offline"] == true;
    let gate = Gate::new(
        move || {
            if offline {
                Offline::Offline
            } else {
                Offline::Online
            }
        },
        Ledger::default(),
    );
    let mut events = vec![];
    let result = (|| {
        let request = Request {
            scope: input["workspace_id"]
                .as_str()
                .map(|v| Scope::Workspace(v.into()))
                .unwrap_or(Scope::ActiveProfile),
            wallet: input["wallet"].as_str().map(str::to_owned),
            rates: Some(false),
            journals: Some(false),
            force_full: input["force_full"] == true,
        };
        let plan = Plan::new(
            &conn,
            &request,
            input["version"].as_str().unwrap(),
            |_, _| Ok(input["backend"].clone()),
        )?;
        let proof = Consent::explicit_trigger(&plan);
        let plan = if input["foreign"] == true {
            drop(plan);
            Plan::new(
                &conn,
                &request,
                input["version"].as_str().unwrap(),
                |_, _| Ok(input["backend"].clone()),
            )?
        } else {
            plan
        };
        if let Some(sql) = input["before_execution_sql"].as_str() {
            conn.execute_batch(sql).unwrap();
        }
        if input["revoke"] == true {
            proof.revoke();
        }
        let cancel = Cancellation::default();
        if input["cancel"] == true {
            cancel.cancel();
        }
        plan.execute_native(
            &mut conn,
            &execution::Options {
                run: input["run"] != false,
                limit: input["limit"].as_i64(),
            },
            &mut Fixed {
                prefix: input["prefix"].as_str().unwrap_or("job").into(),
                id: 0,
            },
            &mut |value| {
                events.push(value.clone());
                Ok(())
            },
            &gate,
            (input["consent"] != false).then_some(&proof),
            &cancel,
            Instant::now() + Duration::from_secs(60),
            input["start_time"].as_u64().unwrap_or(1_700_000_000),
            input["version"].as_str().unwrap(),
            |_, _| {
                Ok(input
                    .get("changed_backend")
                    .unwrap_or(&input["backend"])
                    .clone())
            },
        )
    })();
    let envelope = match result {
        Ok(value) => success("freshness.sync", value),
        Err(error) => error.envelope(None),
    };
    println!(
        "{}",
        kassiber_pyjson::dumps(
            &dict([("envelope", envelope), ("events", Py::List(events))]),
            &Options::new().sort_keys(true)
        )
        .unwrap()
    );
    if input["consent"] == false
        || input["cancel"] == true
        || input["revoke"] == true
        || input["foreign"] == true
        || input.get("before_execution_sql").is_some()
        || input.get("changed_backend").is_some()
        || offline
        || std::env::var("KASSIBER_NO_EGRESS").as_deref() == Ok("1")
    {
        assert!(gate.attempts().records.is_empty());
        assert!(gate.ledger().snapshot(0, 100).records.is_empty());
    }
}
