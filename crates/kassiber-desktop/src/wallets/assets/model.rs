//! importAssets.ts: what an exchange sync or provider import left out by asset, and the wallet
//! setting that decides it (docs/plan/24-crypto-assets.md). A wallet's `asset_scope` is
//! `bitcoin` (the default) or `crypto`; importers report the crypto tickers the `bitcoin` scope
//! skipped, the non-crypto holdings (stocks, ETFs, metals) no scope imports, and the other coins
//! of an exchange Kassiber reads only Bitcoin from (Bitfinex, Bitcoin.de).

use std::collections::BTreeMap;

use serde_json::Value;

/// `AssetScope`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    #[default]
    Bitcoin,
    Crypto,
}

impl Scope {
    /// `isAssetScope`.
    pub fn parse(v: &Value) -> Option<Scope> {
        match v.as_str()? {
            "bitcoin" => Some(Scope::Bitcoin),
            "crypto" => Some(Scope::Crypto),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Scope::Bitcoin => "bitcoin",
            Scope::Crypto => "crypto",
        }
    }
}

/// `ImportAssets`: the `assets` block of an import or sync result.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportAssets {
    pub scope: Scope,
    /// Crypto rows the `bitcoin` scope skipped, by ticker.
    pub skipped: BTreeMap<String, u64>,
    /// Non-crypto rows no scope imports, by ticker.
    pub excluded: BTreeMap<String, u64>,
    /// Crypto rows of an exchange Kassiber reads only Bitcoin from, by ticker: no scope imports
    /// them, so no scope is offered.
    pub unread: BTreeMap<String, u64>,
}

/// Positive whole row counts by ticker; anything else is dropped.
fn counts(v: Option<&Value>) -> BTreeMap<String, u64> {
    v.and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, n)| n.as_u64().filter(|n| *n > 0).map(|n| (k.clone(), n)))
                .collect()
        })
        .unwrap_or_default()
}

/// `readImportAssets`: the `assets` block, or `None` when absent or without a known scope.
pub fn read(v: Option<&Value>) -> Option<ImportAssets> {
    let v = v.filter(|v| v.is_object())?;
    Some(ImportAssets {
        scope: Scope::parse(&v["scope"])?,
        skipped: counts(v.get("skipped")),
        excluded: counts(v.get("excluded")),
        unread: counts(v.get("unread")),
    })
}

/// `TickerSummary`: tickers by row count, most rows first, then by name; and the row total.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TickerSummary {
    pub tickers: Vec<String>,
    pub rows: u64,
}

/// `tickerSummary`.
pub fn ticker_summary(map: &BTreeMap<String, u64>) -> TickerSummary {
    let mut entries: Vec<(&String, &u64)> = map.iter().filter(|(_, n)| **n > 0).collect();
    entries.sort_by(|(a, l), (b, r)| r.cmp(l).then_with(|| a.cmp(b)));
    TickerSummary {
        tickers: entries.iter().map(|(t, _)| (*t).clone()).collect(),
        rows: entries.iter().map(|(_, n)| **n).sum(),
    }
}

/// `shownTickers`: the tickers to name (at most five) and how many stay unnamed.
pub fn shown_tickers(tickers: &[String]) -> (&[String], usize) {
    let limit = 5.min(tickers.len());
    (&tickers[..limit], tickers.len() - limit)
}

/// `AssetScopeRerun`: what follows turning other crypto assets on. `Sync` syncs the exchange
/// again; `Import` imports the same file again (its path was kept for this import); `PickFile`
/// asks the user to choose the file again. Kassiber never re-reads a file on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rerun {
    Sync,
    Import,
    PickFile,
}

impl Rerun {
    /// The `assets.confirm.trackBody.*` / `trackButton.*` key suffix.
    pub fn key(self) -> &'static str {
        match self {
            Rerun::Sync => "sync",
            Rerun::Import => "import",
            Rerun::PickFile => "pick_file",
        }
    }
}

/// What the notice shows for a wallet's scope and its last import (`AssetScopeNotice`'s early
/// return and its sections).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shown {
    pub skipped: TickerSummary,
    pub excluded: TickerSummary,
    pub unread: TickerSummary,
    /// The `bitcoin` scope skipped crypto rows: offer to track them.
    pub offer_tracking: bool,
    pub pairs: u64,
}

impl Shown {
    pub fn of(scope: Scope, assets: Option<&ImportAssets>, swap_pairs: Option<u64>) -> Shown {
        let sum = |f: fn(&ImportAssets) -> &BTreeMap<String, u64>| {
            assets.map(|a| ticker_summary(f(a))).unwrap_or_default()
        };
        let skipped = sum(|a| &a.skipped);
        Shown {
            offer_tracking: scope == Scope::Bitcoin && skipped.rows > 0,
            skipped,
            excluded: sum(|a| &a.excluded),
            unread: sum(|a| &a.unread),
            pairs: swap_pairs.unwrap_or(0),
        }
    }

    /// Nothing to say: no offer, a `bitcoin` scope, nothing excluded or unread, no swaps.
    pub fn is_empty(&self, scope: Scope) -> bool {
        !self.offer_tracking
            && scope != Scope::Crypto
            && self.excluded.rows == 0
            && self.unread.rows == 0
            && self.pairs == 0
    }
}

/// `ImportReplay`: the arguments of the file import the user just ran, kept so that turning
/// other crypto assets on can import the same file again. Only a path the user chose for this
/// import is kept; without one the user picks the file again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Replay {
    pub wallet: String,
    pub source_format: String,
    pub source_file: String,
}

impl Replay {
    /// The `ui.wallets.import_file` arguments.
    pub fn args(&self) -> Value {
        serde_json::json!({
            "wallet": self.wallet,
            "source_format": self.source_format,
            "source_file": self.source_file,
        })
    }

