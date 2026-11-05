use super::{authority_facts, integer_or_zero, raw_py, Error, EventKey};
use crate::codec::{self, text, truth};
use crate::fields::Fields;
use crate::python_text::PythonText;
use kassiber_pyjson::{BigInt, Key, Options, Value as Py};
use serde_json::{json, Value};
use std::borrow::Cow;
use std::fmt::Write;

fn string(value: &Value) -> Cow<'_, str> {
    match value {
        Value::String(value) => Cow::Borrowed(value),
        _ => Cow::Owned(text(value)),
    }
}

fn amount(value: &Value) -> Result<BigInt, Error> {
    if let Some(value) = value.as_i64() {
        return Ok(value.into());
    }
    if let Some(value) = value.as_u64() {
        return Ok(value.into());
    }
    let value = integer_or_zero(value)?;
    Ok(crate::quantity::exact(&value).expect("converted evidence integer"))
}

/// Whether every byte is printable ASCII other than quote and backslash,
/// tested eight bytes at a time.
fn plain(value: &str) -> bool {
    const ONES: u64 = 0x0101_0101_0101_0101;
    const HIGH: u64 = 0x8080_8080_8080_8080;
    // Each test sets a byte's high bit when that byte qualifies; a set bit
    // may stand for a lower byte, but no qualifying word escapes them.
    let zero = |word: u64| word.wrapping_sub(ONES) & !word & HIGH;
    let (words, rest) = value.as_bytes().as_chunks::<8>();
    for word in words {
        let word = u64::from_le_bytes(*word);
        let control = word.wrapping_sub(ONES * 0x20) & !word & HIGH;
        let high = (word.wrapping_add(ONES) | word) & HIGH;
        let quote = zero(word ^ (ONES * u64::from(b'"')));
        let backslash = zero(word ^ (ONES * u64::from(b'\\')));
        if control | high | quote | backslash != 0 {
            return false;
        }
    }
    rest.iter()
        .all(|b| matches!(b, 0x20..=0x7e) && *b != b'"' && *b != b'\\')
}

fn quoted(value: &str, out: &mut String) {
    // Printable ASCII other than quote and backslash is written as is.
    if plain(value) {
        out.reserve(value.len() + 2);
        out.push('"');
        out.push_str(value);
        out.push('"');
        return;
    }
    kassiber_pyjson::dumps_str_into(value, &Options::new().ensure_ascii(false), out);
}

struct Quantity<'a> {
    key: &'a EventKey,
    wallet: Cow<'a, str>,
    direction: String,
    asset: String,
    amount: BigInt,
    fee: BigInt,
    included: bool,
}

impl<'a> Quantity<'a> {
    fn new<F: Fields + ?Sized>(row: &'a F, key: &'a EventKey) -> Result<Self, Error> {
        Ok(Self {
            key,
            wallet: string(row.at("wallet_id")),
            direction: text(row.at("direction")).py_lower(),
            asset: text(row.at("asset")).py_upper(),
            amount: amount(row.at("amount"))?,
            fee: amount(row.at("fee"))?,
            included: truth(row.at("amount_includes_fee")),
        })
    }

    fn preimage(&self) -> String {
        let mut out = String::with_capacity(320);
        write!(
            out,
            "{{\"amount_includes_fee\":{},\"amount_msat\":{},\"asset\":",
            self.included, self.amount
        )
        .unwrap();
        quoted(&self.asset, &mut out);
        out.push_str(",\"direction\":");
        quoted(&self.direction, &mut out);
        out.push_str(",\"event\":{\"chain\":");
        quoted(&self.key.chain, &mut out);
        out.push_str(",\"native_event_id\":");
        quoted(&self.key.native_event_id, &mut out);
        out.push_str(",\"native_namespace\":");
        quoted(&self.key.native_namespace, &mut out);
        out.push_str(",\"network\":");
        quoted(&self.key.network, &mut out);
        write!(
            out,
            "}},\"fee_msat\":{},\"schema_version\":1,\"wallet_id\":",
            self.fee
        )
        .unwrap();
        quoted(&self.wallet, &mut out);
        out.push('}');
        out
    }
}

