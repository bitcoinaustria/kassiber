//! Which assets an import keeps, and how their quantities are stored
//! (docs/plan/24-crypto-assets.md). Bitcoin is always kept. Other crypto
//! assets are kept only when the wallet's `asset_scope` is `crypto`;
//! securities, metals and other non-crypto holdings never are. Every row an
//! importer leaves out by asset is counted, so the user can see what the
//! scope skipped and turn it on.
use super::*;

/// ISO 4217 codes the exchange importers treat as fiat cash legs.
pub(crate) const FIAT: [&str; 29] = [
    "AED", "ARS", "AUD", "BRL", "CAD", "CHF", "CLP", "CNY", "CZK", "DKK", "EUR", "GBP", "HKD",
    "HUF", "ILS", "INR", "JPY", "KRW", "MXN", "NOK", "NZD", "PLN", "RON", "SEK", "SGD", "THB",
    "TRY", "USD", "ZAR",
];

/// What a canonical asset code is, for import purposes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Class {
    /// BTC and Liquid BTC: always imported.
    Bitcoin,
    /// A fiat currency: a cash leg, never a holding row.
    Fiat,
    /// Any other crypto asset: imported only in `crypto` scope.
    Crypto,
    /// Stocks, ETFs, ETCs, metals, indices: never imported. Only a provider's
    /// own asset type can say this; a ticker alone never does.
    #[allow(
        dead_code,
        reason = "only an exchange mapper that reads a provider asset type builds it"
    )]
    Excluded,
}

/// The class of a canonical code when the provider gives no asset type.
pub(crate) fn class(code: &str) -> Class {
    match code {
        "BTC" | "LBTC" => Class::Bitcoin,
        _ if FIAT.contains(&code) => Class::Fiat,
        _ => Class::Crypto,
    }
}

/// A wallet's `asset_scope`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    /// BTC only (the default, and every wallet's behaviour before the setting).
    #[default]
    Bitcoin,
    /// BTC and every other crypto asset.
    Crypto,
}
impl Scope {
    pub const KEY: &'static str = "asset_scope";

    pub fn parse(value: &str) -> Result<Self, AppError> {
        match kassiber_pytext::lower(kassiber_pytext::strip(value)).as_str() {
            "bitcoin" => Ok(Self::Bitcoin),
            "crypto" => Ok(Self::Crypto),
            _ => Err(AppError::new(
                "validation",
                "asset_scope must be 'bitcoin' or 'crypto'",
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bitcoin => "bitcoin",
            Self::Crypto => "crypto",
        }
    }

    /// The scope stored on a wallet; absent means `bitcoin`.
    pub fn of_wallet(conn: &Connection, wallet_id: &str) -> Result<Self, AppError> {
        let stored: String = conn
            .query_row(
                "SELECT config_json FROM wallets WHERE id=?",
                [wallet_id],
                |r| r.get(0),
            )
            .map_err(database_error)?;
        let config: Json = serde_json::from_str(&stored).unwrap_or(Json::Null);
        match config.get(Self::KEY).and_then(Json::as_str) {
            Some(value) => Self::parse(value),
            None => Ok(Self::Bitcoin),
        }
    }

    /// Stores the scope on a wallet through the wallet lifecycle (validation,
    /// journal invalidation), unless it is already set to it. The desktop
    /// toggle uses this; the CLI sets `asset_scope` with `wallets update`.
    pub fn store(
        self,
        conn: &Connection,
        profile_id: &str,
        wallet_id: &str,
    ) -> Result<(), AppError> {
        if Self::of_wallet(conn, wallet_id)? == self {
            return Ok(());
        }
        lifecycle::execute(
            conn,
            lifecycle::Operation::Update {
                profile_id: profile_id.into(),
                wallet: wallet_id.into(),
                label: None,
                account: None,
                config: json!({ Self::KEY: self.as_str() }),
                clear: vec![],
            },
        )
        .map(|_| ())
    }
}

/// The rows an import left out by asset, by canonical ticker.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Tally {
    /// Crypto rows the `bitcoin` scope skipped; `crypto` scope would keep them.
    skipped: BTreeMap<String, i64>,
    /// Non-crypto rows no scope keeps.
    excluded: BTreeMap<String, i64>,
}
impl Tally {
    /// Whether a row in `code` of `class` may be imported under `scope`;
    /// a row left out is counted. Fiat is never a holding and is not counted.
    pub(crate) fn admit(&mut self, scope: Scope, code: &str, class: Class) -> bool {
        match class {
            Class::Bitcoin => true,
            Class::Fiat => false,
            Class::Crypto if scope == Scope::Crypto => true,
            Class::Crypto => {
                *self.skipped.entry(code.into()).or_default() += 1;
                false
            }
            Class::Excluded => {
                *self.excluded.entry(code.into()).or_default() += 1;
                false
            }
        }
    }

