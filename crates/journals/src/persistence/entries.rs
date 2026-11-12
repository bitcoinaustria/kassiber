use super::*;
use crate::storage::compare::{Prepared, Sources};
use std::cell::{OnceCell, RefCell};

pub(super) struct Plan<'a> {
    pub id: String,
    pub metadata: [ValueRef<'a>; 15],
    pub quantity: Sql,
    pub entry: &'a JournalEntryRow<'a>,
    pub calculation: Prepared<'a>,
}

pub(super) struct Row<'a, 'b> {
    pub plan: Plan<'a>,
    pub sources: &'b Sources<'a>,
    pub encoder: &'b RefCell<storage::CalculationEncoder<'a>>,
    pub bindings: OnceCell<[Sql; 12]>,
}

impl Row<'_, '_> {
    fn decimals(&self) -> [Option<&JournalDecimal>; 5] {
        let e = self.plan.entry;
        [
            Some(&e.fiat_value),
            Some(&e.unit_cost),
            e.cost_basis.as_ref(),
            e.proceeds.as_ref(),
            e.gain_loss.as_ref(),
        ]
    }
}

impl storage::BindingRow<29> for Row<'_, '_> {
    type Error = Refusal;

    fn cell(&self, index: usize) -> ValueRef<'_> {
        match index {
            0 => ValueRef::Text(self.plan.id.as_bytes()),
            1..=8 => self.plan.metadata[index - 1],
            9 => (&self.plan.quantity).into(),
            22..=28 => self.plan.metadata[index - 14],
            _ => (&self.bindings.get().expect("prepared changed entry")[index - 10]).into(),
        }
    }

    fn matches(&self, old: &rusqlite::Row<'_>, _: &[usize]) -> Result<bool, Refusal> {
        for index in [9, 8, 7, 3, 4, 5, 6, 1, 2, 22, 23, 24, 25, 26, 27, 0] {
            if old.get_ref(index)? != self.cell(index) {
                return Ok(false);
            }
        }
        for (index, decimal) in self.decimals().into_iter().enumerate() {
            let real = old.get_ref(index + 10)?;
            let exact = old.get_ref(index + 15)?;
            let Some(decimal) = decimal else {
                if real != ValueRef::Null || exact != ValueRef::Null {
                    return Ok(false);
                }
                continue;
            };
            let ValueRef::Text(bytes) = exact else {
                return Ok(false);
            };
            if !fixed_equal(decimal, bytes) {
                return Ok(false);
            }
            let value: f64 = std::str::from_utf8(bytes)
                .expect("matched ASCII decimal")
                .parse()
                .expect("matched fixed decimal");
            let ValueRef::Real(prior) = real else {
                return Ok(false);
            };
            if prior != value
                || (prior == 0.0 && prior.is_sign_negative() != value.is_sign_negative())
            {
                return Ok(false);
            }
        }
        Ok(self
            .plan
            .calculation
            .matches(self.sources, old.get_ref(20)?, old.get_ref(21)?))
    }

    fn prepare(&self) -> Result<(), Refusal> {
        if self.bindings.get().is_some() {
            return Ok(());
        }
        let e = self.plan.entry;
        let (legacy, compact) = match &self.plan.calculation {
            Prepared::Absent => (None, None),
            Prepared::Fallback(legacy, compact) => (legacy.clone(), compact.clone()),
            Prepared::Compact { .. } | Prepared::Direct(_) => self
                .encoder
                .borrow_mut()
                .encode(e.calculation.as_ref().expect("compact calculation"))?,
        };
        let (fiat, fiat_exact) = decimal_bindings(Some(&e.fiat_value))?;
        let (unit, unit_exact) = decimal_bindings(Some(&e.unit_cost))?;
        let (basis, basis_exact) = decimal_bindings(e.cost_basis.as_ref())?;
        let (proceeds, proceeds_exact) = decimal_bindings(e.proceeds.as_ref())?;
        let (gain, gain_exact) = decimal_bindings(e.gain_loss.as_ref())?;
        let bindings = [
            fiat,
            unit,
            basis,
            proceeds,
            gain,
            fiat_exact,
            unit_exact,
            basis_exact,
            proceeds_exact,
            gain_exact,
            opt(legacy),
            opt(compact),
        ];
        self.bindings
            .set(bindings)
            .unwrap_or_else(|_| unreachable!("single entry preparation"));
        Ok(())
    }
}

