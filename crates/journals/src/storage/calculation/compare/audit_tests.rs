use super::*;
use serde_json::json;

fn source() -> Source<'static> {
    Source {
        provenance: Arc::new(SourceProvenance {
            transaction_id: json!("tx\"\n🔥"),
            engine_event_id: "event".into(),
            occurred_at: "2024-01-01T00:00:00Z".into(),
            pricing: PricingFacts::Owned(Box::new(std::array::from_fn(|i| {
                if i % 2 == 0 {
                    Value::Null
                } else {
                    json!(format!("price-{i}"))
                }
            }))),
        }),
        asset: "BTC".into(),
        spot_price_exact: "12.000".into(),
        fiat_fee_exact: "0.010".into(),
        crypto_fee_msat: json!(-7).into(),
        crypto_fee_msat_exact: "-7".into(),
    }
}

fn calculation(count: usize) -> Calculation<'static> {
    let source = source();
    Calculation {
        schema_version: 1,
        engine: "engine\"\n🔥",
        method: "fifo\\\t".into(),
        fragments: (0..count)
            .map(|i| Fragment {
                quantity_msat: json!(-(i as i64)).into(),
                quantity_msat_exact: format!("{}", -(i as i64)),
                cost_basis_exact: format!("{i}.000").into(),
                proceeds_exact: format!("{i}.100").into(),
                gain_loss_exact: "é\r\u{0008}\u{000c}".into(),
                expenses_exact: None,
                unit_basis_override_exact: (i % 2 == 0).then(|| "1.2300".into()),
                basis_authority: "acquired_lot",
                acquisition_basis_exact: (i % 3 == 0).then(|| "2.000".into()),
                acquisition_quantity_msat: (i % 2 == 0).then(|| json!(31).into()),
                acquisition_quantity_msat_exact: (i % 2 == 0).then(|| "31".into()),
                event: source.clone().into(),
                lot: (i % 3 != 0).then(|| {
                    let mut lot = source.clone();
                    lot.provenance = Arc::new((*source.provenance).clone());
                    lot.into()
                }),
            })
            .collect(),
        loss_deductible: None,
        valuation: None,
    }
}

fn cell(value: &Option<String>) -> ValueRef<'_> {
    value
        .as_ref()
        .map_or(ValueRef::Null, |v| ValueRef::Text(v.as_bytes()))
}

fn assert_comparison(calculation: &Calculation) {
    let mut sources = Sources::default();
    let prepared = sources
        .prepare(Some(calculation), &mut Encoder::default())
        .unwrap();
    let encoded =
        crate::storage::encode_calculation(&serde_json::to_value(calculation).unwrap()).unwrap();
    assert!(prepared.matches(&sources, cell(&encoded.0), cell(&encoded.1)));
    for invalid in [
        ValueRef::Integer(0),
        ValueRef::Real(0.0),
        ValueRef::Blob(b"[]"),
    ] {
        assert!(!prepared.matches(&sources, invalid, cell(&encoded.1)));
        assert!(!prepared.matches(&sources, cell(&encoded.0), invalid));
    }
    assert!(!prepared.matches(&sources, ValueRef::Text(b"null"), cell(&encoded.1)));
    assert!(!prepared.matches(&sources, cell(&encoded.0), ValueRef::Text(b"null")));
    let (text, legacy) = if let Some(text) = &encoded.0 {
        (text, true)
    } else {
        (encoded.1.as_ref().unwrap(), false)
    };
    let matches = |bytes: &[u8]| {
        if legacy {
            prepared.matches(&sources, ValueRef::Text(bytes), cell(&encoded.1))
        } else {
            prepared.matches(&sources, cell(&encoded.0), ValueRef::Text(bytes))
        }
    };
    for end in 0..text.len() {
        assert!(
            !matches(&text.as_bytes()[..end]),
            "accepted truncation {end}: {text}"
        );
    }
    let mut bytes = text.as_bytes().to_vec();
    for index in 0..bytes.len() {
        bytes[index] ^= 0x80;
        assert!(!matches(&bytes), "accepted changed byte {index}: {text}");
        bytes[index] ^= 0x80;
    }
    bytes.push(b' ');
    assert!(!matches(&bytes));
}