pub(super) struct Commitment {
    pub quantity_hash: String,
    pub detail_hash: String,
    pub payload_json: String,
}

impl Commitment {
    pub fn build<F: Fields + ?Sized>(
        row: &F,
        key: &EventKey,
        override_raw: Option<Py>,
    ) -> Result<Self, Error> {
        let quantity = Quantity::new(row, key)?;
        Self::from_quantity(row, &quantity, override_raw)
    }

    fn from_quantity<F: Fields + ?Sized>(
        row: &F,
        quantity: &Quantity<'_>,
        override_raw: Option<Py>,
    ) -> Result<Self, Error> {
        let preimage = quantity.preimage();
        if quantity.amount.bits() >= 14200 || quantity.fee.bits() >= 14200 {
            codec::hash(&super::quantity_payload_of(row, quantity.key)?, false)
                .map_err(Error::encoding)?;
        }
        let quantity_hash = codec::sha256(&preimage);
        let vout = codec::from_json(row.at("swap_refund_funding_vout"))?;
        let raw: Cow<'_, Py> = match (override_raw, row.at("raw_json"), row.loaded_raw()) {
            (Some(raw), _, _) => Cow::Owned(raw),
            (None, Value::Null, _) => Cow::Owned(Py::Dict(vec![])),
            // raw_py of a cached string: empty is an empty object, and
            // unparsable text is kept opaque.
            (None, Value::String(s), Some(loaded)) => match loaded {
                _ if s.is_empty() => Cow::Owned(Py::Dict(vec![])),
                Ok(raw) => Cow::Borrowed(raw),
                Err(()) => Cow::Owned(Py::Dict(vec![(
                    Key::Str("unparsed_text".into()),
                    Py::Str(s.as_str().into()),
                )])),
            },
            (None, raw, _) => Cow::Owned(raw_py(raw, "unparsed_text")?),
        };
        let options = Options::new()
            .sort_keys(true)
            .compact()
            .ensure_ascii(false)
            .default_str(true);
        // The raw object's text length bounds most of its dump.
        let raw_len = row.at("raw_json").as_str().map_or(0, str::len);
        let mut payload_json = String::with_capacity(preimage.len() + raw_len + 384);
        payload_json.push('{');
        for (index, field) in [
            "confirmed_at",
            "fingerprint",
            "kind",
            "occurred_at",
            "payment_hash",
            "payment_hash_source",
        ]
        .into_iter()
        .enumerate()
        {
            if index > 0 {
                payload_json.push(',');
            }
            quoted(field, &mut payload_json);
            payload_json.push(':');
            quoted(&string(row.at(field)), &mut payload_json);
        }
        payload_json.push_str(",\"quantity\":");
        payload_json.push_str(&preimage);
        payload_json.push_str(",\"raw_json\":");
        let raw_result = kassiber_pyjson::dumps_into(raw.as_ref(), &options, 1, &mut payload_json);
        payload_json.push_str(",\"schema_version\":1,\"swap_refund_funding_txid\":");
        quoted(
            &string(row.at("swap_refund_funding_txid")),
            &mut payload_json,
        );
        payload_json.push_str(",\"swap_refund_funding_vout\":");
        let vout_result = kassiber_pyjson::dumps_into(&vout, &options, 1, &mut payload_json);
        payload_json.push('}');
        if raw_result.is_err() || vout_result.is_err() {
            let mut detail = codec::from_json(
                &json!({"schema_version":1,"quantity":super::quantity_payload_of(row,quantity.key)?,"fingerprint":text(row.at("fingerprint")),"occurred_at":text(row.at("occurred_at")),"confirmed_at":text(row.at("confirmed_at")),"kind":text(row.at("kind")),"payment_hash":text(row.at("payment_hash")),"payment_hash_source":text(row.at("payment_hash_source")),"swap_refund_funding_txid":text(row.at("swap_refund_funding_txid")),"swap_refund_funding_vout":row.at("swap_refund_funding_vout")}),
            )?;
            if let Py::Dict(values) = &mut detail {
                values.push((Key::Str("raw_json".into()), raw.into_owned()));
            }
            payload_json = codec::dumps(&detail, false).map_err(Error::encoding)?;
        }
        Ok(Self {
            quantity_hash,
            detail_hash: codec::sha256(&payload_json),
            payload_json,
        })
    }

    pub fn value(self) -> Value {
        json!({"$type":"EvidenceSnapshot","quantity_hash":self.quantity_hash,"detail_hash":self.detail_hash,"payload_json":self.payload_json})
    }
}

