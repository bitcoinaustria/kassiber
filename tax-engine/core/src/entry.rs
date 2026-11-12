//! Engine entries: RP2's `InTransaction`, `OutTransaction`, and
//! `IntraTransaction` constructors.
//!
//! [`Entry::from_input`] runs every constructor check in RP2's order, with
//! RP2's 13-place comparisons and its exact messages, and stores the values
//! the constructor derives. The first failing check decides the error.

use std::fmt;

use crate::decimal::Decimal;
use crate::error::{EngineError, EngineResult};
use crate::model::{ConfigSpec, Dec, DerivedEntry, EntryInput, EntryKind};
use crate::num::{add, eq13, equal_within, gt13, lt13, mul, sub, truthy, zero, FIAT_EXPONENT};
use crate::time::Timestamp;

/// RP2's `TransactionType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TransactionType {
    Airdrop,
    Buy,
    Donate,
    Fee,
    Gift,
    Hardfork,
    Income,
    Interest,
    Lost,
    Mining,
    Move,
    Sell,
    Staking,
    Wages,
}

impl TransactionType {
    const ALL: [TransactionType; 14] = [
        TransactionType::Airdrop,
        TransactionType::Buy,
        TransactionType::Donate,
        TransactionType::Fee,
        TransactionType::Gift,
        TransactionType::Hardfork,
        TransactionType::Income,
        TransactionType::Interest,
        TransactionType::Lost,
        TransactionType::Mining,
        TransactionType::Move,
        TransactionType::Sell,
        TransactionType::Staking,
        TransactionType::Wages,
    ];

    /// The enum value (`"sell"`).
    pub fn value(self) -> &'static str {
        match self {
            TransactionType::Airdrop => "airdrop",
            TransactionType::Buy => "buy",
            TransactionType::Donate => "donate",
            TransactionType::Fee => "fee",
            TransactionType::Gift => "gift",
            TransactionType::Hardfork => "hardfork",
            TransactionType::Income => "income",
            TransactionType::Interest => "interest",
            TransactionType::Lost => "lost",
            TransactionType::Mining => "mining",
            TransactionType::Move => "move",
            TransactionType::Sell => "sell",
            TransactionType::Staking => "staking",
            TransactionType::Wages => "wages",
        }
    }

    /// Whether receipts of this type are income at receipt.
    pub fn is_earn(self) -> bool {
        matches!(
            self,
            TransactionType::Airdrop
                | TransactionType::Hardfork
                | TransactionType::Income
                | TransactionType::Interest
                | TransactionType::Mining
                | TransactionType::Staking
                | TransactionType::Wages
        )
    }

    /// RP2's lookup: the lowercased text must be an enum value.
    fn parse(text: &str) -> Option<Self> {
        let lowered = text.to_lowercase();
        Self::ALL.into_iter().find(|t| t.value() == lowered)
    }
}

impl fmt::Display for TransactionType {
    /// `str(TransactionType.SELL)`: `TransactionType.SELL`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TransactionType.{}", self.value().to_ascii_uppercase())
    }
}

/// The kind-specific values an entry stores.
#[derive(Clone, Debug)]
pub enum Detail {
    In {
        exchange: String,
        holder: String,
        crypto_in: Decimal,
        crypto_fee: Decimal,
        fiat_fee: Decimal,
        fiat_in_no_fee: Decimal,
        fiat_in_with_fee: Decimal,
    },
    Out {
        exchange: String,
        holder: String,
        crypto_out_no_fee: Decimal,
        crypto_fee: Decimal,
        crypto_out_with_fee: Decimal,
        fiat_out_no_fee: Decimal,
        /// Whether `fiat_out_no_fee` was given rather than derived.
        fiat_out_explicit: bool,
        fiat_fee: Decimal,
        fiat_out_with_fee: Decimal,
    },
    Intra {
        from_exchange: String,
        from_holder: String,
        to_exchange: String,
        to_holder: String,
        crypto_sent: Decimal,
        crypto_received: Decimal,
        crypto_fee: Decimal,
        fiat_fee: Decimal,
    },
}

/// A validated entry with RP2's stored and derived values.
#[derive(Clone, Debug)]
pub struct Entry {
    pub kind: EntryKind,
    pub row: i64,
    pub unique_id: String,
    pub asset: String,
    pub transaction_type: TransactionType,
    pub notes: String,
    pub ts: Timestamp,
    pub spot_price: Decimal,
    /// `str(transaction)`, for messages.
    pub text: String,
    pub detail: Detail,
}