pub(super) fn validate(decimal: &JournalDecimal) -> Result<(), Refusal> {
    let d = &decimal.0;
    if (-i128::from(kassiber_pydecimal::MAX_FIXED_PADDING) - 1..=0)
        .contains(&i128::from(d.exponent()))
    {
        return Ok(());
    }
    let padding = if d.exponent() >= 0 {
        if d.is_zero() {
            0
        } else {
            i128::from(d.exponent())
        }
    } else {
        (-i128::from(d.adjusted()) - 1).max(0)
    };
    if padding > i128::from(kassiber_pydecimal::MAX_FIXED_PADDING) {
        return Err(kassiber_pydecimal::DecimalError::FormatTooLong.into());
    }
    Ok(())
}

fn fixed_equal(decimal: &JournalDecimal, raw: &[u8]) -> bool {
    fn compare(d: &kassiber_pydecimal::Decimal, raw: &[u8]) -> Option<bool> {
        let raw = if d.is_sign_negative() {
            raw.strip_prefix(b"-")?
        } else {
            raw
        };
        if d.is_zero() && d.exponent() >= 0 {
            return Some(raw == b"0");
        }
        let (left, right) = if d.exponent() >= 0 {
            let padding = usize::try_from(d.exponent()).ok()?;
            let end = raw.len().checked_sub(padding)?;
            let coefficient = &raw[..end];
            if coefficient.first().is_none_or(|&b| b == b'0')
                || !raw[end..].iter().all(|&b| b == b'0')
            {
                return Some(false);
            }
            (coefficient, &b""[..])
        } else {
            let scale = usize::try_from(-i128::from(d.exponent())).ok()?;
            let point = raw.len().checked_sub(scale.checked_add(1)?)?;
            if raw.get(point) != Some(&b'.') || point == 0 {
                return Some(false);
            }
            let left = &raw[..point];
            let right = &raw[point + 1..];
            if left == b"0" {
                let start = right
                    .iter()
                    .position(|&b| b != b'0')
                    .unwrap_or(right.len() - 1);
                (&right[start..], &b""[..])
            } else {
                if left[0] == b'0' {
                    return Some(false);
                }
                (left, right)
            }
        };
        Some(d.coefficient_matches_ascii(left, right))
    }
    compare(&decimal.0, raw).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row<'a, 'b>(
        entry: &'a JournalEntryRow,
        sources: &'b mut Sources<'a>,
        encoder: &'b RefCell<storage::CalculationEncoder<'a>>,
        clock: &'static [u8],
    ) -> Row<'a, 'b> {
        let mut metadata = [ValueRef::Text(b"metadata"); 15];
        metadata[1] = ValueRef::Text(b"p");
        metadata[5] = ValueRef::Text(b"time");
        metadata[14] = ValueRef::Text(clock);
        let calculation = sources
            .prepare(entry.calculation.as_ref(), &mut encoder.borrow_mut())
            .unwrap();
        Row {
            plan: Plan {
                id: "id".into(),
                metadata,
                quantity: Sql::Integer(42),
                entry,
                calculation,
            },
            sources,
            encoder,
            bindings: OnceCell::new(),
        }
    }

    #[derive(Default)]
    struct Writes(usize);
    impl storage::WriteObserver for Writes {
        fn wrote(&mut self, _: &'static str) -> rusqlite::Result<()> {
            self.0 += 1;
            Ok(())
        }
    }

    #[test]
    fn typed_retained_columns_force_real_delta_writes_and_unchanged_rows_prepare_no_bindings() {
        let cases: Value =
            serde_json::from_str(include_str!("../../tests/data/render.json")).unwrap();
        for case in array(&cases) {
            let Ok(response) = crate::boundary::ledger_response(&case["response"]) else {
                continue;
            };
            let Ok(state) = crate::boundary::build(
                &case["profile"],
                &case["custody"],
                array(&case["wallets"]),
                &response,
                String::new,
            ) else {
                continue;
            };
            let Some(entry) = state.entries.iter().find(|e| {
                e.calculation.is_some()
                    && [
                        Some(&e.fiat_value),
                        Some(&e.unit_cost),
                        e.cost_basis.as_ref(),
                        e.proceeds.as_ref(),
                        e.gain_loss.as_ref(),
                    ]
                    .into_iter()
                    .flatten()
                    .all(|d| !d.0.is_zero() || !d.0.is_sign_negative())
            }) else {
                continue;
            };
            let mut entry = entry.clone();
            entry.fiat_value = JournalDecimal(
                "123456789012345678901234567890123456789012345678901234567890E-32"
                    .parse()
                    .unwrap(),
            );
            let entry = &entry;
            for column in 0..29 {
                let conn = Connection::open_in_memory().unwrap();
                let columns = storage::ENTRY_COLUMNS
                    .iter()
                    .enumerate()
                    .map(|(index, name)| {
                        let kind = if (10..=14).contains(&index) {
                            "REAL"
                        } else if index == 0 {
                            "TEXT PRIMARY KEY"
                        } else {
                            ""
                        };
                        format!("{name} {kind}")
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                conn.execute_batch(&format!("CREATE TABLE journal_entries({columns})"))
                    .unwrap();
                let mut sources = Sources::default();
                let encoder = RefCell::new(storage::CalculationEncoder::default());
                let original = row(entry, &mut sources, &encoder, b"first");
                assert!(matches!(
                    original.plan.calculation,
                    Prepared::Compact { .. } | Prepared::Direct(_)
                ));
                storage::replace_entry_bindings(&conn, "p", &[original], &mut Writes::default())
                    .unwrap();
                let mut rebuilt = row(entry, &mut sources, &encoder, b"later");
                if column == 0 {
                    rebuilt.plan.id = "new-id".into();
                } else if column == 2 {
                    rebuilt.plan.metadata[1] = ValueRef::Blob(b"p");
                } else if column != 28 {
                    conn.execute_batch(&format!(
                        "UPDATE journal_entries SET {}=x'706f69736f6e'",
                        storage::ENTRY_COLUMNS[column]
                    ))
                    .unwrap();
                }
                let mut writes = Writes::default();
                storage::replace_entry_bindings(
                    &conn,
                    "p",
                    std::slice::from_ref(&rebuilt),
                    &mut writes,
                )
                .unwrap();
                assert_eq!(
                    writes.0,
                    if column == 28 { 0 } else { 2 },
                    "{}",
                    storage::ENTRY_COLUMNS[column]
                );
                assert_eq!(rebuilt.bindings.get().is_some(), column != 28);
                if column != 28 {
                    use storage::BindingRow;
                    let prepared = rebuilt.bindings.get().unwrap().clone();
                    rebuilt.prepare().unwrap();
                    assert_eq!(rebuilt.bindings.get().unwrap(), &prepared);
                }
                assert_eq!(
                    conn.query_row("SELECT COUNT(*) FROM journal_entries", [], |r| r
                        .get::<_, i64>(0))
                        .unwrap(),
                    1
                );
            }
            return;
        }
        panic!("missing typed calculation fixture");
    }

    #[test]
    fn typed_retained_comparison_checks_scalar_types_optional_pairs_and_real_values() {
        use storage::BindingRow;
        let cases: Value =
            serde_json::from_str(include_str!("../../tests/data/render.json")).unwrap();
        let responses: Vec<_> = array(&cases)
            .iter()
            .filter_map(|case| {
                crate::boundary::ledger_response(&case["response"])
                    .ok()
                    .map(|response| (case, response))
            })
            .collect();
        let entry = responses
            .iter()
            .find_map(|(case, response)| {
                let state = crate::boundary::build(
                    &case["profile"],
                    &case["custody"],
                    array(&case["wallets"]),
                    response,
                    String::new,
                )
                .ok()?;
                state.entries.into_iter().find(|e| e.calculation.is_some())
            })
            .expect("typed calculation fixture");
        let conn = Connection::open_in_memory().unwrap();
        let query = format!("SELECT {}", vec!["?"; 29].join(","));
        for optional in [false, true] {
            let mut entry = entry.clone();
            entry.fiat_value = JournalDecimal("1.25".parse().unwrap());
            entry.unit_cost = JournalDecimal("2.50".parse().unwrap());
            entry.cost_basis = optional.then(|| JournalDecimal("3.75".parse().unwrap()));
            entry.proceeds = optional.then(|| JournalDecimal("4.00".parse().unwrap()));
            entry.gain_loss = optional.then(|| JournalDecimal("5.25".parse().unwrap()));
            let mut sources = Sources::default();
            let encoder = RefCell::new(storage::CalculationEncoder::default());
            let binding = row(&entry, &mut sources, &encoder, b"clock");
            binding.prepare().unwrap();
            let original: [Sql; 29] =
                std::array::from_fn(|index| Sql::try_from(binding.cell(index)).unwrap());
            let matches = |cells: &[Sql; 29]| {
                conn.query_row(&query, params_from_iter(cells), |old| {
                    Ok(binding.matches(old, &[]).unwrap())
                })
                .unwrap()
            };
            assert!(matches(&original));
            for column in 0..29 {
                let mut replacements = vec![
                    Sql::Null,
                    Sql::Integer(7),
                    Sql::Real(98765.5),
                    t("different"),
                    Sql::Blob(b"different".to_vec()),
                ];
                if let Sql::Text(text) = &original[column] {
                    replacements.push(Sql::Blob(text.as_bytes().to_vec()));
                }
                if let Sql::Real(real) = &original[column] {
                    replacements.push(t(real.to_string()));
                    if real.fract() == 0.0 {
                        replacements.push(Sql::Integer(*real as i64));
                    }
                }
                for replacement in replacements {
                    if replacement == original[column] {
                        continue;
                    }
                    let mut cells = original.clone();
                    cells[column] = replacement;
                    assert_eq!(
                        matches(&cells),
                        column == 28,
                        "optional={optional} column={column}: {:?}",
                        cells[column]
                    );
                }
            }
        }
    }

    #[test]
    fn decimal_padding_validation_preserves_limits_without_formatting() {
        let limit = i128::from(kassiber_pydecimal::MAX_FIXED_PADDING);
        for coefficient in [
            "0",
            "-0",
            "1",
            "-1",
            "10",
            "12345678901234567890123456789012345678901234567890",
        ] {
            let digits = coefficient.trim_start_matches('-').len() as i128;
            for padding in [limit - 1, limit, limit + 1] {
                let exponent = -digits - padding;
                let decimal = JournalDecimal(format!("{coefficient}E{exponent}").parse().unwrap());
                assert_eq!(validate(&decimal).is_ok(), padding <= limit);
            }
            for exponent in [-1, 0, 1, limit, limit + 1] {
                let decimal = JournalDecimal(format!("{coefficient}E{exponent}").parse().unwrap());
                assert_eq!(
                    validate(&decimal).is_ok(),
                    decimal.0.is_zero() || exponent <= limit
                );
            }
        }
    }

    #[test]
    fn fixed_decimal_comparison_refuses_truncated_non_ascii_and_extreme_inputs() {
        for spelling in ["0", "-0", "1", "-1", "1E1", "1E-1", "0.00", "-0.00"] {
            let decimal = JournalDecimal(spelling.parse().unwrap());
            let fixed = decimal.0.to_fixed_string().unwrap();
            for raw in [
                &b""[..],
                &b"-"[..],
                &b"."[..],
                &b".1"[..],
                &b"1."[..],
                &b"1\xff"[..],
                &b"\xff"[..],
                &b"00.1"[..],
                &b"0.01\xff"[..],
            ] {
                assert!(!fixed_equal(&decimal, raw), "{spelling}: {raw:?}");
            }
            for length in 0..fixed.len() {
                assert!(!fixed_equal(&decimal, &fixed.as_bytes()[..length]));
            }
        }
        for spelling in ["1E999999999999999999", "1E-1999999999999999997"] {
            let decimal = JournalDecimal(spelling.parse().unwrap());
            for raw in [b"".as_slice(), b"0", b"1", b"0.1", b"1.0"] {
                assert!(!fixed_equal(&decimal, raw));
            }
            assert!(validate(&decimal).is_err());
        }
    }

    #[test]
    fn typed_decimal_comparison_retains_every_fixed_digit_sign_and_scale() {
        for coefficient in [
            "0",
            "-0",
            "1",
            "-1",
            "100",
            "-12345",
            "340282366920938463463374607431768211455",
            "340282366920938463463374607431768211456",
            "123456789012345678901234567890123456789012345678901234567890",
            "115792089237316195423570985008687907853269984665640564039457584007913129639935",
            "115792089237316195423570985008687907853269984665640564039457584007913129639936",
        ] {
            for exponent in [-100, -40, -32, -12, -9, -6, -1, 0, 1, 32, 65, 1000] {
                let decimal = JournalDecimal(format!("{coefficient}E{exponent}").parse().unwrap());
                validate(&decimal).unwrap();
                let fixed = decimal.0.to_fixed_string().unwrap();
                assert!(
                    fixed_equal(&decimal, fixed.as_bytes()),
                    "{coefficient}E{exponent}"
                );
                for altered in [
                    format!("+{fixed}"),
                    format!("0{fixed}"),
                    format!("{fixed}0"),
                    format!("{fixed} "),
                    decimal.0.to_string(),
                    fixed.replace('.', ":"),
                    fixed.trim_start_matches('-').to_owned(),
                    format!(
                        "{}0{}",
                        if decimal.0.is_sign_negative() {
                            "-"
                        } else {
                            ""
                        },
                        fixed.trim_start_matches('-')
                    ),
                ] {
                    assert_eq!(
                        fixed_equal(&decimal, altered.as_bytes()),
                        altered == fixed,
                        "{coefficient}E{exponent}: {altered}"
                    );
                }
                for index in fixed
                    .bytes()
                    .enumerate()
                    .filter_map(|(index, byte)| byte.is_ascii_digit().then_some(index))
                {
                    let mut changed = fixed.as_bytes().to_vec();
                    changed[index] = if changed[index] == b'9' { b'8' } else { b'9' };
                    assert!(!fixed_equal(&decimal, &changed));
                }
            }
        }
        assert!(!fixed_equal(&JournalDecimal("21".parse().unwrap()), b"1;"));
        assert!(!fixed_equal(
            &JournalDecimal(
                "100000000000000000000000000000000000000000000000000000000000000000000000000000"
                    .parse()
                    .unwrap()
            ),
            b"215792089237316195423570985008687907853269984665640564039457584007913129639936"
        ));
        let coefficient = "1234567890".repeat(100);
        for exponent in [-1100, -1000, -500, -1, 0, 10] {
            let decimal = JournalDecimal(format!("{coefficient}E{exponent}").parse().unwrap());
            let fixed = decimal.0.to_fixed_string().unwrap();
            assert!(fixed_equal(&decimal, fixed.as_bytes()));
            for index in [0, fixed.len() / 2, fixed.len() - 1] {
                let mut changed = fixed.as_bytes().to_vec();
                changed[index] = if changed[index] == b'9' { b'8' } else { b'9' };
                assert!(!fixed_equal(&decimal, &changed));
            }
        }
        for text in ["1E100000001", "1E-100000002"] {
            let decimal = JournalDecimal(text.parse().unwrap());
            assert!(validate(&decimal).is_err());
            assert!(decimal.0.to_fixed_string().is_err());
        }
    }
}