#[test]
fn comparison_dispatch_and_refusals_preserve_exact_storage_bytes() {
    let mut sources = Sources::default();
    let absent = sources.prepare(None, &mut Encoder::default()).unwrap();
    assert!(matches!(absent, Prepared::Absent));
    assert!(absent.matches(&sources, ValueRef::Null, ValueRef::Null));
    assert!(!absent.matches(&sources, ValueRef::Text(b""), ValueRef::Null));
    assert!(!absent.matches(&sources, ValueRef::Null, ValueRef::Text(b"")));
    assert!(absent.consume(&sources, &mut Cursor(b"")).is_none());
    for count in [0, 1, 2, 8, 9, 20] {
        let mut calculation = calculation(count);
        if count == 8 || count == 20 {
            for (index, fragment) in calculation.fragments.iter_mut().enumerate() {
                Arc::make_mut(&mut Arc::make_mut(&mut fragment.event).provenance).engine_event_id =
                    format!("event-{index}").into();
                let mut lot = source();
                Arc::make_mut(&mut lot.provenance).engine_event_id = format!("lot-{index}").into();
                fragment.lot = Some(lot.into());
            }
        }
        let mut sources = Sources::default();
        let prepared = sources.prepare(Some(&calculation), &mut Encoder::default());
        let prepared = prepared.unwrap();
        assert_eq!(matches!(prepared, Prepared::Direct(_)), count <= 8);
        assert_eq!(matches!(prepared, Prepared::Compact { .. }), count > 8);
        assert_comparison(&calculation);
    }
    for count in [1, 9] {
        for field in 0..10 {
            let mut calculation = calculation(count);
            let fragment = &mut calculation.fragments[0];
            match field {
                0 => calculation.schema_version = 2,
                1 => fragment.quantity_msat_exact = "-999".into(),
                2 => fragment.quantity_msat = json!(1.5).into(),
                3 => fragment.acquisition_quantity_msat = None,
                4 => fragment.acquisition_quantity_msat_exact = None,
                5 => fragment.acquisition_quantity_msat_exact = Some("9".into()),
                6 => {
                    Arc::make_mut(&mut Arc::make_mut(&mut fragment.event).provenance)
                        .transaction_id = json!(true)
                }
                7 => Arc::make_mut(&mut fragment.event).crypto_fee_msat_exact = "1".into(),
                8 => {
                    Arc::make_mut(&mut Arc::make_mut(&mut fragment.event).provenance).pricing[6] =
                        json!({})
                }
                9 => {
                    fragment.lot = Some(source().into());
                    Arc::make_mut(&mut Arc::make_mut(fragment.lot.as_mut().unwrap()).provenance)
                        .transaction_id = json!([]);
                }
                _ => unreachable!(),
            }
            let mut sources = Sources::default();
            let prepared = sources
                .prepare(Some(&calculation), &mut Encoder::default())
                .unwrap();
            assert!(
                matches!(prepared, Prepared::Fallback(..)),
                "field {field}, count {count}"
            );
            assert_comparison(&calculation);
        }
    }
}

#[test]
fn comparison_source_dictionary_retains_every_field_and_cache_identity() {
    for field in 0..19 {
        for count in [2, 9] {
            let mut calculation = calculation(count);
            let source = Arc::make_mut(&mut calculation.fragments[1].event);
            match field {
                0 => Arc::make_mut(&mut source.provenance).transaction_id = Value::Null,
                1 => Arc::make_mut(&mut source.provenance)
                    .engine_event_id
                    .to_mut()
                    .push('x'),
                2 => Arc::make_mut(&mut source.provenance).occurred_at.push('x'),
                3 => source.asset.to_mut().push('x'),
                4 => source.spot_price_exact.to_mut().push('0'),
                5 => source.fiat_fee_exact.to_mut().push('0'),
                6 => {
                    source.crypto_fee_msat = json!(0).into();
                    source.crypto_fee_msat_exact = "0".into();
                }
                _ => Arc::make_mut(&mut source.provenance).pricing[field - 7] = json!("changed"),
            }
            assert_comparison(&calculation);
            let mut sources = Sources::default();
            let original = sources.source(&calculation.fragments[0].event).unwrap();
            assert_eq!(
                sources.source(&calculation.fragments[0].event),
                Some(original)
            );
            let changed = sources.source(&calculation.fragments[1].event).unwrap();
            assert!(sources.shapes[original] != sources.shapes[changed]);
            let prepared = sources.compact(&calculation).unwrap();
            let Prepared::Compact {
                sources: dictionary,
                links,
                ..
            } = prepared
            else {
                panic!()
            };
            assert_eq!(dictionary.len(), 2);
            assert_eq!(links[0], [Some(0), None]);
            assert_eq!(links[1], [Some(1), Some(0)]);
        }
    }
    let mut invalid = source();
    invalid.crypto_fee_msat_exact = "invalid".into();
    let mut sources = Sources::default();
    assert_eq!(sources.source(&invalid), None);
    assert_eq!(sources.source(&invalid), None);
    assert_eq!(sources.cache.len(), 1);
    assert!(sources.shapes.is_empty());
}