pub(super) struct Observation<'a> {
    quantity: Quantity<'a>,
    pub commitment: Commitment,
    pub transaction_id: Cow<'a, str>,
    anchor_id: Cow<'a, str>,
    profile_id: Cow<'a, str>,
    pub occurred_at: Cow<'a, str>,
    authoritative: bool,
    authority_reason: String,
    fee_attribution: String,
}

impl<'a> Observation<'a> {
    pub fn new<F: Fields + ?Sized>(row: &'a F, key: &'a EventKey) -> Result<Self, Error> {
        let quantity = Quantity::new(row, key)?;
        let commitment = Commitment::from_quantity(row, &quantity, None)?;
        let cached = row.at("_kassiber_verified_chain_observation");
        let (authoritative, authority_reason) = if truth(&cached["authoritative"]) {
            (true, text(&cached["reason"]))
        } else {
            let auth = authority_facts(row)?;
            (auth.authoritative, auth.reason.to_owned())
        };
        let attr = text(row.at("observation_fee_attribution"))
            .py_strip()
            .py_lower();
        let result = Self {
            quantity,
            commitment,
            transaction_id: string(row.at("id")),
            anchor_id: string(if truth(row.at("journal_transaction_id")) {
                row.at("journal_transaction_id")
            } else {
                row.at("id")
            }),
            profile_id: string(row.at("profile_id")),
            occurred_at: string(row.at("occurred_at")),
            authoritative,
            authority_reason,
            fee_attribution: if authoritative
                && ["exact", "implicit_wallet_delta"].contains(&attr.as_str())
            {
                attr
            } else {
                "unknown".into()
            },
        };
        result.validate()?;
        Ok(result)
    }

    pub fn slot(&self) -> (&str, &str, &str) {
        (
            &self.quantity.wallet,
            &self.quantity.direction,
            &self.quantity.asset,
        )
    }
    pub fn selection(&self) -> (&str, &str, &str) {
        (
            &self.occurred_at,
            &self.commitment.detail_hash,
            &self.transaction_id,
        )
    }
    /// The exact amount, fee and inclusion: Python admits reviewed amounts
    /// up to its integer-string limit, beyond any machine integer.
    pub fn semantics(&self) -> (&BigInt, &BigInt, bool) {
        (
            &self.quantity.amount,
            &self.quantity.fee,
            self.quantity.included,
        )
    }

    fn validate(&self) -> Result<(), Error> {
        if self.transaction_id.is_empty()
            || self.anchor_id.is_empty()
            || self.quantity.wallet.is_empty()
        {
            return Err("quantity observations require transaction, anchor, and wallet ids".into());
        }
        let direction = &self.quantity.direction;
        if !["inbound", "outbound"].contains(&direction.as_str()) {
            return Err("quantity observation direction must be inbound or outbound".into());
        }
        if self.quantity.asset.is_empty() {
            return Err("quantity observations require an asset".into());
        }
        let (amount, fee, included) = self.semantics();
        let zero = BigInt::from(0);
        if *amount < zero {
            return Err("quantity observation amount_msat must be non-negative".into());
        }
        if *fee < zero {
            return Err("quantity observation fee_msat must be non-negative".into());
        }
        if direction == "inbound" && included {
            return Err("inbound quantity cannot use amount_includes_fee".into());
        }
        if *amount == zero && !(direction == "outbound" && *fee > zero) {
            return Err("zero quantity is valid only for a fee-only outbound".into());
        }
        if included && fee > amount {
            return Err("an included fee cannot exceed the observed amount".into());
        }
        Ok(())
    }