    /// A replay with a path to read, if any.
    pub fn usable(r: Option<&Replay>) -> Option<&Replay> {
        r.filter(|r| !r.source_file.trim().is_empty())
    }
}

/// The acknowledgement a plaintext book's wallet changes ask for.
pub const PLAINTEXT_CHANGE_ACK: &str = "CHANGE LOCAL DATA";

/// Why a confirmation cannot be sent yet: the missing passphrase or acknowledgement, as an
/// `assets.confirm.*` key.
pub fn auth_issue(encrypted: bool, secret: &str) -> Option<&'static str> {
    if encrypted && secret.is_empty() {
        Some("errorPassphrase")
    } else if !encrypted && secret.trim() != PLAINTEXT_CHANGE_ACK {
        Some("errorAck")
    } else {
        None
    }
}

/// The `ui.wallets.update` arguments that store `scope` (`App::send` binds them to the open
/// book with `expected_scope`).
pub fn update_args(wallet: &str, scope: Scope, encrypted: bool, secret: &str) -> Value {
    let auth = if encrypted {
        serde_json::json!({"passphrase_secret": secret})
    } else {
        serde_json::json!({"plaintext_change_ack": PLAINTEXT_CHANGE_ACK})
    };
    serde_json::json!({
        "wallet": wallet,
        "asset_scope": scope.as_str(),
        "auth_response": auth,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_only_a_known_scope_and_positive_counts() {
        assert_eq!(read(None), None);
        assert_eq!(read(Some(&json!({"scope": "all"}))), None);
        assert_eq!(read(Some(&json!([1]))), None);
        let a = read(Some(&json!({
            "scope": "bitcoin",
            "skipped": {"ETH": 3, "SOL": 0, "BAD": "2", "NEG": -1},
            "excluded": {"AAPL": 2},
            "next_step": "kassiber wallets update ..."
        })))
        .unwrap();
        assert_eq!(a.scope, Scope::Bitcoin);
        assert_eq!(a.skipped, BTreeMap::from([("ETH".to_owned(), 3)]));
        assert_eq!(a.excluded, BTreeMap::from([("AAPL".to_owned(), 2)]));
        assert!(a.unread.is_empty());
    }

    #[test]
    fn tickers_sort_by_rows_then_name_and_name_five() {
        let map: BTreeMap<String, u64> = [("SOL", 2), ("ETH", 5), ("ADA", 2), ("DOT", 1)]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect();
        let s = ticker_summary(&map);
        assert_eq!(s.tickers, ["ETH", "ADA", "SOL", "DOT"]);
        assert_eq!(s.rows, 10);
        let many: Vec<String> = (0..7).map(|i| format!("T{i}")).collect();
        let (shown, more) = shown_tickers(&many);
        assert_eq!((shown.len(), more), (5, 2));
        assert_eq!(shown_tickers(&many[..2]).1, 0);
    }

    #[test]
    fn the_notice_offers_tracking_only_for_a_bitcoin_scope_that_skipped_rows() {
        let a = read(Some(&json!({"scope": "bitcoin", "skipped": {"ETH": 3}}))).unwrap();
        let s = Shown::of(Scope::Bitcoin, Some(&a), None);
        assert!(s.offer_tracking && !s.is_empty(Scope::Bitcoin));
        // A crypto wallet says so (and offers the way back) even with nothing skipped.
        let c = Shown::of(Scope::Crypto, None, None);
        assert!(!c.offer_tracking && !c.is_empty(Scope::Crypto));
        // Nothing to say for a Bitcoin wallet whose import skipped nothing.
        assert!(Shown::of(Scope::Bitcoin, None, Some(0)).is_empty(Scope::Bitcoin));
        // Swaps alone are worth a line; so are unread coins, with no offer.
        assert!(!Shown::of(Scope::Bitcoin, None, Some(2)).is_empty(Scope::Bitcoin));
        let u = read(Some(&json!({"scope": "bitcoin", "unread": {"ETH": 4}}))).unwrap();
        let s = Shown::of(Scope::Bitcoin, Some(&u), None);
        assert!(!s.offer_tracking && s.unread.rows == 4 && !s.is_empty(Scope::Bitcoin));
    }

    #[test]
    fn a_scope_change_asks_for_the_passphrase_or_the_typed_acknowledgement() {
        assert_eq!(auth_issue(true, ""), Some("errorPassphrase"));
        assert_eq!(auth_issue(true, "pw"), None);
        assert_eq!(auth_issue(false, "change local data"), Some("errorAck"));
        assert_eq!(auth_issue(false, " CHANGE LOCAL DATA "), None);
        assert_eq!(
            update_args("w1", Scope::Crypto, true, "pw"),
            json!({"wallet": "w1", "asset_scope": "crypto",
                   "auth_response": {"passphrase_secret": "pw"}})
        );
        assert_eq!(
            update_args("w1", Scope::Bitcoin, false, "ignored")["auth_response"],
            json!({"plaintext_change_ack": "CHANGE LOCAL DATA"})
        );
    }

    #[test]
    fn only_a_kept_path_replays_an_import() {
        let r = Replay {
            wallet: "Bitpanda".into(),
            source_format: "cointracking_csv".into(),
            source_file: "/tmp/x.csv".into(),
        };
        assert_eq!(Replay::usable(Some(&r)), Some(&r));
        assert_eq!(
            r.args(),
            json!({"wallet": "Bitpanda", "source_format": "cointracking_csv", "source_file": "/tmp/x.csv"})
        );
        let blank = Replay {
            source_file: " ".into(),
            ..r
        };
        assert_eq!(Replay::usable(Some(&blank)), None);
        assert_eq!(Replay::usable(None), None);
    }
}
