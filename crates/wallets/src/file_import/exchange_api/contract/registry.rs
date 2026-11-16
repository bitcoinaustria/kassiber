//! Every exchange Kassiber imports, in one table: what the coordinators,
//! the desktop check and the CLI dispatch need to know about each, so no
//! caller matches on an exchange's name.
use super::super::*;
use super::{Balance, Mapper};

/// Which assets an exchange's importer reads at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::file_import) enum Reads {
    /// Every asset; the wallet's asset scope decides what is kept.
    AllAssets,
    /// Only Bitcoin, in either scope: other assets are counted as `unread`.
    BitcoinOnly,
}

pub(in crate::file_import) struct Descriptor {
    /// The backend and wallet kind (`kraken`).
    pub kind: &'static str,
    /// The provider's name in records and messages (`Kraken`).
    pub provider: &'static str,
    pub reads: Reads,
    /// The API's base URL when the backend names none.
    pub default_url: &'static str,
    /// The API authenticates with the key alone (no secret).
    pub key_only: bool,
    /// Every history read of one sync, each event handed to the mapper; then
    /// the provider's own BTC balance, where it states one.
    pub fetch: fn(&Client<'_>, &mut Mapper) -> Result<Option<Balance>, AppError>,
    /// The desktop key check: the fewest reads that prove the key reads what
    /// the sync needs; `Some(true)` when the exchange confirms it cannot trade.
    pub probe: fn(&Client<'_>) -> Result<Option<bool>, AppError>,
    /// The envelope's `skipped` keys for fiat-only, other-asset and internal
    /// events, where the exchange reports them.
    pub skipped: Option<[&'static str; 3]>,
}

pub(in crate::file_import) static EXCHANGES: [&Descriptor; 4] = [
    &kraken::EXCHANGE,
    &coinbase::EXCHANGE,
    &binance::EXCHANGE,
    &bitpanda::EXCHANGE,
];

/// The exchange of a backend or wallet kind.
pub(in crate::file_import) fn by_kind(kind: &str) -> Option<&'static Descriptor> {
    EXCHANGES
        .iter()
        .copied()
        .find(|exchange| exchange.kind == kind)
}