fn type_error_none(name: &str, expected: &str) -> EngineError {
    EngineError::type_error(format!("Parameter '{name}' has non-{expected} value None"))
}

fn require_string<'a>(name: &str, value: Option<&'a str>) -> EngineResult<&'a str> {
    value.ok_or_else(|| type_error_none(name, "string"))
}

fn require_decimal<'a>(name: &str, value: &'a Option<Dec>) -> EngineResult<&'a Decimal> {
    value
        .as_ref()
        .map(|d| &d.0)
        .ok_or_else(|| type_error_none(name, "RP2Decimal"))
}

/// `Configuration.type_check_positive_decimal` on a present value.
fn check_positive(name: &str, value: &Decimal, non_zero: bool) -> EngineResult<()> {
    if lt13(value, &zero())? {
        return Err(EngineError::value(format!(
            "Parameter '{name}' has non-positive value {value}"
        )));
    }
    if non_zero && eq13(value, &zero())? {
        return Err(EngineError::value(format!(
            "Parameter '{name}' has zero value"
        )));
    }
    Ok(())
}

/// `Configuration.type_check_positive_decimal`, including the type check.
fn positive_decimal(name: &str, value: &Option<Dec>, non_zero: bool) -> EngineResult<Decimal> {
    let value = require_decimal(name, value)?;
    check_positive(name, value, non_zero)?;
    Ok(value.clone())
}

/// `Configuration.type_check_exchange` / `_holder` / `_asset`.
fn known(name: &str, value: Option<&str>, names: Option<&[String]>) -> EngineResult<String> {
    let text = require_string(name, value)?;
    if let Some(names) = names {
        if !names.iter().any(|n| n == text) {
            return Err(EngineError::value(format!(
                "Parameter '{name}' value is not known: {text}"
            )));
        }
    }
    Ok(text.to_owned())
}

/// What `AbstractTransaction.__init__` stores.
struct Base {
    ts: Timestamp,
    transaction_type: TransactionType,
    spot_price: Decimal,
    unique_id: String,
    notes: String,
}

/// `AbstractTransaction.__init__`: asset, timestamp, type, spot price, row,
/// unique id, and notes, in that order.
fn base(
    input: &EntryInput,
    config: Option<&ConfigSpec>,
    transaction_type: Option<&str>,
    spot_price: &Option<Dec>,
) -> EngineResult<Base> {
    known(
        "asset",
        Some(&input.asset),
        config.map(|c| c.assets.as_slice()),
    )?;
    let ts = Timestamp::from_input(&input.ts)?;
    let type_text = require_string("transaction_type", transaction_type)?;
    let transaction_type = TransactionType::parse(type_text).ok_or_else(|| {
        EngineError::value(format!(
            "Parameter 'transaction_type' has invalid transaction type value: {type_text}"
        ))
    })?;
    let spot_price = positive_decimal("spot_price", spot_price, false)?;
    let notes = match &input.notes {
        Some(text) if !text.is_empty() => text.clone(),
        _ => String::new(),
    };
    Ok(Base {
        ts,
        transaction_type,
        spot_price,
        unique_id: input.unique_id.clone().unwrap_or_default(),
        notes,
    })
}

impl Entry {
    /// Runs the constructor for `input`. The membership checks run only
    /// with the configuration's names.
    pub fn from_input(input: &EntryInput, config: Option<&ConfigSpec>) -> EngineResult<Entry> {
        reject_foreign_fields(input)?;
        match input.kind {
            EntryKind::In => Self::new_in(input, config),
            EntryKind::Out => Self::new_out(input, config),
            EntryKind::Intra => Self::new_intra(input, config),
        }
    }

    fn assemble(input: &EntryInput, base: Base, detail: Detail) -> Entry {
        Entry {
            kind: input.kind,
            row: input.row,
            unique_id: base.unique_id,
            asset: input.asset.clone(),
            transaction_type: base.transaction_type,
            notes: base.notes,
            ts: base.ts,
            spot_price: base.spot_price,
            text: input.text.clone(),
            detail,
        }
    }

