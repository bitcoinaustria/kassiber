use super::{exact_integer_text, text, Encoder, SourceKey};
use crate::{model::*, Refusal};
use kassiber_store::rusqlite::types::ValueRef;
use serde_json::Value;
use std::collections::HashMap;
use std::fmt::Write;
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Scalar<'a> {
    Null,
    Text(&'a str, bool),
    Integer(&'a str),
    Small(i64),
}
impl<'a> Scalar<'a> {
    fn text(value: &'a Value) -> Option<Self> {
        match value {
            Value::Null => Some(Self::Null),
            Value::String(s) => Some(Self::quoted(s)),
            _ => None,
        }
    }
    fn quoted(s: &'a str) -> Self {
        Self::Text(
            s,
            s.bytes()
                .all(|b| matches!(b, 0x20..=0x7e) && !matches!(b, b'"' | b'\\')),
        )
    }
    fn integer(value: &'a JournalMsat) -> Self {
        match value {
            JournalMsat::Small(n) => Self::Small(*n),
            JournalMsat::Wide(value) => {
                let n = value.as_number().expect("validated integer").as_str();
                Self::Integer(if n == "-0" { "0" } else { n })
            }
        }
    }
    fn consume(self, cursor: &mut Cursor<'_>) -> Option<()> {
        match self {
            Self::Null => cursor.take(b"null"),
            Self::Text(s, plain) => cursor.quoted_known(s, plain),
            Self::Integer(s) => cursor.take(s.as_bytes()),
            Self::Small(n) => write!(cursor, "{n}").ok(),
        }
    }
}

#[derive(Default)]
pub(crate) struct Sources<'a> {
    cache: HashMap<SourceKey<'a>, Option<usize>>,
    shapes: Vec<[Scalar<'a>; 19]>,
}

pub(crate) enum Prepared<'a> {
    Absent,
    Direct(&'a Calculation<'a>),
    Compact {
        calculation: &'a Calculation<'a>,
        sources: Vec<usize>,
        links: Vec<[Option<usize>; 2]>,
    },
    Fallback(Option<String>, Option<String>),
}

impl<'a> Sources<'a> {
    fn source(&mut self, value: &'a Source) -> Option<usize> {
        let key = SourceKey {
            provenance: Arc::as_ptr(&value.provenance),
            asset: &value.asset,
            spot: &value.spot_price_exact,
            fiat_fee: &value.fiat_fee_exact,
            crypto_fee: &value.crypto_fee_msat,
            crypto_fee_exact: &value.crypto_fee_msat_exact,
        };
        if let Some(id) = self.cache.get(&key) {
            return *id;
        }
        let id = self.shape(value).map(|shape| {
            let id = self.shapes.len();
            self.shapes.push(shape);
            id
        });
        self.cache.insert(key, id);
        id
    }

    fn shape(&self, source: &'a Source) -> Option<[Scalar<'a>; 19]> {
        if !text(&source.provenance.transaction_id)
            || !exact_integer_text(&source.crypto_fee_msat, Some(&source.crypto_fee_msat_exact))
            || source.provenance.pricing.iter().any(|v| !text(v))
        {
            return None;
        }
        let mut shape = [Scalar::Null; 19];
        shape[0] = Scalar::text(&source.provenance.transaction_id)?;
        for (index, value) in [
            source.provenance.engine_event_id.as_ref(),
            &source.provenance.occurred_at,
            &source.asset,
            &source.spot_price_exact,
            &source.fiat_fee_exact,
        ]
        .into_iter()
        .enumerate()
        {
            shape[index + 1] = Scalar::quoted(value);
        }
        shape[6] = Scalar::integer(&source.crypto_fee_msat);
        for (index, pricing) in [0, 5, 1, 2, 9, 10, 11, 8, 7, 4, 3, 6]
            .into_iter()
            .enumerate()
        {
            shape[index + 7] = Scalar::text(&source.provenance.pricing[pricing])?;
        }
        Some(shape)
    }