#[test]
fn comparison_scalars_refuse_noncanonical_tokens() {
    for scalar in [
        Scalar::Null,
        Scalar::quoted("x"),
        Scalar::Integer("7"),
        Scalar::Small(7),
    ] {
        let token = match scalar {
            Scalar::Null => b"null".as_slice(),
            Scalar::Text(..) => b"\"x\"",
            Scalar::Integer(_) | Scalar::Small(_) => b"7",
        };
        let mut cursor = Cursor(token);
        assert!(scalar.consume(&mut cursor).is_some());
        assert!(cursor.0.is_empty());
        assert!(scalar.consume(&mut Cursor(b"wrong")).is_none());
    }
    let mut cursor = Cursor(b"value tail");
    assert!(cursor.write_str("value").is_ok());
    assert_eq!(cursor.0, b" tail");
    assert!(cursor.write_str("wrong").is_err());
    assert!(Scalar::text(&json!(0)).is_none());
    assert!(Cursor(b"").scalar(&json!(true)).is_none());
    let zero = Value::Number(serde_json::Number::from_string_unchecked("-0".into()));
    assert_eq!(zero.as_number().unwrap().as_str(), "-0");
    assert!(Scalar::integer(&zero.clone().into())
        .consume(&mut Cursor(b"0"))
        .is_some());
    let mut negative = source();
    negative.crypto_fee_msat = zero.into();
    negative.crypto_fee_msat_exact = "-0".into();
    let mut positive = negative.clone();
    positive.crypto_fee_msat = json!(0).into();
    positive.crypto_fee_msat_exact = "0".into();
    assert!(source_equal(&negative, &positive));
    assert!(source_equal(&positive, &negative));
    for plain in [false, true] {
        assert!(Cursor(b"\"plain\"").quoted_known("plain", plain).is_some());
        assert!(Cursor(b"\"different\"")
            .quoted_known("plain", plain)
            .is_none());
    }
    assert!(Cursor(b"null").optional(None).is_some());
    assert!(Cursor(b"\"x\"").optional(Some("x")).is_some());
    assert!(Cursor(b"null").optional(Some("x")).is_none());
    assert!(Cursor(b"\"x\"").optional(None).is_none());
}

#[test]
fn a_rules_countrys_entry_facts_keep_the_json_form() {
    // Spain's gift loss and Japan's 70 % valuation are stored beside the
    // fragments, so the compact tuple (which has no slot for them) is not
    // used, as for a fragment's expenses; every other entry keeps its bytes.
    let mut calculation = calculation(2);
    calculation.loss_deductible = Some(false);
    calculation.valuation = Some("seventy_percent");
    let encoded =
        crate::storage::encode_calculation(&serde_json::to_value(&calculation).unwrap()).unwrap();
    assert!(encoded.1.is_none());
    let json = encoded.0.clone().unwrap();
    for key in ["loss_deductible", "seventy_percent"] {
        assert!(json.contains(key), "{json}");
    }
    assert_eq!(
        crate::storage::encode_typed_calculation(&calculation).unwrap(),
        encoded
    );
    assert_comparison(&calculation);
    let plain = self::calculation(2);
    assert!(plain.plain() && !calculation.plain());
    let only_valuation = Calculation {
        loss_deductible: None,
        ..calculation.clone()
    };
    assert!(!only_valuation.plain());
    assert!(crate::storage::encode_typed_calculation(&only_valuation)
        .unwrap()
        .1
        .is_none());
}
