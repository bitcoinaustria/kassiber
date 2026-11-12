use super::{exact_integer_text as dynamic_exact_integer_text, integer, text};
use crate::{model::*, Refusal};
use kassiber_pyjson::{dumps_str_into, Options};
use serde_json::Value;
use std::collections::HashMap;
use std::fmt::Write;
use std::sync::Arc;
pub(crate) mod compare;

#[derive(Hash, PartialEq, Eq)]
struct SourceKey<'a> {
    provenance: *const SourceProvenance<'a>,
    asset: &'a str,
    spot: &'a str,
    fiat_fee: &'a str,
    crypto_fee: &'a JournalMsat,
    crypto_fee_exact: &'a str,
}

#[derive(Default)]
pub(crate) struct Encoder<'a> {
    sources: HashMap<SourceKey<'a>, Arc<str>>,
}
impl<'a> Encoder<'a> {
    fn source(&mut self, value: &'a Source, options: &Options) -> Option<Arc<str>> {
        let key = SourceKey {
            provenance: Arc::as_ptr(&value.provenance),
            asset: &value.asset,
            spot: &value.spot_price_exact,
            fiat_fee: &value.fiat_fee_exact,
            crypto_fee: &value.crypto_fee_msat,
            crypto_fee_exact: &value.crypto_fee_msat_exact,
        };
        if let Some(encoded) = self.sources.get(&key) {
            return Some(encoded.clone());
        }
        let encoded: Arc<str> = source(value, options)?.into();
        self.sources.insert(key, encoded.clone());
        Some(encoded)
    }

    pub(crate) fn encode(
        &mut self,
        calculation: &'a Calculation,
    ) -> Result<(Option<String>, Option<String>), Refusal> {
        if let Some(compact) = compact(calculation, self) {
            Ok((None, Some(compact)))
        } else {
            let value = serde_json::to_value(calculation)
                .map_err(|e| Refusal::new("ValueError", e.to_string()))?;
            super::encode_calculation(&value)
        }
    }
}

