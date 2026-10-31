use kassiber_chain::lightning::{LndReportClient, ProfitabilitySnapshot, ReportAction};
use kassiber_envelope::{dict, s};
use kassiber_net::{Consent, Gate, Ledger, Offline};
use kassiber_pyjson::Value;
use std::io::Read;
use std::time::{Duration, Instant};

fn projection(snapshot: ProfitabilitySnapshot) -> Value {
    dict([
        (
            "channels",
            Value::List(
                snapshot
                    .channels
                    .into_iter()
                    .map(|c| {
                        dict([
                            ("id", Value::Str(c.id)),
                            ("peer_alias", Value::Str(c.peer_alias)),
                            ("capacity_sat", Value::Int(c.capacity_sat)),
                            (
                                "earned_routing_sat",
                                c.earned_routing_sat.map(Value::Int).unwrap_or(Value::Null),
                            ),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "routing",
            snapshot
                .routing
                .map(|r| {
                    dict([
                        ("window_label", s(r.window_label)),
                        ("routing_revenue_sat", Value::Int(r.routing_revenue_sat)),
                        ("payment_cost_sat", Value::Int(r.payment_cost_sat)),
                        ("rebalance_cost_sat", Value::Int(r.rebalance_cost_sat)),
                        ("onchain_cost_sat", Value::Int(r.onchain_cost_sat)),
                        ("net_profit_sat", Value::Int(r.net_profit_sat)),
                        ("forward_count", Value::Int(r.forward_count.into())),
                        ("payment_count", Value::Int(r.payment_count.into())),
                        ("rebalance_count", Value::Int(r.rebalance_count.into())),
                    ])
                })
                .unwrap_or(Value::Null),
        ),
    ])
}

fn main() {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).unwrap();
    let input: serde_json::Value = serde_json::from_str(&input).unwrap();
    let offline = input["offline"].as_bool().unwrap_or(false);
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
    let result = LndReportClient::new(
        &input["backend"],
        gate.clone(),
        input["version"].as_str().unwrap_or("fixture"),
        ReportAction {
            book_id: "disposable-book".into(),
            profile_id: "profile".into(),
            connection_id: "wallet".into(),
            window_days: input["window"].as_u64().unwrap_or(30) as u16,
            export_file: None,
        },
    )
    .and_then(|client| {
        let consent = (input["consent"].as_bool() != Some(false))
            .then(|| Consent::explicit_trigger(client.request()));
        client.fetch(consent.as_ref(), Instant::now() + Duration::from_secs(5))
    });
    let result = match result {
        Ok(value) => dict([("result", projection(value))]),
        Err(error) => error.envelope(None),
    };
    let output = dict([
        ("response", result),
        ("attempts", Value::Int(gate.attempts().records.len().into())),
        (
            "records",
            Value::Int(gate.ledger().snapshot(0, 2000).records.len().into()),
        ),
    ]);
    println!(
        "{}",
        kassiber_pyjson::dumps(&output, &Default::default()).unwrap()
    );
}
