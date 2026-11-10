//! The Connections screen's local catalog (`_connections_sources_payload`,
//! `list_wallet_kinds`). Reads only: no book, configuration or network access.
use kassiber_envelope::{dict, s};
use kassiber_pyjson::Value as Py;

/// `_UI_WALLET_SOURCE_FORMATS`, the import formats the daemon accepts.
pub const SOURCE_FORMATS: &[&str] = &[
    "json",
    "csv",
    "btcpay_json",
    "btcpay_csv",
    "phoenix_csv",
    "river_csv",
    "bullbitcoin_csv",
    "bullbitcoin_wallet_csv",
    "coinfinity_csv",
    "21bitcoin_csv",
    "pocketbitcoin_csv",
    "strike_csv",
    "ledgerlive_csv",
    "binance_supplemental_csv",
    "cointracking_csv",
    "blockpit_csv",
    "wasabi_bundle",
    "generic_ledger",
];

/// `list_wallet_kinds()`: `WALLET_KINDS` with their catalog rows, checked
/// against Python by `tests/test_connection_catalog_drift.py`.
pub fn wallet_kinds() -> Py {
    kassiber_pytext::loads(include_str!("connections_catalog.json")).expect("wallet kind catalog")
}

/// `ui.connections.sources`: wallet kinds plus the sorted source formats.
pub fn sources() -> Py {
    let mut formats = SOURCE_FORMATS.to_vec();
    formats.sort_unstable();
    dict([
        ("wallet_kinds", wallet_kinds()),
        (
            "source_formats",
            Py::List(formats.into_iter().map(s).collect()),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_sort_formats_by_code_point_and_keep_catalog_order() {
        let Py::List(formats) = kassiber_envelope::get(&sources(), "source_formats")
            .unwrap()
            .clone()
        else {
            panic!("formats");
        };
        assert_eq!(formats.len(), SOURCE_FORMATS.len());
        assert_eq!(formats[0], s("21bitcoin_csv"));
        assert_eq!(formats.last(), Some(&s("wasabi_bundle")));
        let Py::List(kinds) = wallet_kinds() else {
            panic!("kinds");
        };
        let first = kassiber_envelope::get(&kinds[0], "kind").unwrap();
        assert_eq!(first, &s("descriptor"));
    }
}