    pub(crate) fn prepare(
        &mut self,
        calculation: Option<&'a Calculation<'a>>,
        encoder: &mut Encoder<'a>,
    ) -> Result<Prepared<'a>, Refusal> {
        let Some(calculation) = calculation else {
            return Ok(Prepared::Absent);
        };
        if calculation.plain() {
            if calculation.fragments.len() <= 8 && valid_direct(calculation) {
                return Ok(Prepared::Direct(calculation));
            }
            if let Some(prepared) = self.compact(calculation) {
                return Ok(prepared);
            }
        }
        let (legacy, compact) = encoder.encode(calculation)?;
        Ok(Prepared::Fallback(legacy, compact))
    }

    fn compact(&mut self, calculation: &'a Calculation<'a>) -> Option<Prepared<'a>> {
        if calculation.schema_version != 1 {
            return None;
        }
        let mut links = Vec::with_capacity(calculation.fragments.len());
        for fragment in &calculation.fragments {
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
            links.push([
                Some(self.source(&fragment.event)?),
                match &fragment.lot {
                    Some(lot) => Some(self.source(lot)?),
                    None => None,
                },
            ]);
        }
        let mut sources = Vec::new();
        if links.len() <= 8 {
            for link in &mut links {
                for id in link.iter_mut().flatten() {
                    let position = sources.iter().position(|prior: &usize| {
                        prior == id || self.shapes[*prior] == self.shapes[*id]
                    });
                    *id = position.unwrap_or_else(|| {
                        sources.push(*id);
                        sources.len() - 1
                    });
                }
            }
        } else {
            let mut positions = HashMap::new();
            for link in &mut links {
                for id in link.iter_mut().flatten() {
                    let position = *positions.entry(&self.shapes[*id]).or_insert_with(|| {
                        sources.push(*id);
                        sources.len() - 1
                    });
                    *id = position;
                }
            }
        }
        Some(Prepared::Compact {
            calculation,
            sources,
            links,
        })
    }
}

impl Prepared<'_> {
    pub(crate) fn matches(
        &self,
        sources: &Sources<'_>,
        legacy: ValueRef<'_>,
        compact: ValueRef<'_>,
    ) -> bool {
        match self {
            Self::Absent => legacy == ValueRef::Null && compact == ValueRef::Null,
            Self::Fallback(a, b) => {
                fn cell(v: &Option<String>) -> ValueRef<'_> {
                    v.as_ref()
                        .map_or(ValueRef::Null, |s| ValueRef::Text(s.as_bytes()))
                }
                legacy == cell(a) && compact == cell(b)
            }
            Self::Direct(calculation) => {
                if legacy != ValueRef::Null {
                    return false;
                }
                let ValueRef::Text(bytes) = compact else {
                    return false;
                };
                let mut cursor = Cursor(bytes);
                consume_direct(calculation, &mut cursor).is_some() && cursor.0.is_empty()
            }
            Self::Compact { .. } => {
                if legacy != ValueRef::Null {
                    return false;
                }
                let ValueRef::Text(bytes) = compact else {
                    return false;
                };
                let mut cursor = Cursor(bytes);
                self.consume(sources, &mut cursor).is_some() && cursor.0.is_empty()
            }
        }
    }

    fn consume(&self, dictionary: &Sources<'_>, cursor: &mut Cursor<'_>) -> Option<()> {
        let Self::Compact {
            calculation,
            sources,
            links,
        } = self
        else {
            return None;
        };
        cursor.take(b"[2,")?;
        cursor.quoted(calculation.engine)?;
        cursor.take(b",")?;
        cursor.quoted(&calculation.method)?;
        cursor.take(b",[")?;
        for (index, id) in sources.iter().enumerate() {
            if index > 0 {
                cursor.take(b",")?;
            }
            cursor.take(b"[")?;
            for (index, scalar) in dictionary.shapes[*id].iter().enumerate() {
                if index > 0 {
                    cursor.take(b",")?;
                }
                scalar.consume(cursor)?;
            }
            cursor.take(b"]")?;
        }
        cursor.take(b"],[")?;
        for (index, (fragment, link)) in calculation.fragments.iter().zip(links).enumerate() {
            if index > 0 {
                cursor.take(b",")?;
            }
            cursor.take(b"[")?;
            Scalar::integer(&fragment.quantity_msat).consume(cursor)?;
            for value in [
                &fragment.cost_basis_exact,
                &fragment.proceeds_exact,
                &fragment.gain_loss_exact,
            ] {
                cursor.take(b",")?;
                cursor.quoted(value)?;
            }
            cursor.take(b",")?;
            cursor.optional(fragment.unit_basis_override_exact.as_deref())?;
            cursor.take(b",")?;
            cursor.quoted(fragment.basis_authority)?;
            cursor.take(b",")?;
            cursor.optional(fragment.acquisition_basis_exact.as_deref())?;
            cursor.take(b",")?;
            match &fragment.acquisition_quantity_msat {
                Some(value) => Scalar::integer(value).consume(cursor)?,
                None => cursor.take(b"null")?,
            }
            for position in link {
                cursor.take(b",")?;
                match position {
                    Some(position) => write!(cursor, "{position}").ok()?,
                    None => cursor.take(b"null")?,
                }
            }
            cursor.take(b"]")?;
        }
        cursor.take(b"]]")
    }
}

