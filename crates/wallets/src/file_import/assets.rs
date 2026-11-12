//! Which assets an import keeps, and how their quantities are stored
//! (docs/plan/23-crypto-assets.md). Bitcoin is always kept. Other crypto
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
            // POSIX single quotes; a quote inside becomes '\''.
            let wallet = format!("'{}'", wallet.replace('\'', "'\\''"));
            fields.push((
                "next_step",
                s(format!(
                    "kassiber wallets update --wallet {wallet} --config '{{\"asset_scope\": \"crypto\"}}', then sync or import again to keep {}",
                    self.skipped.keys().cloned().collect::<Vec<_>>().join(", ")
                )),
            ));
        }
        dict(fields)
    }
}

/// The stored integer of a non-Bitcoin quantity: exactly `amount * 10^11`.
/// Bitcoin keeps its msat conversion; every other asset uses the same unit
/// but is refused, never rounded, when it is finer than 11 decimals or does
/// not fit the column.
pub(crate) fn exact_units(asset: &str, field: &str, amount: &Decimal) -> Result<i64, AppError> {
    let scaled = amount
        .mul(&dec("100000000000")?, &CTX)
        .map_err(decimal_error)?;
    let whole = scaled
        .quantize(
            0,
            &Context {
                prec: 1000,
                rounding: Rounding::Down,
            },
        )
        .map_err(decimal_error)?;
    if whole != scaled {
        return Err(AppError::new(
            "validation",
            format!("{asset} {field} {amount} has more than 11 decimal places"),
        )
        .with_hint("Kassiber stores other crypto assets to 11 decimal places and does not round them; check the amount in the source."));
    }
    whole.to_string().parse::<i64>().map_err(|_| {
        AppError::new(
            "validation",
            format!("{asset} {field} {amount} is too large to store"),
        )
        .with_hint("Kassiber stores at most about 92 million units of an asset other than Bitcoin in one row.")
    })
}