    /// `{"scope", "skipped": {TICKER: rows}, "excluded": {TICKER: rows}}`,
    /// plus `next_step` naming the command that keeps the skipped assets
    /// when the `bitcoin` scope skipped any.
    pub(crate) fn envelope(&self, scope: Scope, wallet: &str) -> Py {
        let counts = |map: &BTreeMap<String, i64>| {
            dict(
                map.iter()
                    .map(|(code, rows)| (code.as_str(), Py::from(*rows)))
                    .collect::<Vec<_>>(),
            )
        };
        let mut fields = vec![
            ("scope", s(scope.as_str())),
            ("skipped", counts(&self.skipped)),
            ("excluded", counts(&self.excluded)),
        ];
        if !self.skipped.is_empty() {
            fields.push((
                "next_step",
                s(format!(
                    "{}, then sync or import again to keep {}",
                    keep_command(wallet),
                    self.skipped.keys().cloned().collect::<Vec<_>>().join(", ")
                )),
            ));
        }
        dict(fields)
    }
}

/// The canonical asset a row of a file export keeps under `scope`, for
/// exports without a provider asset type: Bitcoin (BTC, LBTC) always; another
/// crypto ticker in the `crypto` scope, else `None` and counted in `tally`.
/// A row without an asset, or whose asset is not a crypto asset (fiat, a
/// Bitcoin unit, a metal), is refused, naming `row`.
pub(super) fn file_row(
    code: Option<&str>,
    scope: Scope,
    tally: &mut Tally,
    row: &str,
) -> Result<Option<String>, AppError> {
    let code = code.unwrap_or_default();
    if code.is_empty() {
        return Err(AppError::new("validation", format!("{row} names no asset")));
    }
    if let Some(bitcoin) = exchange::bitcoin(Some(code)) {
        return Ok(Some(bitcoin.into()));
    }
    match tax_platform::ticker(code) {
        tax_platform::Ticker::Crypto => {
            Ok(tally.admit(scope, code, Class::Crypto).then(|| code.into()))
        }
        _ => Err(not_crypto(code, row)),
    }
}

fn not_crypto(code: &str, row: &str) -> AppError {
    AppError::new(
        "validation",
        format!("{row} has asset '{code}', which is not a crypto asset"),
    )
    .with_hint("Kassiber imports BTC and, in a wallet whose asset_scope is crypto, other crypto assets; check the row in the source.")
}

/// Refuses a row of a format without its own asset tally (generic CSV/JSON,
/// BTCPay) whose `asset` the wallet does not hold: Bitcoin and Liquid asset
/// ids always pass, another crypto ticker only in the `crypto` scope (the
/// refusal names the command that sets it), a Bitcoin alias (`XBT`) with
/// a request to write BTC, anything else never.
pub(super) fn screen_stored(
    code: &str,
    scope: Scope,
    wallet: &str,
    row: &str,
) -> Result<(), AppError> {
    let liquid = code.len() == 64 && code.bytes().all(|b| b.is_ascii_hexdigit());
    if liquid || class(code) == Class::Bitcoin {
        return Ok(());
    }
    match tax_platform::ticker(code) {
        tax_platform::Ticker::Crypto if scope == Scope::Crypto => Ok(()),
        tax_platform::Ticker::Crypto => Err(AppError::new(
            "validation",
            format!(
                "{row} has asset '{code}', which this wallet's bitcoin asset scope does not import"
            ),
        )
        .with_hint(format!(
            "To import {code}, run {}, then import the file again.",
            keep_command(wallet)
        ))),
        // Earlier builds stored the alias as written, a separate asset.
        tax_platform::Ticker::Bitcoin => Err(AppError::new(
            "validation",
            format!("{row} has asset '{code}', another name for BTC"),
        )
        .with_hint("Kassiber stores Bitcoin as BTC: write BTC in the row's asset column, then import the file again.")),
        _ => Err(not_crypto(code, row)),
    }
}

/// The `(amount, fee)` of a fee paid in a third asset (BEST, BNB, KCS): a
/// fee-only spend of that asset, no principal and the whole quantity as its
/// fee. The tax engine books a fee-only spend under the country's fee rule;
/// a positive principal would be a sale at the fee's value instead. Every
/// importer's third-asset fee row goes through here.
pub(super) fn fee_only(quantity: PriceNumber) -> (PriceNumber, PriceNumber) {
    (PriceNumber::Finite(Decimal::zero()), quantity)
}

/// The net quantity an inbound row of an asset other than Bitcoin stores
/// when the provider reports a gross amount and a fee in that same asset
/// (Kraken's ledger, an export's received amount with its fee): the row
/// keeps `gross - fee` as its amount and no fee. Custody counts only an
/// inbound row's amount, so a kept fee would leave the gross quantity in the
/// book; the fee's value stays in the acquisition's cost, as a fiat fee on a
/// buy does. `None` when there is nothing to net: a Bitcoin row (unchanged),
/// no fee, or a non-finite value (the coordinator's own error). A fee that
/// is not smaller than the amount is refused, naming the row.
pub(super) fn net_inbound(
    asset: &str,
    amount: &PriceNumber,
    fee: &PriceNumber,
    row: &str,
) -> Result<Option<PriceNumber>, AppError> {
    let (PriceNumber::Finite(gross), PriceNumber::Finite(charged)) = (amount, fee) else {
        return Ok(None);
    };
    if class(asset) == Class::Bitcoin || charged.is_zero() {
        return Ok(None);
    }
    let (gross, charged) = (gross.copy_abs(), charged.copy_abs());
    let net_amount = gross.sub(&charged, &CTX).map_err(decimal_error)?;
    if !net_amount.is_positive() {
        return Err(AppError::new(
            "validation",
            format!("{row} receives {gross} {asset} but charges a fee of {charged} {asset}"),
        )
        .with_hint("Kassiber stores an incoming amount net of its fee and cannot store a fee that takes all of it; check the row in the source."));
    }
    Ok(Some(PriceNumber::Finite(net_amount)))
}

/// The command that sets a wallet's scope to `crypto`, for hints that tell
/// the user how to keep the assets the `bitcoin` scope leaves out.
pub(crate) fn keep_command(wallet: &str) -> String {
    // POSIX single quotes; a quote inside becomes '\''.
    let wallet = format!("'{}'", wallet.replace('\'', "'\\''"));
    format!("kassiber wallets update --wallet {wallet} --config '{{\"asset_scope\": \"crypto\"}}'")
}

/// The stored integer of a non-Bitcoin quantity: exactly `amount * 10^11`.
/// Bitcoin keeps its msat conversion; every other asset uses the same unit
/// but is refused, never rounded, when it is finer than 11 decimals or does
/// not fit the column. Only the decimal's own digits and exponent are read:
/// no rounded operation runs before the check, so a digit past the 11th
/// decimal is never rounded away. Callers pass the value as parsed, not one
/// that a rounded `abs` or sum has already shortened.
/// The absolute value a mapper stores for `asset`. Bitcoin keeps the
/// reference's context-rounded `abs` (its msat conversion rounds anyway);
/// another asset keeps the provider's exact digits, so `exact_units` refuses
/// a finer quantity instead of seeing it already rounded.
pub(crate) fn stored_abs(asset: &str, n: PriceNumber) -> Result<PriceNumber, AppError> {
    match n {
        PriceNumber::Finite(v) if class(asset) != Class::Bitcoin => {
            Ok(PriceNumber::Finite(v.copy_abs()))
        }
        n => super::river::abs(n),
    }
}

pub(crate) fn exact_units(asset: &str, field: &str, amount: &Decimal) -> Result<i64, AppError> {
    let too_large = || {
        AppError::new(
            "validation",
            format!("{asset} {field} {amount} is too large to store"),
        )
        .with_hint("Kassiber stores at most about 92 million units of an asset other than Bitcoin in one row.")
    };
    let digits = amount.coefficient().to_string();
    let significant = digits.trim_end_matches('0');
    if significant.is_empty() {
        return Ok(0);
    }
    // `amount = significant * 10^exponent`, with no trailing zero left.
    let exponent = i128::from(amount.exponent()) + (digits.len() - significant.len()) as i128;
    let shift = exponent + 11;
    if shift < 0 {
        return Err(AppError::new(
            "validation",
            format!("{asset} {field} {amount} has more than 11 decimal places"),
        )
        .with_hint("Kassiber stores other crypto assets to 11 decimal places and does not round them; check the amount in the source."));
    }
    // i64 holds at most 19 digits.
    if significant.len() as i128 + shift > 19 {
        return Err(too_large());
    }
    let units = format!("{significant}{}", "0".repeat(shift as usize))
        .parse::<i64>()
        .map_err(|_| too_large())?;
    Ok(if amount.is_sign_negative() {
        -units
    } else {
        units
    })
}

#[cfg(test)]
mod exact_unit_tests {
    use super::*;