    fn new_in(input: &EntryInput, config: Option<&ConfigSpec>) -> EngineResult<Entry> {
        let base = base(
            input,
            config,
            input.transaction_type.as_deref(),
            &input.spot_price,
        )?;
        let exchanges = config.map(|c| c.exchanges.as_slice());
        let holders = config.map(|c| c.holders.as_slice());
        let exchange = known("exchange", input.exchange.as_deref(), exchanges)?;
        let holder = known("holder", input.holder.as_deref(), holders)?;
        let crypto_in = positive_decimal("crypto_in", &input.crypto_in, true)?;
        let crypto_fee = match &input.crypto_fee {
            Some(Dec(value)) if truthy(value) => {
                check_positive("crypto_fee", value, false)?;
                value.clone()
            }
            _ => zero(),
        };
        let mut fiat_fee = match &input.fiat_fee {
            Some(Dec(value)) if truthy(value) => {
                check_positive("fiat_fee", value, false)?;
                value.clone()
            }
            _ => zero(),
        };
        let prefix = message_prefix(&input.asset, input.kind, &base.ts, input.row);
        let spot = &base.spot_price;
        if eq13(spot, &zero())? {
            return Err(EngineError::value(format!(
                "{prefix}: parameter 'spot_price' cannot be 0"
            )));
        }
        match (&input.crypto_fee, &input.fiat_fee) {
            (Some(_), None) => fiat_fee = mul(&crypto_fee, spot)?,
            (Some(_), Some(_)) => {
                return Err(EngineError::value(format!(
                    "{prefix}: both 'crypto_fee' and 'fiat_fee' are defined: only one allowed"
                )));
            }
            _ => {}
        }
        let fiat_in_no_fee = match &input.fiat_in_no_fee {
            None => mul(&crypto_in, spot)?,
            some => positive_decimal("fiat_in_no_fee", some, true)?,
        };
        let fiat_in_with_fee = match &input.fiat_in_with_fee {
            None => add(&fiat_in_no_fee, &fiat_fee)?,
            some => positive_decimal("fiat_in_with_fee", some, true)?,
        };
        let kind_ok = matches!(
            base.transaction_type,
            TransactionType::Buy | TransactionType::Gift | TransactionType::Donate
        ) || base.transaction_type.is_earn();
        if !kind_ok {
            return Err(EngineError::value(format!(
                "{prefix}: invalid transaction type {}",
                base.transaction_type
            )));
        }
        // RP2 only logs these mismatches, but evaluating them can raise.
        equal_within(&mul(&crypto_in, spot)?, &fiat_in_no_fee, FIAT_EXPONENT)?;
        equal_within(
            &fiat_in_with_fee,
            &add(&fiat_in_no_fee, &fiat_fee)?,
            FIAT_EXPONENT,
        )?;
        let detail = Detail::In {
            exchange,
            holder,
            crypto_in,
            crypto_fee,
            fiat_fee,
            fiat_in_no_fee,
            fiat_in_with_fee,
        };
        Ok(Self::assemble(input, base, detail))
    }