    pub fn reconcile(&mut self, proof: &Value) -> Result<(), Error> {
        self.commitment.quantity_hash=codec::hash(&json!({"observed_quantity_hash":self.commitment.quantity_hash,"receipt_reconciliation":proof}),false)?.0;
        let mut payload = codec::loads(&self.commitment.payload_json)?;
        if let Py::Dict(values) = &mut payload {
            values.push((
                Key::Str("receipt_reconciliation".into()),
                codec::from_json(proof)?,
            ));
        }
        let (hash, text) = codec::hash_py(&payload, false)?;
        self.commitment.detail_hash = hash;
        self.commitment.payload_json = text;
        self.quantity.amount = crate::quantity::exact(&proof["amount_msat"])
            .expect("converted reconciliation integer");
        self.validate()
    }

    /// The typed observation, sharing its event's key object and moving
    /// its texts.
    pub fn into_leg(self, event: &std::sync::Arc<super::canonical::Keys>) -> super::canonical::Leg {
        let string = |value: Cow<'_, str>| Value::String(value.into_owned());
        super::canonical::Leg::new(
            [
                Value::Bool(self.quantity.included),
                crate::quantity::number(&self.quantity.amount),
                string(self.anchor_id),
                Value::String(self.quantity.asset),
                Value::Bool(self.authoritative),
                Value::String(self.quantity.direction),
                Value::String(self.commitment.detail_hash),
                Value::String(self.commitment.payload_json),
                Value::String(self.fee_attribution),
                crate::quantity::number(&self.quantity.fee),
                Value::String(self.authority_reason),
                string(self.occurred_at),
                string(self.profile_id),
                Value::String(self.commitment.quantity_hash),
                string(self.transaction_id),
                string(self.quantity.wallet),
            ],
            event.clone(),
        )
    }