    fn units(amount: &str) -> Result<i64, String> {
        exact_units("ETH", "amount", &dec(amount).unwrap()).map_err(|error| error.message)
    }

    /// The check reads the decimal as given: a digit past the 11th decimal
    /// is refused even where a 32-digit rounded product would drop it, and
    /// trailing zeros of any length are no extra precision.
    #[test]
    fn exact_units_never_round_before_the_check() {
        assert_eq!(
            units("1.0000000000000000000000000000000000000001"),
            Err("ETH amount 1.0000000000000000000000000000000000000001 has more than 11 decimal places".into())
        );
        assert_eq!(
            units("12345678.123456789010000000000000000000001"),
            Err("ETH amount 12345678.123456789010000000000000000000001 has more than 11 decimal places".into())
        );
        assert_eq!(
            units("1.000000000000000000000000000000000000000000"),
            Ok(100_000_000_000)
        );
        assert_eq!(units("0.00000000001"), Ok(1));
        assert_eq!(units("-2.5"), Ok(-250_000_000_000));
        assert_eq!(units("0E-50"), Ok(0));
        assert_eq!(
            units("1E-12"),
            Err("ETH amount 1E-12 has more than 11 decimal places".into())
        );
        assert_eq!(units("92233720.36854775807"), Ok(i64::MAX));
        assert_eq!(
            units("92233720.36854775808"),
            Err("ETH amount 92233720.36854775808 is too large to store".into())
        );
        assert_eq!(
            units("1E+1000"),
            Err("ETH amount 1E+1000 is too large to store".into())
        );
    }
}
