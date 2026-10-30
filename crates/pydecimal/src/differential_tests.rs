//! The fixed-width fast paths against the `BigUint` arithmetic they bypass.
//!
//! An operand whose coefficient is stored as `Coeff::Big`, even one that fits
//! `u128`, takes the `BigUint` path of every operation. Each case runs an
//! operation on the canonical operands and on those forced copies and
//! requires the same representation or the same error. Operands cluster
//! around the fixed widths' edges (38 and 39 digits, `u128::MAX`, `10^k`
//! and its neighbours) and the exponent limits, and every rounding mode
//! runs at precisions on both sides of 38 digits.
//! `KASSIBER_DECIMAL_DIFFERENTIAL_CASES` sets the number of cases.

use super::*;

/// An operation's name and its fast and `BigUint` results.
type Check = (
    &'static str,
    Result<Decimal, DecimalError>,
    Result<Decimal, DecimalError>,
);

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.below(items.len() as u64) as usize]
    }
}

const ROUNDINGS: [Rounding; 8] = [
    Rounding::HalfEven,
    Rounding::HalfUp,
    Rounding::HalfDown,
    Rounding::Down,
    Rounding::Up,
    Rounding::Ceiling,
    Rounding::Floor,
    Rounding::ZeroFiveUp,
];

const PRECISIONS: [u32; 18] = [
    1, 2, 3, 9, 16, 19, 28, 31, 32, 32, 32, 33, 34, 37, 38, 39, 40, 60,
];

fn digits_value(rng: &mut Rng, count: usize) -> BigUint {
    if count == 0 {
        return BigUint::zero();
    }
    let mut digits: Vec<u8> = (0..count).map(|_| rng.below(10) as u8).collect();
    digits[0] = 1 + rng.below(9) as u8;
    BigUint::from_radix_be(&digits, 10).expect("digits")
}

fn coefficient(rng: &mut Rng) -> BigUint {
    let ten = |k: usize| num_traits::pow(BigUint::from(10u32), k);
    match rng.below(16) {
        0 => BigUint::zero(),
        1 => ten(rng.below(46) as usize),
        2 => ten(1 + rng.below(45) as usize) - 1u32,
        3 => ten(rng.below(45) as usize) * 5u32,
        4 => BigUint::from(u128::MAX) - BigUint::from(rng.below(3)),
        5 => BigUint::from(u128::MAX) + BigUint::from(1 + rng.below(3)),
        6 => ten(rng.pick(&[31, 32, 33, 37, 38, 39])) - 1u32,
        7 => BigUint::from(rng.below(10)),
        8..=11 => {
            let count = rng.below(21) as usize;
            digits_value(rng, count)
        }
        12..=14 => {
            let count = 21 + rng.below(18) as usize;
            digits_value(rng, count)
        }
        _ => {
            let count = 39 + rng.below(40) as usize;
            digits_value(rng, count)
        }
    }
}

fn exponent(rng: &mut Rng) -> i64 {
    match rng.below(20) {
        0 => EMIN - 60 + rng.below(80) as i64,
        1 => EMAX - 60 + rng.below(60) as i64,
        2 => -(rng.below(200) as i64),
        3 => rng.below(200) as i64,
        _ => rng.below(81) as i64 - 40,
    }
}

fn operand(rng: &mut Rng) -> Decimal {
    let negative = rng.below(2) == 1;
    let coeff = coefficient(rng);
    let exp = exponent(rng);
    Decimal::from_parts(negative, coeff, exp).unwrap_or_else(|_| Decimal::zero())
}

/// The same value with its coefficient stored as `Coeff::Big`, so every
/// operation on it takes the `BigUint` path. Zero stays canonical: the
/// zero checks read a zero `Coeff::Small`.
fn forced(value: &Decimal) -> Decimal {
    if value.is_zero() {
        return value.clone();
    }
    Decimal {
        coeff: Coeff::Big(value.coeff.big().into_owned()),
        ..value.clone()
    }
}

fn same(fast: &Result<Decimal, DecimalError>, slow: &Result<Decimal, DecimalError>) -> bool {
    match (fast, slow) {
        (Ok(a), Ok(b)) => a.same_representation(b),
        (Err(a), Err(b)) => a == b,
        _ => false,
    }
}

fn show(result: &Result<Decimal, DecimalError>) -> String {
    match result {
        Ok(value) => value.to_canonical(),
        Err(error) => format!("{error:?}"),
    }
}

#[test]
fn fast_paths_match_the_bignum_paths() {
    let cases: usize = std::env::var("KASSIBER_DECIMAL_DIFFERENTIAL_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60_000);
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    let mut failures = Vec::new();
    for case in 0..cases {
        let ctx = Context::new(rng.pick(&PRECISIONS), rng.pick(&ROUNDINGS));
        let (a, b) = (operand(&mut rng), operand(&mut rng));
        let (fa, fb) = (forced(&a), forced(&b));
        let quantum = match rng.below(4) {
            0 => RP2_COMPARE_EXPONENT,
            1 => a.exponent() + rng.below(41) as i64 - 20,
            _ => exponent(&mut rng),
        };
        let checks: [Check; 9] = [
            ("add", a.add(&b, &ctx), fa.add(&fb, &ctx)),
            ("sub", a.sub(&b, &ctx), fa.sub(&fb, &ctx)),
            ("mul", a.mul(&b, &ctx), fa.mul(&fb, &ctx)),
            ("div", a.div(&b, &ctx), fa.div(&fb, &ctx)),
            (
                "quantize",
                a.quantize(quantum, &ctx),
                fa.quantize(quantum, &ctx),
            ),
            ("plus", a.plus(&ctx), fa.plus(&ctx)),
            ("neg", a.neg(&ctx), fa.neg(&ctx)),
            ("abs", a.abs(&ctx), fa.abs(&ctx)),
            ("normalize", a.normalize(&ctx), fa.normalize(&ctx)),
        ];
        for (name, fast, slow) in &checks {
            if !same(fast, slow) {
                failures.push(format!(
                    "case {case} {name} {} {} at {}:{:?}: fast {} slow {}",
                    a.to_canonical(),
                    b.to_canonical(),
                    ctx.prec,
                    ctx.rounding,
                    show(fast),
                    show(slow)
                ));
            }
        }
        if a.cmp(&b) != fa.cmp(&fb) {
            failures.push(format!("case {case} cmp {a:?} {b:?}"));
        }
        let rp2 = Context::RP2;
        if a.rp2_lt(&b, &rp2) != fa.rp2_lt(&fb, &rp2) {
            failures.push(format!("case {case} rp2_lt {a:?} {b:?}"));
        }
        if a.to_string() != fa.to_string() {
            failures.push(format!("case {case} text {a:?}"));
        }
        // Fixed-point text pads to the exponent; keep it short.
        if a.exponent().abs() < 1_000 {
            let places = rng.below(40) as u32;
            let rounding = rng.pick(&ROUNDINGS);
            if a.to_fixed_string() != fa.to_fixed_string()
                || a.to_fixed_places(places, rounding) != fa.to_fixed_places(places, rounding)
            {
                failures.push(format!("case {case} fixed {a:?} {places} {rounding:?}"));
            }
        }
        if a.to_canonical() != fa.to_canonical() {
            failures.push(format!("case {case} canonical {a:?}"));
        }
        match Decimal::from_canonical(&a.to_canonical()) {
            Ok(back) if back.same_representation(&a) => {}
            other => failures.push(format!("case {case} round trip {a:?} {other:?}")),
        }
        if failures.len() > 20 {
            break;
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