    fn new_out(input: &EntryInput, config: Option<&ConfigSpec>) -> EngineResult<Entry> {
        let base = base(
            input,
            config,
            input.transaction_type.as_deref(),
            &input.spot_price,
        )?;
        let exchanges = config.map(|c| c.exchanges.as_slice());
        let holders = config.map(|c| c.holders.as_slice());
        let exchange = known("exchange", input.exchange.as_deref(), exchanges)?;
        let holder = known("holder", input.holder.as_deref(), holders)?;
        let prefix = message_prefix(&input.asset, input.kind, &base.ts, input.row);
        let spot = &base.spot_price;
        let (crypto_out_no_fee, crypto_fee) = if base.transaction_type == TransactionType::Fee {
            let no_fee = positive_decimal("crypto_out_no_fee", &input.crypto_out_no_fee, false)?;
            if !eq13(&no_fee, &zero())? {
                return Err(EngineError::value(format!(
                    "{prefix}: fee-typed transaction has non-zero 'crypto_out_no_fee'"
                )));
            }
            let fee = positive_decimal("crypto_fee", &input.crypto_fee, true)?;
            (no_fee, fee)
        } else {
            if eq13(spot, &zero())? {
                return Err(EngineError::value(format!(
                    "{prefix}: parameter 'spot_price' cannot be 0"
                )));
            }
            let no_fee = positive_decimal("crypto_out_no_fee", &input.crypto_out_no_fee, true)?;
            let fee = positive_decimal("crypto_fee", &input.crypto_fee, false)?;
            (no_fee, fee)
        };
        let crypto_out_with_fee = match &input.crypto_out_with_fee {
            None => add(&crypto_out_no_fee, &crypto_fee)?,
            some => positive_decimal("crypto_out_with_fee", some, true)?,
        };
        let fiat_out_explicit = input.fiat_out_no_fee.is_some();
        let fiat_out_no_fee = match &input.fiat_out_no_fee {
            None => mul(&crypto_out_no_fee, spot)?,
            some => positive_decimal("fiat_out_no_fee", some, true)?,
        };
        let fiat_fee = match &input.fiat_fee {
            None => mul(&crypto_fee, spot)?,
            some => positive_decimal("fiat_fee", some, false)?,
        };
        let fiat_out_with_fee = add(&fiat_out_no_fee, &fiat_fee)?;
        let kind_ok = matches!(
            base.transaction_type,
            TransactionType::Donate
                | TransactionType::Fee
                | TransactionType::Gift
                | TransactionType::Lost
                | TransactionType::Sell
                | TransactionType::Staking
        );
        if !kind_ok {
            return Err(EngineError::value(format!(
                "{prefix}: invalid transaction type {}",
                base.transaction_type
            )));
        }
        // RP2 only logs these mismatches, but evaluating them can raise.
        equal_within(
            &crypto_out_with_fee,
            &add(&crypto_out_no_fee, &crypto_fee)?,
            FIAT_EXPONENT,
        )?;
        equal_within(&mul(&crypto_fee, spot)?, &fiat_fee, FIAT_EXPONENT)?;
        equal_within(
            &mul(&crypto_out_no_fee, spot)?,
            &fiat_out_no_fee,
            FIAT_EXPONENT,
        )?;
        let detail = Detail::Out {
            exchange,
            holder,
            crypto_out_no_fee,
            crypto_fee,
            crypto_out_with_fee,
            fiat_out_no_fee,
            fiat_out_explicit,
            fiat_fee,
            fiat_out_with_fee,
        };
        Ok(Self::assemble(input, base, detail))
    }

    fn new_intra(input: &EntryInput, config: Option<&ConfigSpec>) -> EngineResult<Entry> {
        let crypto_sent = positive_decimal("crypto_sent", &input.crypto_sent, true)?;
        let crypto_received = positive_decimal("crypto_received", &input.crypto_received, false)?;
        let crypto_fee = sub(&crypto_sent, &crypto_received)?;
        let mut spot_price = input.spot_price.clone();
        let spot_missing = match &spot_price {
            None => true,
            Some(Dec(value)) => eq13(value, &zero())?,
        };
        if spot_missing {
            if eq13(&crypto_fee, &zero())? {
                spot_price = Some(Dec(zero()));
            } else {
                let unique_id = input.unique_id.as_deref().unwrap_or("None");
                return Err(EngineError::value(format!(
                    "crypto_fee is non-zero ({crypto_fee}) but spot_price is empty or zero: {} {} {crypto_sent} {unique_id} ",
                    input.ts.raw, input.asset
                )));
            }
        }
        let base = base(input, config, Some("MOVE"), &spot_price)?;
        let exchanges = config.map(|c| c.exchanges.as_slice());
        let holders = config.map(|c| c.holders.as_slice());
        let from_exchange = known("from_exchange", input.from_exchange.as_deref(), exchanges)?;
        let from_holder = known("from_holder", input.from_holder.as_deref(), holders)?;
        let to_exchange = known("to_exchange", input.to_exchange.as_deref(), exchanges)?;
        let to_holder = known("to_holder", input.to_holder.as_deref(), holders)?;
        if lt13(&crypto_sent, &crypto_received)? {
            let prefix = message_prefix(&input.asset, input.kind, &base.ts, input.row);
            return Err(EngineError::value(format!(
                "{prefix}: crypto sent < crypto received"
            )));
        }
        let fiat_fee = mul(&crypto_fee, &base.spot_price)?;
        let detail = Detail::Intra {
            from_exchange,
            from_holder,
            to_exchange,
            to_holder,
            crypto_sent,
            crypto_received,
            crypto_fee,
            fiat_fee,
        };
        Ok(Self::assemble(input, base, detail))
    }