    pub fn value(&self) -> Value {
        json!({"$type":"QuantityObservation","transaction_id":self.transaction_id,"anchor_transaction_id":self.anchor_id,"quantity_hash":self.commitment.quantity_hash,"evidence_detail_hash":self.commitment.detail_hash,"evidence_payload_json":self.commitment.payload_json,"event_key":self.quantity.key.value(),"profile_id":self.profile_id,"authoritative_chain_observation":self.authoritative,"observation_authority_reason":self.authority_reason,"fee_attribution":self.fee_attribution,"wallet_id":self.quantity.wallet,"asset":self.quantity.asset,"direction":self.quantity.direction,"amount_msat":crate::quantity::number(&self.quantity.amount),"fee_msat":crate::quantity::number(&self.quantity.fee),"amount_includes_fee":self.quantity.included,"occurred_at":self.occurred_at})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legacy(row: &Value, key: &EventKey, raw: Py) -> Result<Value, Error> {
        let quantity = super::super::quantity_payload(row, key)?;
        let quantity_hash = codec::hash(&quantity, false).map_err(Error::encoding)?.0;
        let mut detail = codec::from_json(
            &json!({"schema_version":1,"quantity":quantity,"fingerprint":text(&row["fingerprint"]),"occurred_at":text(&row["occurred_at"]),"confirmed_at":text(&row["confirmed_at"]),"kind":text(&row["kind"]),"payment_hash":text(&row["payment_hash"]),"payment_hash_source":text(&row["payment_hash_source"]),"swap_refund_funding_txid":text(&row["swap_refund_funding_txid"]),"swap_refund_funding_vout":row["swap_refund_funding_vout"]}),
        )?;
        if let Py::Dict(fields) = &mut detail {
            fields.push((Key::Str("raw_json".into()), raw));
        }
        let (detail_hash, payload_json) =
            codec::hash_py(&detail, false).map_err(Error::encoding)?;
        Ok(
            json!({"$type":"EvidenceSnapshot","quantity_hash":quantity_hash,"detail_hash":detail_hash,"payload_json":payload_json}),
        )
    }

    #[test]
    fn plain_words_match_the_byte_test() {
        let byte_plain = |text: &[u8]| {
            text.iter()
                .all(|b| matches!(b, 0x20..=0x7e) && *b != b'"' && *b != b'\\')
        };
        // Every byte value at every position of a word and of its tail.
        for len in [1, 7, 8, 9, 16, 17] {
            for at in 0..len {
                for byte in 0..=0x7f_u8 {
                    let mut text = vec![b'a'; len];
                    text[at] = byte;
                    let text = String::from_utf8(text).unwrap();
                    assert_eq!(plain(&text), byte_plain(text.as_bytes()), "{text:?}");
                }
                let mut text = "a".repeat(len);
                text.replace_range(at..=at, "é");
                assert!(!plain(&text), "{text:?}");
            }
        }
        assert!(plain(""));
        assert!(plain("0123456789abcdef0123456789abcdef~ !#[]{}"));
    }

    #[test]
    fn typed_evidence_commitments_preserve_exceptional_preimages() {
        let key = EventKey {
            chain: "bitcoin".into(),
            network: "regtest".into(),
            native_namespace: "chain".into(),
            native_event_id: "0".repeat(64),
        };
        let mut row = json!({"wallet_id":"钱\n🐍","direction":"OUTBOUND","asset":"ßᲉ","amount":"١_٢٣","fee":1.9,"kind":false,"payment_hash":"\t\"","fingerprint":"é","swap_refund_funding_vout":{"é":[1.0,-0.0,1e22]}});
        let mut raws = vec![
            codec::loads(r#"{"é":[NaN,Infinity,-Infinity,-0.0,18446744073709551616]}"#).unwrap(),
            codec::loads(r#"{"surrogate":"\ud800"}"#).unwrap(),
            codec::loads(r#"{"surrogate":"\ud800\ud801","next":"钱"}"#).unwrap(),
        ];
        let mut deep = Py::Null;
        for _ in 0..902 {
            deep = Py::List(vec![deep]);
        }
        raws.push(deep);
        for raw in raws {
            let expected = legacy(&row, &key, raw.clone());
            let actual = Commitment::build(&row, &key, Some(raw)).map(Commitment::value);
            match (expected, actual) {
                (Ok(expected), Ok(actual)) => assert_eq!(actual, expected),
                (Err(expected), Err(actual)) => {
                    assert_eq!(actual.code, expected.code);
                    assert_eq!(actual.message, expected.message);
                }
                (expected, actual) => panic!("{expected:?} != {actual:?}"),
            }
        }
        row["amount"] = serde_json::from_str(&format!("1{}", "0".repeat(4300))).unwrap();
        let expected = legacy(&row, &key, Py::Null).unwrap_err();
        let actual = Commitment::build(&row, &key, Some(Py::Null)).err().unwrap();
        assert_eq!(actual.code, expected.code);
        assert_eq!(actual.message, expected.message);
    }
}

#[cfg(test)]
mod quoting_tests {
    use super::*;

    #[test]
    fn quoted_strings_match_the_encoder() {
        for text in [
            "",
            "plain text",
            "a\"b",
            "a\\b",
            "tab\tx",
            "\u{7f}",
            "é",
            "\u{1f600}",
            "~ !",
        ] {
            let mut fast = String::new();
            quoted(text, &mut fast);
            let mut slow = String::new();
            kassiber_pyjson::dumps_str_into(text, &Options::new().ensure_ascii(false), &mut slow);
            assert_eq!(fast, slow, "{text:?}");
        }
        assert_eq!(
            codec::sha256("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
