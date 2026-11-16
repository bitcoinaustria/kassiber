//! The setup fields of `ConnectionSource` that overview/catalog.rs (generated for the picker)
//! does not carry: `status`, `setupKind`, `forwardTo`, `walletKind`, `exchange`, `sourceFormat`,
//! `chain`, `network` — per source id, from ui-tauri/src/lib/connectionCatalog.tsx.

use crate::overview::add_connection::Source;
use crate::overview::catalog::SOURCES;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupKind {
    Descriptor,
    AddressList,
    SilentPayment,
    FileWallet,
    FileEnrichment,
    PriorReport,
    Samourai,
    Btcpay,
    BullbitcoinWallet,
    Bip329,
    BackendSettings,
    ExchangeApi,
    Planned,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Meta {
    pub setup: Option<SetupKind>,
    pub forward_to: Option<&'static str>,
    pub wallet_kind: Option<&'static str>,
    pub exchange: Option<&'static str>,
    pub source_format: Option<&'static str>,
    pub chain: Option<&'static str>,
    pub network: Option<&'static str>,
}

/// `meta(id)`; unknown ids have no setup.
pub fn meta(id: &str) -> Meta {
    use SetupKind::*;
    let m = |setup: Option<SetupKind>,
             wallet_kind: Option<&'static str>,
             exchange: Option<&'static str>,
             source_format: Option<&'static str>,
             chain: Option<&'static str>,
             network: Option<&'static str>| Meta {
        setup,
        forward_to: None,
        wallet_kind,
        exchange,
        source_format,
        chain,
        network,
    };
    let fwd = |to: &'static str| Meta {
        forward_to: Some(to),
        ..Meta::default()
    };
    match id {
        "descriptor" => m(
            Some(Descriptor),
            Some("descriptor"),
            None,
            None,
            Some("bitcoin"),
            None,
        ),
        "address-list" => m(
            Some(AddressList),
            Some("address"),
            None,
            None,
            Some("bitcoin"),
            Some("main"),
        ),
        "silent-payment" => m(
            Some(SilentPayment),
            Some("silent-payment"),
            None,
            None,
            Some("bitcoin"),
            Some("main"),
        ),
        "liquid-descriptor" => m(
            Some(Descriptor),
            Some("descriptor"),
            None,
            None,
            Some("liquid"),
            Some("liquidv1"),
        ),
        "samourai" => m(
            Some(Samourai),
            Some("samourai"),
            None,
            None,
            Some("bitcoin"),
            Some("main"),
        ),
        "wasabi" => m(
            Some(FileWallet),
            Some("wasabi"),
            None,
            Some("wasabi_bundle"),
            Some("bitcoin"),
            None,
        ),
        "bitcoin-core" | "electrum" | "esplora" => {
            m(Some(BackendSettings), None, None, None, None, None)
        }
        "sparrow"
        | "specter"
        | "bluewallet"
        | "blockstream-green"
        | "liana"
        | "nunchuk"
        | "bitbox"
        | "trezor"
        | "coldcard"
        | "foundation-passport" => fwd("descriptor"),
        "ledger" => m(
            Some(FileWallet),
            Some("ledgerlive"),
            None,
            Some("ledgerlive_csv"),
            None,
            None,
        ),
        "core-ln" => m(
            Some(BackendSettings),
            Some("coreln"),
            None,
            None,
            None,
            None,
        ),
        "lnd" => m(Some(BackendSettings), Some("lnd"), None, None, None, None),
        "phoenix" => m(
            Some(FileWallet),
            Some("phoenix"),
            None,
            Some("phoenix_csv"),
            None,
            None,
        ),
        "bullbitcoin-wallet" => m(
            Some(BullbitcoinWallet),
            Some("bullbitcoin"),
            None,
            Some("bullbitcoin_wallet_csv"),
            None,
            None,
        ),
        "btcpay" => m(Some(Btcpay), None, None, None, None, None),
        "btcpay-csv" => m(
            Some(FileWallet),
            Some("custom"),
            None,
            Some("btcpay_csv"),
            None,
            None,
        ),
        "river" => m(
            Some(FileWallet),
            Some("river"),
            None,
            Some("river_csv"),
            None,
            None,
        ),
        "bullbitcoin" => m(
            Some(FileEnrichment),
            Some("bullbitcoin"),
            None,
            Some("bullbitcoin_csv"),
            None,
            None,
        ),
        "pocket-bitcoin" => m(
            Some(FileEnrichment),
            Some("pocketbitcoin"),
            None,
            Some("pocketbitcoin_csv"),
            None,
            None,
        ),
        "strike" => m(
            Some(FileWallet),
            Some("strike"),
            None,
            Some("strike_csv"),
            None,
            None,
        ),
        "21bitcoin" => m(
            Some(FileWallet),
            Some("21bitcoin"),
            None,
            Some("21bitcoin_csv"),
            None,
            None,
        ),
        "coinfinity" => m(
            Some(FileEnrichment),
            Some("coinfinity"),
            None,
            Some("coinfinity_csv"),
            None,
            None,
        ),
        "bitpanda" => m(Some(ExchangeApi), None, Some("bitpanda"), None, None, None),
        "kraken" => m(Some(ExchangeApi), None, Some("kraken"), None, None, None),
        "coinbase" => m(Some(ExchangeApi), None, Some("coinbase"), None, None, None),
        "binance-api" => m(Some(ExchangeApi), None, Some("binance"), None, None, None),
        "binance" => m(
            Some(FileEnrichment),
            Some("binance"),
            None,
            Some("binance_supplemental_csv"),
            None,
            None,
        ),
        "prior-tax-report" => m(Some(PriorReport), None, None, None, None, None),
        "cointracking" => m(
            Some(FileWallet),
            Some("cointracking"),
            None,
            Some("cointracking_csv"),
            None,
            None,
        ),
        "blockpit" => m(
            Some(FileWallet),
            Some("blockpit"),
            None,
            Some("blockpit_csv"),
            None,
            None,
        ),
        "generic-ledger" | "custom-import" => m(
            Some(FileWallet),
            Some("custom"),
            None,
            Some("generic_ledger"),
            None,
            None,
        ),
        "csv" => m(
            Some(FileWallet),
            Some("custom"),
            None,
            Some("csv"),
            None,
            None,
        ),
        "bip329" => m(Some(Bip329), None, None, None, None, None),
        _ => Meta::default(),
    }
}

impl Meta {
    /// `setupKind ?? "planned"`.
    pub fn kind(&self) -> SetupKind {
        self.setup.unwrap_or(SetupKind::Planned)
    }
}

pub fn source(id: &str) -> Option<&'static Source> {
    SOURCES.iter().find(|s| s.id == id)
}

/// `isExchangeEvidenceFormat`.
pub fn is_exchange_evidence(fmt: Option<&str>) -> bool {
    matches!(
        fmt,
        Some(
            "bullbitcoin_csv" | "coinfinity_csv" | "pocketbitcoin_csv" | "binance_supplemental_csv"
        )
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_ready_catalog_source_has_a_setup_or_forward() {
        for s in SOURCES {
            let m = meta(s.id);
            if !s.planned {
                assert!(
                    m.setup.is_some() || m.forward_to.is_some(),
                    "{} has no setup",
                    s.id
                );
            }
        }
    }
}
