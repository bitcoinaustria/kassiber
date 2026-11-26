//! The setup fields of `ConnectionSource` that overview/catalog.rs (generated for the picker)
//! does not carry: `status`, `setupKind`, `forwardTo`, `walletKind`, `exchange`, `sourceFormat`,
//! `chain`, `network` — per source id, from ui-tauri/src/lib/connectionCatalog.tsx. The tests
//! below keep the wallet kinds and source formats to the ones the daemon accepts.

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
        "bisq" => m(
            Some(FileWallet),
            Some("bisq"),
            None,
            Some("bisq_csv"),
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
        "bitvavo-api" => m(Some(ExchangeApi), None, Some("bitvavo"), None, None, None),
        "bitstamp-api" => m(Some(ExchangeApi), None, Some("bitstamp"), None, None, None),
        "bitfinex" => m(Some(ExchangeApi), None, Some("bitfinex"), None, None, None),
        "bitcoinde-api" => m(Some(ExchangeApi), None, Some("bitcoinde"), None, None, None),
        "bitvavo" => m(
            Some(FileWallet),
            Some("bitvavo"),
            None,
            Some("bitvavo_csv"),
            None,
            None,
        ),
        "bitstamp" => m(
            Some(FileWallet),
            Some("bitstamp"),
            None,
            Some("bitstamp_csv"),
            None,
            None,
        ),
        "bitfinex-csv" => m(
            Some(FileWallet),
            Some("bitfinex"),
            None,
            Some("bitfinex_csv"),
            None,
            None,
        ),
        "bitcoinde" => m(
            Some(FileWallet),
            Some("bitcoinde"),
            None,
            Some("bitcoinde_csv"),
            None,
            None,
        ),
        "kraken" => m(Some(ExchangeApi), None, Some("kraken"), None, None, None),
        "coinbase" => m(Some(ExchangeApi), None, Some("coinbase"), None, None, None),
        "binance-api" => m(Some(ExchangeApi), None, Some("binance"), None, None, None),
        "binance-history" => m(
            Some(FileWallet),
            Some("binance"),
            None,
            Some("binance_history_csv"),
            None,
            None,
        ),
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
    use std::collections::BTreeSet;

    use super::*;

    fn backend_file(path: &str) -> String {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../").to_owned() + path;
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    /// The wallet kinds the daemon accepts (`crates/wallets/src/connections_catalog.json`).
    fn wallet_kinds() -> BTreeSet<String> {
        let rows: serde_json::Value =
            serde_json::from_str(&backend_file("wallets/src/connections_catalog.json")).unwrap();
        rows.as_array()
            .unwrap()
            .iter()
            .map(|row| row["kind"].as_str().unwrap().to_owned())
            .collect()
    }

    /// The import formats the daemon accepts (`crates/wallets/src/connections.rs`
    /// `SOURCE_FORMATS`).
    fn source_formats() -> BTreeSet<String> {
        let text = backend_file("wallets/src/connections.rs");
        let body = text
            .split_once("pub const SOURCE_FORMATS: &[&str] = &[")
            .expect("SOURCE_FORMATS")
            .1
            .split_once("];")
            .unwrap()
            .0;
        body.split('"')
            .skip(1)
            .step_by(2)
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn ready_sources_reference_known_wallet_kinds_and_formats() {
        let (kinds, formats) = (wallet_kinds(), source_formats());
        assert!(
            kinds.len() > 10 && formats.len() > 10,
            "{kinds:?} {formats:?}"
        );
        let mut checked = 0;
        for s in SOURCES.iter().filter(|s| !s.planned) {
            let m = meta(s.id);
            if let Some(kind) = m.wallet_kind {
                assert!(
                    kinds.contains(kind),
                    "{} references unknown wallet kind {kind}",
                    s.id
                );
            }
            if let Some(format) = m.source_format {
                assert!(
                    formats.contains(format),
                    "{} references unknown source format {format}",
                    s.id
                );
            }
            // Backend-only, label-import, evidence-archive, BTCPay and exchange API sources
            // name no wallet kind; the daemon owns their target. Forwards take their target's.
            use SetupKind::*;
            if !matches!(
                m.setup,
                None | Some(BackendSettings | Bip329 | Btcpay | PriorReport | ExchangeApi)
            ) {
                assert!(m.wallet_kind.is_some(), "{} has no wallet kind", s.id);
                checked += 1;
            }
            if let Some(to) = m.forward_to {
                assert!(
                    source(to).is_some_and(|t| !t.planned),
                    "{} forwards to {to}",
                    s.id
                );
            }
        }
        assert!(checked > 10, "only {checked} wallet sources checked");
    }

    #[test]
    fn exchange_api_sources_are_the_daemons_exchanges() {
        let exchanges: BTreeSet<&str> = SOURCES
            .iter()
            .map(|s| meta(s.id))
            .filter(|m| m.setup == Some(SetupKind::ExchangeApi))
            .filter_map(|m| m.exchange)
            .collect();
        let kinds: BTreeSet<&str> = crate::wallets::add::exchange::model::KINDS
            .into_iter()
            .collect();
        assert_eq!(exchanges, kinds);
    }

    #[test]
    fn btcpay_csv_import_is_manual_file_wallet_source() {
        assert_eq!(meta("btcpay").setup, Some(SetupKind::Btcpay));
        let csv = source("btcpay-csv").expect("manual BTCPay CSV source");
        assert!(!csv.planned);
        let m = meta("btcpay-csv");
        assert_eq!(m.setup, Some(SetupKind::FileWallet));
        assert_eq!(m.wallet_kind, Some("custom"));
        assert_eq!(m.source_format, Some("btcpay_csv"));
        crate::i18n::set_lang(crate::i18n::Lang::En);
        assert!(
            csv.details()
                .iter()
                .any(|d| d.contains("invoice and payment-request provenance")),
            "{:?}",
            csv.details()
        );
        assert!(wallet_kinds().contains("custom"));
    }

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

/// `refreshHintKey`: the key under `connections:coverage.refreshHints` of a file source's short
/// "how to download a newer export" hint (docs/reference/desktop.md#sources-live-sync-and-file-snapshots).
pub fn refresh_hint_key(id: &str) -> Option<&'static str> {
    Some(match id {
        "phoenix" => "phoenix",
        "river" => "river",
        "bullbitcoin" => "bullbitcoin",
        "pocket-bitcoin" => "pocketbitcoin",
        "strike" => "strike",
        "21bitcoin" => "21bitcoin",
        "coinfinity" => "coinfinity",
        "binance" => "binance",
        "cointracking" => "cointracking",
        "blockpit" => "blockpit",
        _ => return None,
    })
}

/// `fileSourceId`: for an exchange API source, the same exchange's file import. Only where the
/// file lands in a wallet of the same kind, so the key can later attach to it instead of
/// double-counting; the Add connection dialog then asks "connect with a key or import a file".
pub fn file_source_id(id: &str) -> Option<&'static str> {
    match id {
        "binance-api" => Some("binance"),
        _ => None,
    }
}

/// `pairedSources`: the API source and the file source of one provider, when the catalog pairs
/// them and both are ready.
pub fn paired_sources(id: &str) -> Option<(&'static str, &'static str)> {
    let ready = |id: &str| {
        crate::overview::catalog::SOURCES
            .iter()
            .any(|s| s.id == id && !s.planned)
    };
    let api = crate::overview::catalog::SOURCES
        .iter()
        .map(|s| s.id)
        .find(|a| *a == id && file_source_id(a).is_some())
        .or_else(|| {
            crate::overview::catalog::SOURCES
                .iter()
                .map(|s| s.id)
                .find(|a| file_source_id(a) == Some(id))
        })?;
    let file = file_source_id(api)?;
    (ready(api) && ready(file)).then_some((api, file))
}

#[cfg(test)]
mod coverage_tests {
    use super::*;

    #[test]
    fn binance_pairs_its_key_and_its_file() {
        assert_eq!(
            paired_sources("binance-api"),
            Some(("binance-api", "binance"))
        );
        assert_eq!(paired_sources("binance"), Some(("binance-api", "binance")));
        assert_eq!(paired_sources("kraken"), None);
        assert_eq!(refresh_hint_key("pocket-bitcoin"), Some("pocketbitcoin"));
        assert_eq!(refresh_hint_key("kraken"), None);
    }
}