    /// RP2's `internal_id`.
    pub fn internal_id(&self) -> String {
        self.row.to_string()
    }

    /// The amount the entry moves: what a lot holds, or what a disposal
    /// consumes.
    pub fn crypto_balance_change(&self) -> &Decimal {
        match &self.detail {
            Detail::In { crypto_in, .. } => crypto_in,
            Detail::Out {
                crypto_out_with_fee,
                ..
            } => crypto_out_with_fee,
            Detail::Intra { crypto_fee, .. } => crypto_fee,
        }
    }

    /// The proceeds numerator of a taxable event.
    pub fn fiat_taxable_amount(&self) -> Decimal {
        match &self.detail {
            Detail::In {
                fiat_in_with_fee, ..
            } => {
                if self.transaction_type.is_earn() {
                    fiat_in_with_fee.clone()
                } else {
                    zero()
                }
            }
            Detail::Out {
                fiat_fee,
                fiat_out_no_fee,
                fiat_out_explicit,
                ..
            } => match self.transaction_type {
                TransactionType::Fee => fiat_fee.clone(),
                TransactionType::Lost if !fiat_out_explicit => zero(),
                _ => fiat_out_no_fee.clone(),
            },
            Detail::Intra { fiat_fee, .. } => fiat_fee.clone(),
        }
    }

    /// RP2's `crypto_taxable_amount`.
    pub fn crypto_taxable_amount(&self) -> Decimal {
        match &self.detail {
            Detail::In { crypto_in, .. } => {
                if self.transaction_type.is_earn() {
                    crypto_in.clone()
                } else {
                    zero()
                }
            }
            Detail::Out {
                crypto_fee,
                crypto_out_no_fee,
                ..
            } => {
                if self.transaction_type == TransactionType::Fee {
                    crypto_fee.clone()
                } else {
                    crypto_out_no_fee.clone()
                }
            }
            Detail::Intra { crypto_fee, .. } => crypto_fee.clone(),
        }
    }

    /// Whether the entry is a taxable event. A move is taxable when its fiat
    /// fee is positive at 13 places, which RP2 evaluates lazily (and which can
    /// raise).
    pub fn is_taxable(&self) -> EngineResult<bool> {
        match &self.detail {
            Detail::In { .. } => Ok(self.transaction_type.is_earn()),
            Detail::Out { .. } => Ok(true),
            Detail::Intra { fiat_fee, .. } => gt13(fiat_fee, &zero()),
        }
    }

    /// Whether the entry is an earn receipt (income at receipt).
    pub fn is_earning(&self) -> bool {
        matches!(self.detail, Detail::In { .. }) && self.transaction_type.is_earn()
    }

    /// A lot's `crypto_in`; `None` for other kinds.
    pub fn crypto_in(&self) -> Option<&Decimal> {
        match &self.detail {
            Detail::In { crypto_in, .. } => Some(crypto_in),
            _ => None,
        }
    }

    /// A lot's `fiat_in_with_fee`; `None` for other kinds.
    pub fn fiat_in_with_fee(&self) -> Option<&Decimal> {
        match &self.detail {
            Detail::In {
                fiat_in_with_fee, ..
            } => Some(fiat_in_with_fee),
            _ => None,
        }
    }