fn valid_source(source: &Source) -> bool {
    text(&source.provenance.transaction_id)
        && exact_integer_text(&source.crypto_fee_msat, Some(&source.crypto_fee_msat_exact))
        && source.provenance.pricing.iter().all(text)
}

fn valid_direct(calculation: &Calculation) -> bool {
    calculation.schema_version == 1
        && calculation.fragments.iter().all(|fragment| {
            fragment.expenses_exact.is_none()
                && exact_integer_text(&fragment.quantity_msat, Some(&fragment.quantity_msat_exact))
                && match (
                    &fragment.acquisition_quantity_msat,
                    &fragment.acquisition_quantity_msat_exact,
                ) {
                    (None, None) => true,
                    (Some(quantity), Some(exact)) => exact_integer_text(quantity, Some(exact)),
                    _ => false,
                }
                && valid_source(&fragment.event)
                && fragment
                    .lot
                    .as_ref()
                    .is_none_or(|source| valid_source(source))
        })
}

fn source_equal(a: &Source, b: &Source) -> bool {
    fn integer_equal(a: &JournalMsat, b: &JournalMsat) -> bool {
        match (a.as_i64(), b.as_i64()) {
            (Some(a), Some(b)) => a == b,
            _ => a.to_string() == b.to_string(),
        }
    }
    a.asset == b.asset
        && a.spot_price_exact == b.spot_price_exact
        && a.fiat_fee_exact == b.fiat_fee_exact
        && integer_equal(&a.crypto_fee_msat, &b.crypto_fee_msat)
        && (Arc::ptr_eq(&a.provenance, &b.provenance)
            || (a.provenance.transaction_id == b.provenance.transaction_id
                && a.provenance.engine_event_id == b.provenance.engine_event_id
                && a.provenance.occurred_at == b.provenance.occurred_at
                && a.provenance.pricing.iter().eq(b.provenance.pricing.iter())))
}

fn consume_direct(calculation: &Calculation, cursor: &mut Cursor<'_>) -> Option<()> {
    let mut sources: [Option<&Source>; 16] = [None; 16];
    let mut count = 0;
    for fragment in &calculation.fragments {
        for source in [Some(&fragment.event), fragment.lot.as_ref()]
            .into_iter()
            .flatten()
        {
            if !sources[..count]
                .iter()
                .flatten()
                .any(|prior| source_equal(prior, source))
            {
                sources[count] = Some(source);
                count += 1;
            }
        }
    }
    cursor.take(b"[2,")?;
    cursor.quoted(calculation.engine)?;
    cursor.take(b",")?;
    cursor.quoted(&calculation.method)?;
    cursor.take(b",[")?;
    for (index, source) in sources[..count].iter().flatten().enumerate() {
        if index > 0 {
            cursor.take(b",")?;
        }
        cursor.take(b"[")?;
        cursor.scalar(&source.provenance.transaction_id)?;
        for value in [
            source.provenance.engine_event_id.as_ref(),
            &source.provenance.occurred_at,
            &source.asset,
            &source.spot_price_exact,
            &source.fiat_fee_exact,
        ] {
            cursor.take(b",")?;
            cursor.quoted(value)?;
        }
        cursor.take(b",")?;
        Scalar::integer(&source.crypto_fee_msat).consume(cursor)?;
        for index in [0, 5, 1, 2, 9, 10, 11, 8, 7, 4, 3, 6] {
            cursor.take(b",")?;
            cursor.scalar(&source.provenance.pricing[index])?;
        }
        cursor.take(b"]")?;
    }
    cursor.take(b"],[")?;
    for (index, fragment) in calculation.fragments.iter().enumerate() {
        if index > 0 {
            cursor.take(b",")?;
        }
        cursor.take(b"[")?;
        Scalar::integer(&fragment.quantity_msat).consume(cursor)?;
        for value in [
            &fragment.cost_basis_exact,
            &fragment.proceeds_exact,
            &fragment.gain_loss_exact,
        ] {
            cursor.take(b",")?;
            cursor.quoted(value)?;
        }
        cursor.take(b",")?;
        cursor.optional(fragment.unit_basis_override_exact.as_deref())?;
        cursor.take(b",")?;
        cursor.quoted(fragment.basis_authority)?;
        cursor.take(b",")?;
        cursor.optional(fragment.acquisition_basis_exact.as_deref())?;
        cursor.take(b",")?;
        match &fragment.acquisition_quantity_msat {
            Some(value) => Scalar::integer(value).consume(cursor)?,
            None => cursor.take(b"null")?,
        }
        for source in [Some(&fragment.event), fragment.lot.as_ref()] {
            cursor.take(b",")?;
            match source {
                Some(source) => {
                    let position = sources[..count]
                        .iter()
                        .flatten()
                        .position(|prior| source_equal(prior, source))?;
                    write!(cursor, "{position}").ok()?;
                }
                None => cursor.take(b"null")?,
            }
        }
        cursor.take(b"]")?;
    }
    cursor.take(b"]]")
}