fn scalar(value: &Value, out: &mut String, options: &Options) -> Option<()> {
    match value {
        Value::Null => out.push_str("null"),
        Value::String(s) => dumps_str_into(s, options, out),
        Value::Number(n) if integer(value) => {
            let n = n.as_str();
            out.push_str(if n == "-0" { "0" } else { n });
        }
        _ => return None,
    }
    Some(())
}
fn msat(value: &JournalMsat, out: &mut String, options: &Options) -> Option<()> {
    match value {
        JournalMsat::Small(value) => write!(out, "{value}").ok(),
        JournalMsat::Wide(value) => scalar(value, out, options),
    }
}
fn exact_integer_text(value: &JournalMsat, exact: Option<&str>) -> bool {
    match value {
        JournalMsat::Small(value) => exact == Some(value.to_string().as_str()),
        JournalMsat::Wide(value) => dynamic_exact_integer_text(value, exact),
    }
}
fn optional(value: &Option<impl AsRef<str>>, out: &mut String, options: &Options) {
    if let Some(value) = value {
        dumps_str_into(value.as_ref(), options, out);
    } else {
        out.push_str("null");
    }
}
fn source(source: &Source, options: &Options) -> Option<String> {
    if !text(&source.provenance.transaction_id)
        || !exact_integer_text(&source.crypto_fee_msat, Some(&source.crypto_fee_msat_exact))
        || source.provenance.pricing.iter().any(|v| !text(v))
    {
        return None;
    }
    let mut out = String::with_capacity(512);
    out.push('[');
    scalar(&source.provenance.transaction_id, &mut out, options)?;
    for value in [
        source.provenance.engine_event_id.as_ref(),
        &source.provenance.occurred_at,
        &source.asset,
        &source.spot_price_exact,
        &source.fiat_fee_exact,
    ] {
        out.push(',');
        dumps_str_into(value, options, &mut out);
    }
    out.push(',');
    msat(&source.crypto_fee_msat, &mut out, options)?;
    for index in [0, 5, 1, 2, 9, 10, 11, 8, 7, 4, 3, 6] {
        out.push(',');
        scalar(&source.provenance.pricing[index], &mut out, options)?;
    }
    out.push(']');
    Some(out)
}
fn compact<'a>(calculation: &'a Calculation, encoder: &mut Encoder<'a>) -> Option<String> {
    if calculation.schema_version != 1 {
        return None;
    }
    // A rules country's entry facts have no slot in the compact tuple.
    if !calculation.plain() {
        return None;
    }
    let options = Options::new().compact();
    let mut sources = Vec::<Arc<str>>::new();
    let mut positions = HashMap::new();
    let mut fragments = String::with_capacity(calculation.fragments.len() * 256);
    fragments.push('[');
    for (index, fragment) in calculation.fragments.iter().enumerate() {
        // The compact tuple has no expenses slot.
        if fragment.expenses_exact.is_some()
            || !exact_integer_text(&fragment.quantity_msat, Some(&fragment.quantity_msat_exact))
        {
            return None;
        }
        match (
            &fragment.acquisition_quantity_msat,
            &fragment.acquisition_quantity_msat_exact,
        ) {
            (None, None) => {}
            (Some(quantity), Some(exact)) if exact_integer_text(quantity, Some(exact)) => {}
            _ => return None,
        }
        if index > 0 {
            fragments.push(',');
        }
        fragments.push('[');
        msat(&fragment.quantity_msat, &mut fragments, &options)?;
        for value in [
            &fragment.cost_basis_exact,
            &fragment.proceeds_exact,
            &fragment.gain_loss_exact,
        ] {
            fragments.push(',');
            dumps_str_into(value, &options, &mut fragments);
        }
        fragments.push(',');
        optional(
            &fragment.unit_basis_override_exact,
            &mut fragments,
            &options,
        );
        fragments.push(',');
        dumps_str_into(fragment.basis_authority, &options, &mut fragments);
        fragments.push(',');
        optional(&fragment.acquisition_basis_exact, &mut fragments, &options);
        fragments.push(',');
        if let Some(quantity) = &fragment.acquisition_quantity_msat {
            msat(quantity, &mut fragments, &options)?;
        } else {
            fragments.push_str("null");
        }
        for value in [Some(&fragment.event), fragment.lot.as_ref()] {
            fragments.push(',');
            if let Some(value) = value {
                let encoded = encoder.source(value, &options)?;
                let position = *positions.entry(encoded.clone()).or_insert_with(|| {
                    sources.push(encoded);
                    sources.len() - 1
                });
                write!(fragments, "{position}").ok()?;
            } else {
                fragments.push_str("null");
            }
        }
        fragments.push(']');
    }
    fragments.push(']');
    let mut out = String::with_capacity(
        fragments.len() + sources.iter().map(|s| s.len()).sum::<usize>() + 64,
    );
    out.push_str("[2,");
    dumps_str_into(calculation.engine, &options, &mut out);
    out.push(',');
    dumps_str_into(&calculation.method, &options, &mut out);
    out.push_str(",[");
    for (index, source) in sources.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(source);
    }
    out.push_str("],");
    out.push_str(&fragments);
    out.push(']');
    Some(out)
}
#[cfg(test)]
pub(crate) fn encode(
    calculation: &Calculation,
) -> Result<(Option<String>, Option<String>), Refusal> {
    Encoder::default().encode(calculation)
}

#[cfg(test)]
mod audit_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn typed_scalar_encoding_preserves_tokens_and_refuses_noncompact_values() {
        let options = Options::new().compact();
        for (value, expected) in [
            (Value::Null, "null"),
            (json!("\"\n🔥"), "\"\\\"\\n\\ud83d\\udd25\""),
            (json!(i64::MIN), "-9223372036854775808"),
            (json!(u64::MAX), "18446744073709551615"),
            (
                Value::Number(serde_json::Number::from_string_unchecked("-0".into())),
                "0",
            ),
        ] {
            let mut out = String::from("prefix");
            assert_eq!(scalar(&value, &mut out, &options), Some(()));
            assert_eq!(out, format!("prefix{expected}"));
        }
        for value in [json!(true), json!(1.0), json!([]), json!({})] {
            let mut out = String::from("prefix");
            assert_eq!(scalar(&value, &mut out, &options), None);
            assert_eq!(out, "prefix");
        }
        let mut out = String::new();
        optional(&None::<String>, &mut out, &options);
        optional(&Some(String::from("\n\"")), &mut out, &options);
        assert_eq!(out, "null\"\\n\\\"\"");
    }
}
