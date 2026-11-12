//! Public CoinJoin shape rules, Python `privacy_hygiene._coinjoin_shape`.
//!
//! A shape is a collaboration boundary that suppresses ownership guesses; it
//! never proves participation and never grants ownership. Thresholds and
//! sources: "CoinJoin shape rules" in docs/reference/local-chain-analysis.md.
//! The analysis index, legacy hygiene and the transaction graph share this one
//! implementation; each reads the legs with its own ported value parser.

use kassiber_pyjson::BigInt;
use std::collections::{BTreeMap, BTreeSet};

pub const MIN_INPUTS: usize = 5;
pub const MIN_OUTPUTS: usize = 5;
pub const MIN_EQUAL_OUTPUTS: usize = 5;
pub const MAX_LEG_RATIO: usize = 5;
pub const LARGE_MIN_LEGS: usize = 20;
/// Public chain facts alone decide these, so a public observer may keep them.
pub const SOURCES: [&str; 3] = [
    "equal_output_coinjoin",
    "large_equal_output_coinjoin",
    "distinct_value_coinjoin",
];

/// One leg as the transaction itself carries it: the value (an input's
/// prevout value, never a wallet inventory amount) and the normalized script.
#[derive(Clone, Debug, Default)]
pub struct Leg {
    pub value: Option<BigInt>,
    pub script: Option<String>,
}

/// Legacy hygiene's `coinjoin_pattern` impact for a shape source.
pub fn impact(pattern: &str) -> Option<i64> {
    match pattern {
        "equal_output_coinjoin" => Some(18),
        "large_equal_output_coinjoin" => Some(20),
        "distinct_value_coinjoin" => Some(16),
        _ => None,
    }
}

/// The largest number of `outputs` sharing one positive value.
pub fn most_common(outputs: &[Leg]) -> usize {
    let mut counts = BTreeMap::<&BigInt, usize>::new();
    for value in outputs.iter().filter_map(|leg| leg.value.as_ref()) {
        if *value > BigInt::from(0) {
            *counts.entry(value).or_default() += 1;
        }
    }
    counts.into_values().max().unwrap_or(0)
}

/// Name the shape of a transaction with `input_count` input entries, of which
/// `inputs` are the readable ones, and its non-OP_RETURN `outputs`.
pub fn shape(input_count: usize, inputs: &[Leg], outputs: &[Leg]) -> Option<&'static str> {
    if input_count < MIN_INPUTS || outputs.len() < MIN_OUTPUTS {
        return None;
    }
    let top = most_common(outputs);
    if top >= MIN_EQUAL_OUTPUTS {
        if input_count >= LARGE_MIN_LEGS && outputs.len() >= LARGE_MIN_LEGS {
            return Some("large_equal_output_coinjoin");
        }
        if outputs.len() < MAX_LEG_RATIO * input_count {
            return Some("equal_output_coinjoin");
        }
    }
    if outputs.len() >= MAX_LEG_RATIO * input_count
        || input_count >= MAX_LEG_RATIO * outputs.len()
        || inputs.len() != input_count
        || top < 2
    {
        return None;
    }
    // An unavailable leg is neither repetition nor reuse: not evaluated.
    let legs = || inputs.iter().chain(outputs);
    if legs()
        .any(|leg| leg.script.is_none() || leg.value.as_ref().is_none_or(|v| *v < BigInt::from(0)))
    {
        return None;
    }
    let scripts: BTreeSet<_> = legs().map(|leg| &leg.script).collect();
    if scripts.len() != inputs.len() + outputs.len() {
        return None;
    }
    let distinct = |side: &[Leg]| {
        side.iter()
            .map(|leg| &leg.value)
            .collect::<BTreeSet<_>>()
            .len()
    };
    if 2 * (distinct(inputs) + distinct(outputs)) > input_count + outputs.len() {
        return None;
    }
    Some("distinct_value_coinjoin")
}
