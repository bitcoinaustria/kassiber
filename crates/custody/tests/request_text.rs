//! The ledger request text (`ledger_request::request_json`) spells every
//! field the engine's rules country has, in the engine's order, and every
//! registry country's rules without loss. Its allowlist once lacked the
//! fields registry countries emit (`refused_receipts` and later), so a review
//! of a German, Swiss, Irish, British, Spanish or Japanese book panicked.
use kassiber_custody::ledger_request::{request_json, REFUSED_RECEIPT_FIELDS, RULES_FIELDS};
use kassiber_tax_core::ledger::request::{Country, Direction};
use kassiber_tax_core::model::{
    BelowMarket, FeeUnits, GiftLosses, Given, Holding, PoolScope, Received, RefusedReceipt,
    RulesSpec, SwapValues,
};
use serde_json::Value;

/// A rules country with every field away from its default, so each one is
/// serialized. A struct literal: a field the engine adds fails to compile
/// here until the test, and the request text, name it.
fn every_field() -> RulesSpec {
    RulesSpec {
        timezone: "Europe/Berlin".into(),
        holding: Holding::MoreThanOneYear,
        tax_year_start: Some("04-06".into()),
        pool_scope: Some(PoolScope::Wallet),
        fee_units: FeeUnits::DisposalBeforeSale,
        swap_values: SwapValues::Higher,
        gifts_given: Given::MarketValue,
        donations_given: Given::MarketValue,
        gifts_received: Received::Quarantine,
        gift_losses: GiftLosses::NotDeductible,
        refused_receipts: vec![
            RefusedReceipt {
                direction: Direction::Inbound,
                kinds: vec!["airdrop".into()],
                code: "airdrop_classification_required".into(),
                detail: "Classify it".into(),
            },
            RefusedReceipt {
                direction: Direction::Outbound,
                kinds: vec!["lost".into(), "theft".into()],
                code: "non_sale_disposal_kind".into(),
                detail: "Review it".into(),
            },
        ],
        zero_cost_kinds: vec!["fork".into()],
        below_market: BelowMarket::SeventyPercent,
    }
}

fn keys(text: &str, value: &Value) -> Vec<String> {
    // The object's keys in text order: serde_json's map sorts them.
    let Value::Object(map) = value else {
        panic!("object")
    };
    let mut keys: Vec<(usize, String)> = map
        .keys()
        .map(|key| (text.find(&format!("\"{key}\":")).unwrap(), key.clone()))
        .collect();
    keys.sort();
    keys.into_iter().map(|(_, key)| key).collect()
}

#[test]
fn request_text_spells_every_engine_rules_field_in_engine_order() {
    let engine = serde_json::to_string(&Country::Rules(every_field())).unwrap();
    let value: Value = serde_json::from_str(&engine).unwrap();
    assert_eq!(request_json(&value), engine);
    assert_eq!(keys(&engine, &value), RULES_FIELDS);
    let receipt = &value["refused_receipts"][1];
    let text = serde_json::to_string(&every_field().refused_receipts[1]).unwrap();
    assert_eq!(keys(&text, receipt), REFUSED_RECEIPT_FIELDS);
}

#[test]
fn request_text_keeps_every_registry_country_policy_exactly() {
    let mut rules = 0;
    for country in kassiber_tax_countries::COUNTRIES {
        let Some(Ok(engine)) = country.engine_country() else {
            continue;
        };
        rules += 1;
        let text = request_json(&engine);
        let read: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(read, engine, "{}", country.code);
        let typed: Country =
            serde_json::from_str(&text).unwrap_or_else(|error| panic!("{}: {error}", country.code));
        assert_eq!(
            text,
            serde_json::to_string(&typed).unwrap(),
            "{}: the engine's own order",
            country.code
        );
    }
    // Germany, Switzerland, Ireland, the United Kingdom, Spain and Japan.
    assert!(rules >= 6, "{rules} rules countries");
}