    /// The values `check_entry` reports.
    pub fn derived(&self) -> DerivedEntry {
        let dec = |value: &Decimal| Dec(value.clone());
        let mut out = DerivedEntry {
            kind: self.kind,
            row: self.row,
            unique_id: self.unique_id.clone(),
            notes: self.notes.clone(),
            transaction_type: self.transaction_type.value(),
            spot_price: dec(&self.spot_price),
            crypto_in: None,
            crypto_fee: Dec(zero()),
            fiat_fee: Dec(zero()),
            fiat_in_no_fee: None,
            fiat_in_with_fee: None,
            crypto_out_no_fee: None,
            crypto_out_with_fee: None,
            fiat_out_no_fee: None,
            fiat_out_with_fee: None,
            crypto_sent: None,
            crypto_received: None,
            crypto_balance_change: dec(self.crypto_balance_change()),
            crypto_taxable_amount: Dec(self.crypto_taxable_amount()),
            fiat_taxable_amount: Dec(self.fiat_taxable_amount()),
            is_taxable: self.is_taxable().ok(),
            is_earn: self.is_earning(),
        };
        match &self.detail {
            Detail::In {
                crypto_in,
                crypto_fee,
                fiat_fee,
                fiat_in_no_fee,
                fiat_in_with_fee,
                ..
            } => {
                out.crypto_in = Some(dec(crypto_in));
                out.crypto_fee = dec(crypto_fee);
                out.fiat_fee = dec(fiat_fee);
                out.fiat_in_no_fee = Some(dec(fiat_in_no_fee));
                out.fiat_in_with_fee = Some(dec(fiat_in_with_fee));
            }
            Detail::Out {
                crypto_out_no_fee,
                crypto_fee,
                crypto_out_with_fee,
                fiat_out_no_fee,
                fiat_fee,
                fiat_out_with_fee,
                ..
            } => {
                out.crypto_out_no_fee = Some(dec(crypto_out_no_fee));
                out.crypto_fee = dec(crypto_fee);
                out.crypto_out_with_fee = Some(dec(crypto_out_with_fee));
                out.fiat_out_no_fee = Some(dec(fiat_out_no_fee));
                out.fiat_fee = dec(fiat_fee);
                out.fiat_out_with_fee = Some(dec(fiat_out_with_fee));
            }
            Detail::Intra {
                crypto_sent,
                crypto_received,
                crypto_fee,
                fiat_fee,
                ..
            } => {
                out.crypto_sent = Some(dec(crypto_sent));
                out.crypto_received = Some(dec(crypto_received));
                out.crypto_fee = dec(crypto_fee);
                out.fiat_fee = dec(fiat_fee);
            }
        }
        out
    }
}

/// Rejects fields the entry's constructor does not take: a caller sending
/// them has drifted from the schema.
fn reject_foreign_fields(input: &EntryInput) -> EngineResult<()> {
    let present = [
        ("transaction_type", input.transaction_type.is_some()),
        ("exchange", input.exchange.is_some()),
        ("holder", input.holder.is_some()),
        ("from_exchange", input.from_exchange.is_some()),
        ("from_holder", input.from_holder.is_some()),
        ("to_exchange", input.to_exchange.is_some()),
        ("to_holder", input.to_holder.is_some()),
        ("crypto_in", input.crypto_in.is_some()),
        ("crypto_fee", input.crypto_fee.is_some()),
        ("fiat_in_no_fee", input.fiat_in_no_fee.is_some()),
        ("fiat_in_with_fee", input.fiat_in_with_fee.is_some()),
        ("fiat_fee", input.fiat_fee.is_some()),
        ("crypto_out_no_fee", input.crypto_out_no_fee.is_some()),
        ("crypto_out_with_fee", input.crypto_out_with_fee.is_some()),
        ("fiat_out_no_fee", input.fiat_out_no_fee.is_some()),
        ("crypto_sent", input.crypto_sent.is_some()),
        ("crypto_received", input.crypto_received.is_some()),
    ];
    let allowed: &[&str] = match input.kind {
        EntryKind::In => &[
            "transaction_type",
            "exchange",
            "holder",
            "crypto_in",
            "crypto_fee",
            "fiat_in_no_fee",
            "fiat_in_with_fee",
            "fiat_fee",
        ],
        EntryKind::Out => &[
            "transaction_type",
            "exchange",
            "holder",
            "crypto_out_no_fee",
            "crypto_fee",
            "crypto_out_with_fee",
            "fiat_out_no_fee",
            "fiat_fee",
        ],
        EntryKind::Intra => &[
            "from_exchange",
            "from_holder",
            "to_exchange",
            "to_holder",
            "crypto_sent",
            "crypto_received",
        ],
    };
    match present
        .iter()
        .find(|(name, is_set)| *is_set && !allowed.contains(name))
    {
        Some((name, _)) => Err(EngineError::request(format!(
            "field '{name}' does not apply to an '{}' entry",
            match input.kind {
                EntryKind::In => "in",
                EntryKind::Out => "out",
                EntryKind::Intra => "intra",
            }
        ))),
        None => Ok(()),
    }
}

/// `"{asset} {Class} ({timestamp}, id {row})"`, the prefix of RP2's
/// per-transaction messages.
fn message_prefix(asset: &str, kind: EntryKind, ts: &Timestamp, row: i64) -> String {
    format!("{asset} {} ({}, id {row})", kind.class_name(), ts.display)
}