struct Cursor<'a>(&'a [u8]);
impl Cursor<'_> {
    fn scalar(&mut self, value: &Value) -> Option<()> {
        match value {
            Value::Null => self.take(b"null"),
            Value::String(s) => self.quoted(s),
            _ => None,
        }
    }
    fn take(&mut self, expected: &[u8]) -> Option<()> {
        self.0 = self.0.strip_prefix(expected)?;
        Some(())
    }
    fn optional(&mut self, value: Option<&str>) -> Option<()> {
        match value {
            Some(value) => self.quoted(value),
            None => self.take(b"null"),
        }
    }
    fn quoted_known(&mut self, value: &str, plain: bool) -> Option<()> {
        if plain {
            self.take(b"\"")?;
            self.take(value.as_bytes())?;
            self.take(b"\"")
        } else {
            self.quoted(value)
        }
    }
    fn quoted(&mut self, value: &str) -> Option<()> {
        if value
            .bytes()
            .all(|b| matches!(b, 0x20..=0x7e) && !matches!(b, b'"' | b'\\'))
        {
            self.take(b"\"")?;
            self.take(value.as_bytes())?;
            return self.take(b"\"");
        }
        self.take(b"\"")?;
        let mut start = 0;
        for (index, c) in value.char_indices() {
            if (' '..='~').contains(&c) && c != '"' && c != '\\' {
                continue;
            }
            self.take(&value.as_bytes()[start..index])?;
            start = index + c.len_utf8();
            match c {
                '"' => self.take(b"\\\"")?,
                '\\' => self.take(b"\\\\")?,
                '\u{8}' => self.take(b"\\b")?,
                '\u{c}' => self.take(b"\\f")?,
                '\n' => self.take(b"\\n")?,
                '\r' => self.take(b"\\r")?,
                '\t' => self.take(b"\\t")?,
                _ => {
                    for unit in c.encode_utf16(&mut [0; 2]) {
                        write!(self, "\\u{unit:04x}").ok()?;
                    }
                }
            }
        }
        self.take(&value.as_bytes()[start..])?;
        self.take(b"\"")
    }
}
impl std::fmt::Write for Cursor<'_> {
    fn write_str(&mut self, value: &str) -> std::fmt::Result {
        self.take(value.as_bytes()).ok_or(std::fmt::Error)
    }
}

#[cfg(test)]
mod audit_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_string_comparison_preserves_escaping_and_byte_boundaries() {
        let controls: String = (0..=127).map(|n| char::from_u32(n).unwrap()).collect();
        for value in [
            controls.as_str(),
            "",
            "é🔥\u{80}\u{ffff}\u{10000}\u{10ffff}",
            "left\\\"right",
            "plain/ascii",
        ] {
            let mut encoded = String::new();
            kassiber_pyjson::dumps_str_into(
                value,
                &kassiber_pyjson::Options::new().compact(),
                &mut encoded,
            );
            let mut cursor = Cursor(encoded.as_bytes());
            assert!(cursor.quoted(value).is_some());
            assert!(cursor.0.is_empty());
            for end in 0..encoded.len() {
                assert!(Cursor(&encoded.as_bytes()[..end]).quoted(value).is_none());
            }
        }
        for (value, alternate) in [
            ("a", "\"\\u0061\""),
            ("/", "\"\\/\""),
            ("\n", "\"\\u000a\""),
            ("é", "\"é\""),
            ("é", "\"\\u00E9\""),
        ] {
            assert!(Cursor(alternate.as_bytes()).quoted(value).is_none());
        }
    }
}
