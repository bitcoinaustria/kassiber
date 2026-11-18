//! The shared filter and record shape, on synthetic events.
use super::*;
use registry::EXCHANGES;

static EVERY_ASSET: Descriptor = Descriptor {
    reads: Reads::AllAssets,
    skipped: Some(&["fiat_only", "other", "internal"]),
    ..kraken::EXCHANGE
};

fn number(text: &str) -> PriceNumber {
    PriceNumber::parse(text).unwrap()
}

/// Every exchange the backends accept has one entry, and no two share a kind.
#[test]
fn every_exchange_kind_has_one_descriptor() {
    let kinds: Vec<&str> = EXCHANGES.iter().map(|exchange| exchange.kind).collect();
    assert_eq!(kinds, KINDS);
    for exchange in EXCHANGES {
        assert_eq!(
            registry::by_kind(exchange.kind).unwrap().provider,
            exchange.provider
        );
    }
    assert!(registry::by_kind("bitcoin").is_none());
}

/// Bitcoin is kept in every scope; without it an event is counted and left
/// out, as `skipped` in the `bitcoin` scope of an exchange that reads every
/// asset, as `unread` from an exchange that reads only Bitcoin.
#[test]
fn the_scope_filters_events_before_their_type_is_read() {
    for (exchange, scope) in [
        (&EVERY_ASSET, Scope::Bitcoin),
        (&EVERY_ASSET, Scope::Crypto),
        (&kraken::EXCHANGE, Scope::Bitcoin),
        (&kraken::EXCHANGE, Scope::Crypto),
    ] {
        let mut mapper = Mapper::new(exchange, scope, "W");
        assert!(mapper.admit(&["BTC", "EUR"]));
        assert!(mapper.admit(&["LBTC", "ETH"]));
        assert!(!mapper.admit(&["EUR", "USD"]));
        assert!(!mapper.admit(&["", ""]));
        let keeps_eth = exchange.reads == Reads::AllAssets && scope == Scope::Crypto;
        assert_eq!(mapper.admit(&["ETH", "EUR", "ETH"]), keeps_eth);
        let envelope = kassiber_pyjson::dumps(
            &mapper.finish().tally.envelope(scope, "W"),
            &Default::default(),
        )
        .unwrap();
        let counted = match (exchange.reads, scope) {
            (Reads::BitcoinOnly, _) => r#""unread": {"ETH": 1}"#,
            (_, Scope::Bitcoin) => r#""skipped": {"ETH": 1}"#,
            _ => r#""skipped": {}"#,
        };
        assert!(envelope.contains(counted), "{envelope}");
    }
}

/// The exchange's own `skipped` counts: fiat-only, other-asset and internal
/// events.
#[test]
fn left_out_events_are_counted_by_reason() {
    let mut mapper = Mapper::new(&EVERY_ASSET, Scope::Bitcoin, "W");
    mapper.admit(&["EUR"]);
    mapper.admit(&["ETH"]);
    mapper.leave(false);
    mapper.leave(true);
    mapper.internal();
    let skipped = mapper.finish().skipped.unwrap();
    assert_eq!(
        skipped,
        dict([
            ("fiat_only", Py::from(2)),
            ("other", Py::from(2)),
            ("internal", Py::from(1)),
        ])
    );
    assert!(Mapper::new(&binance::EXCHANGE, Scope::Bitcoin, "W")
        .finish()
        .skipped
        .is_none());
}

/// An event's one leg is the record's asset and direction; a trade is a buy
/// when the leg arrives.
#[test]
fn an_event_maps_to_one_record() {
    let mut mapper = Mapper::new(&kraken::EXCHANGE, Scope::Bitcoin, "W");
    let sell = Event {
        occurred_at: Some("2024-01-02T03:04:05Z".into()),
        fee: number("0.0001"),
        pricing: Some(Pricing {
            fiat_currency: "EUR".into(),
            fiat_value: number("400"),
            fiat_rate: None,
            method: "kraken_api",
            external_ref: "T1".into(),
        }),
        ..Event::one(
            "kraken:L1".into(),
            Kind::Trade,
            false,
            "BTC".into(),
            number("-0.01"),
        )
    };
    mapper.push(sell).unwrap();
    let deposit = Event::one(
        "kraken:L2".into(),
        Kind::Movement("deposit"),
        true,
        "BTC".into(),
        number("0.5"),
    );
    mapper.push(deposit).unwrap();
    let records = mapper.finish().records;
    let field = |record: &Py, key: &str| kassiber_envelope::py_str(at(record, key));
    assert_eq!(field(&records[0], "kind"), "sell");
    assert_eq!(field(&records[0], "direction"), "outbound");
    assert_eq!(field(&records[0], "amount"), "0.01");
    assert_eq!(field(&records[0], "fee"), "0.0001");
    assert_eq!(field(&records[0], "fiat_rate"), "4.00E+4");
    assert_eq!(field(&records[0], "pricing_external_ref"), "T1");
    assert_eq!(field(&records[1], "kind"), "deposit");
    assert_eq!(field(&records[1], "direction"), "inbound");
    assert_eq!(field(&records[1], "description"), "Kraken deposit");
}
