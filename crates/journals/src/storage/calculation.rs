use super::{exact_integer_text, integer, text};
use crate::{model::*, Refusal};
use kassiber_pyjson::{dumps_str_into, Options};
use serde_json::Value;
use std::collections::HashMap;
use std::fmt::Write;

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
fn optional(value: &Option<String>, out: &mut String, options: &Options) {
    if let Some(value) = value {
        dumps_str_into(value, options, out);
    } else {
        out.push_str("null");
    }
}
fn source(source: &Source, options: &Options) -> Option<String> {
    if !text(&source.provenance.transaction_id)
        || !exact_integer_text(&source.crypto_fee_msat, Some(&source.crypto_fee_msat_exact))
        || source.provenance.pricing.0.iter().any(|v| !text(v))
    {
        return None;
    }
    let mut out = String::with_capacity(512);
    out.push('[');
    scalar(&source.provenance.transaction_id, &mut out, options)?;
    for value in [
        source.provenance.engine_event_id.as_str(),
        &source.provenance.occurred_at,
        &source.asset,
        &source.spot_price_exact,
        &source.fiat_fee_exact,
    ] {
        out.push(',');
        dumps_str_into(value, options, &mut out);
    }
    out.push(',');
    scalar(&source.crypto_fee_msat, &mut out, options)?;
    for index in [0, 5, 1, 2, 9, 10, 11, 8, 7, 4, 3, 6] {
        out.push(',');
        scalar(&source.provenance.pricing.0[index], &mut out, options)?;
    }
    out.push(']');
    Some(out)
}
fn compact(calculation: &Calculation) -> Option<String> {
    if calculation.schema_version != 1 {
        return None;
    }
    let options = Options::new().compact();
    let mut sources = Vec::<String>::new();
    let mut positions = HashMap::new();
    let mut fragments = String::with_capacity(calculation.fragments.len() * 256);
    fragments.push('[');
    for (index, fragment) in calculation.fragments.iter().enumerate() {
        if !exact_integer_text(&fragment.quantity_msat, Some(&fragment.quantity_msat_exact)) {
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
        scalar(&fragment.quantity_msat, &mut fragments, &options)?;
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
            scalar(quantity, &mut fragments, &options)?;
        } else {
            fragments.push_str("null");
        }
        for value in [Some(&fragment.event), fragment.lot.as_ref()] {
            fragments.push(',');
            if let Some(value) = value {
                let encoded = source(value, &options)?;
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
        fragments.len() + sources.iter().map(String::len).sum::<usize>() + 64,
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
pub(crate) fn encode(
    calculation: &Calculation,
) -> Result<(Option<String>, Option<String>), Refusal> {
    if let Some(compact) = compact(calculation) {
        Ok((None, Some(compact)))
    } else {
        let value = serde_json::to_value(calculation)
            .map_err(|e| Refusal::new("ValueError", e.to_string()))?;
        super::encode_calculation(&value)
    }
}
