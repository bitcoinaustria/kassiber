//! The Wallets area's view of a connection: every field `ui.overview.snapshot` sends for a
//! `Connection` (mocks/seed.ts), plus the display helpers of lib/connectionDisplay.ts,
//! lib/lightning.ts and components/kb/wallets/format.ts.
//!
//! The shared `model::Connection` keeps only what the Overview needs, so this area parses the
//! raw snapshot reply (`app.read("ui.overview.snapshot")`) itself.

use rune::Color;
use serde_json::Value;

use crate::app::App;
use crate::theme::Tk;

/// One `Connection` of the overview snapshot (camelCase keys as the daemon sends them).
#[derive(Clone, Debug, Default)]
pub struct Conn {
    pub id: String,
    pub kind: String,
    /// "wallet" | "backend" (absent → wallet).
    pub role: Option<String>,
    pub chain: Option<String>,
    pub label: String,
    pub last: String,
    pub last_sync_at: Option<String>,
    pub last_transaction_at: Option<String>,
    pub asset: Option<String>,
    pub network: Option<String>,
    pub policy_asset: Option<String>,
    pub payment_method_id: Option<String>,
    pub observed_assets: Vec<String>,
    /// BTC (float).
    pub balance: f64,
    /// "synced" | "syncing" | "idle" | "error".
    pub status: String,
    pub sync_mode: Option<String>,
    pub sync_source: Option<String>,
    pub source_format: Option<String>,
    pub deprecated: bool,
    pub backend_kind: Option<String>,
    pub endpoint: Option<String>,
    pub is_default_backend: bool,
    pub wallet_refs: Vec<String>,
    pub transaction_count: Option<f64>,
    pub addresses: Option<f64>,
    pub gap: Option<f64>,
    pub channels: Option<f64>,
    /// `lightningCapabilities` as sent (an object), when present.
    pub lightning_capabilities: Option<Value>,
    /// `node` (a `NodeSnapshot`), when the snapshot carries one.
    pub node: Option<Value>,
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_owned)
}
fn f(v: &Value, k: &str) -> Option<f64> {
    v.get(k).and_then(Value::as_f64)
}

impl Conn {
    pub fn parse(v: &Value) -> Conn {
        Conn {
            id: s(v, "id").unwrap_or_default(),
            kind: s(v, "kind").unwrap_or_default(),
            role: s(v, "role"),
            chain: s(v, "chain"),
            label: s(v, "label").unwrap_or_default(),
            last: s(v, "last").unwrap_or_default(),
            last_sync_at: s(v, "lastSyncAt"),
            last_transaction_at: s(v, "lastTransactionAt"),
            asset: s(v, "asset"),
            network: s(v, "network"),
            policy_asset: s(v, "policyAsset"),
            payment_method_id: s(v, "paymentMethodId"),
            observed_assets: v
                .get("observedAssets")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
            balance: f(v, "balance").unwrap_or(0.0),
            status: s(v, "status").unwrap_or_else(|| "idle".into()),
            sync_mode: s(v, "syncMode"),
            sync_source: s(v, "syncSource"),
            source_format: s(v, "sourceFormat"),
            deprecated: v
                .get("deprecated")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            backend_kind: s(v, "backendKind"),
            endpoint: s(v, "endpoint"),
            is_default_backend: v
                .get("isDefaultBackend")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            wallet_refs: v
                .get("walletRefs")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
            transaction_count: f(v, "transactionCount"),
            addresses: f(v, "addresses"),
            gap: f(v, "gap"),
            channels: f(v, "channels"),
            lightning_capabilities: v
                .get("lightningCapabilities")
                .filter(|x| x.is_object())
                .cloned(),
            node: v.get("node").filter(|x| x.is_object()).cloned(),
        }
    }

    pub fn is_backend(&self) -> bool {
        self.role.as_deref() == Some("backend") || self.kind == "backend"
    }

    /// `connectionSupportsLightningCapability(connection, cap)` (lib/lightning.ts): the declared
    /// capabilities, else the legacy node kinds (core-ln/coreln/lnd) get nodeSnapshot and
    /// routingProfitability.
    pub fn lightning_capability(&self, cap: &str) -> bool {
        if let Some(c) = &self.lightning_capabilities {
            return c.get(cap).and_then(Value::as_bool) == Some(true);
        }
        matches!(self.kind.as_str(), "core-ln" | "coreln" | "lnd")
            && matches!(cap, "nodeSnapshot" | "routingProfitability")
    }

    /// `isNodeConnection` (routes/ConnectionDetail.tsx): renders `NodeConnectionDetail`.
    pub fn is_node(&self) -> bool {
        self.lightning_capability("nodeSnapshot")
    }

    /// `connectionCategoryLabel`: the chain wins over the kind (a Liquid descriptor is "Liquid").
    pub fn category_label(&self) -> &'static str {
        if self.is_backend() {
            return "Infrastructure";
        }
        if self.chain.as_deref() == Some("liquid") {
            return "Liquid";
        }
        kind_category_label(&self.kind)
    }

    /// `connectionCategorySortRank`.
    pub fn category_sort_rank(&self) -> usize {
        let c = self.category_label();
        CATEGORY_ORDER
            .iter()
            .position(|x| *x == c)
            .unwrap_or(CATEGORY_ORDER.len())
    }

    /// `connectionTypeLabel`.
    pub fn type_label(&self) -> String {
        if self.is_backend() {
            return self
                .sync_source
                .as_deref()
                .map(str::trim)
                .filter(|x| !x.is_empty())
                .unwrap_or("Backend endpoint")
                .to_owned();
        }
        if let Some(fmt) = self
            .source_format
            .as_deref()
            .map(str::trim)
            .filter(|x| !x.is_empty())
        {
            return source_format_label(fmt)
                .map(str::to_owned)
                .unwrap_or_else(|| fmt.to_owned());
        }
        if self.sync_mode.as_deref() == Some("btcpay")
            || self.sync_source.as_deref() == Some("btcpay")
        {
            return match self.payment_method_id.as_deref().filter(|x| !x.is_empty()) {
                Some(p) => format!("BTCPay API · {p}"),
                None => "BTCPay API".into(),
            };
        }
        match self.kind.as_str() {
            "xpub" | "descriptor" => "Wallet export",
            "silent-payment" => "Silent Payments watch-only",
            "address" => "Address list",
            "samourai" => "Samourai watch-only",
            "core-ln" => "Core Lightning API",
            "lnd" => "LND API",
            "nwc" => "NWC API",
            "cashu" => "Cashu wallet",
            "btcpay" => "BTCPay API",
            "csv" => "Generic CSV",
            "bip329" => "BIP329 labels",
            "custom" => "Custom source",
            k => kind_label(k),
        }
        .to_owned()
    }

    /// `connectionAssetLabel`: "BTC" | "LBTC" | "LN-BTC".
    pub fn asset_label(&self) -> &'static str {
        let signals = [
            self.asset.as_deref(),
            self.chain.as_deref(),
            self.network.as_deref(),
            self.policy_asset.as_deref(),
            self.payment_method_id.as_deref(),
            self.source_format.as_deref(),
            self.sync_mode.as_deref(),
            self.sync_source.as_deref(),
            Some(self.label.as_str()),
        ];
        if has_signal(&signals, &["liquid", "liquidv1", "lbtc", "l-btc"]) {
            return "LBTC";
        }
        if matches!(self.kind.as_str(), "core-ln" | "lnd" | "nwc" | "phoenix") {
            return "LN-BTC";
        }
        if has_signal(
            &signals,
            &["lightning", "ln-btc", "btc-ln", "btc_lightning"],
        ) {
            return "LN-BTC";
        }
        for raw in &self.observed_assets {
            match raw.trim().to_uppercase().as_str() {
                "LBTC" | "L-BTC" => return "LBTC",
                "BTC" | "SATS" => return "BTC",
                _ => {}
            }
        }
        "BTC"
    }
}

fn has_signal(values: &[Option<&str>], signals: &[&str]) -> bool {
    values.iter().flatten().any(|v| {
        let n = v.trim().to_lowercase();
        !n.is_empty()
            && signals.iter().any(|s| {
                n.split(|c: char| !c.is_ascii_alphanumeric())
                    .any(|p| p == *s)
                    || n.contains(s)
            })
    })
}

pub const CATEGORY_ORDER: [&str; 12] = [
    "On-chain",
    "Liquid",
    "Lightning",
    "NWC",
    "Ecash",
    "BTCPay",
    "Exchange",
    "Custodial platform",
    "CSV",
    "BIP329",
    "Custom",
    "Infrastructure",
];

/// A category label ([`CATEGORY_ORDER`], the filter's and sort's stable id) in the UI language.
pub fn category_display(label: &str) -> String {
    let key = match label {
        "On-chain" => "connections:table.category.onChain",
        "Liquid" => "connections:table.category.liquid",
        "Lightning" => "connections:table.category.lightning",
        "NWC" => "connections:table.category.nwc",
        "Ecash" => "connections:table.category.ecash",
        "BTCPay" => "connections:table.category.btcpay",
        "Exchange" => "connections:table.category.exchange",
        "Custodial platform" => "connections:table.category.custodial",
        "CSV" => "connections:table.category.csv",
        "BIP329" => "connections:table.category.bip329",
        "Custom" => "connections:table.category.custom",
        "Infrastructure" => "connections:table.category.infrastructure",
        other => return other.to_owned(),
    };
    crate::i18n::t(key)
}

/// `connectionKindCategoryLabels`.
pub fn kind_category_label(kind: &str) -> &'static str {
    match kind {
        "xpub" | "address" | "descriptor" | "silent-payment" | "samourai" => "On-chain",
        "core-ln" | "lnd" | "phoenix" => "Lightning",
        "nwc" => "NWC",
        "cashu" => "Ecash",
        "btcpay" => "BTCPay",
        "kraken" | "bitstamp" | "coinbase" | "bitpanda" | "bitvavo" | "bitfinex" | "bitcoinde"
        | "river" | "bullbitcoin" | "coinfinity" => "Exchange",
        "strike" => "Custodial platform",
        "csv" => "CSV",
        "bip329" => "BIP329",
        "backend" => "Infrastructure",
        _ => "Custom",
    }
}

/// `connectionKindLabels`.
pub fn kind_label(kind: &str) -> &'static str {
    match kind {
        "xpub" | "descriptor" => "Wallet export",
        "address" => "Address",
        "silent-payment" => "Silent Payments",
        "samourai" => "Samourai",
        "core-ln" => "Core Lightning",
        "lnd" => "LND",
        "nwc" => "NWC",
        "cashu" => "Cashu",
        "btcpay" => "BTCPay",
        "kraken" => "Kraken",
        "bitstamp" => "Bitstamp",
        "coinbase" => "Coinbase",
        "bitpanda" => "Bitpanda",
        "bitvavo" => "Bitvavo",
        "bitfinex" => "Bitfinex",
        "bitcoinde" => "Bitcoin.de",
        "river" => "River",
        "bullbitcoin" => "Bull Bitcoin",
        "coinfinity" => "Coinfinity",
        "strike" => "Strike",
        "phoenix" => "Phoenix",
        "csv" => "CSV",
        "bip329" => "BIP329",
        "backend" => "Backend",
        _ => "Custom",
    }
}

/// `sourceFormatLabels`.
pub fn source_format_label(fmt: &str) -> Option<&'static str> {
    Some(match fmt {
        "csv" => "Generic CSV",
        "json" => "Generic JSON",
        "phoenix_csv" => "Phoenix CSV",
        "river_csv" => "River CSV",
        "bullbitcoin_csv" => "Bull Bitcoin CSV",
        "bullbitcoin_wallet_csv" => "Bull Bitcoin Wallet CSV",
        "coinfinity_csv" => "Coinfinity CSV",
        "pocketbitcoin_csv" => "Pocket Bitcoin CSV",
        "21bitcoin_csv" => "21bitcoin CSV",
        "strike_csv" => "Strike CSV",
        "bitstamp_csv" => "Bitstamp CSV",
        "ledgerlive_csv" => "Ledger Live CSV",
        "binance_supplemental_csv" => "Binance supplemental CSV",
        "binance_history_csv" => "Binance Transaction History CSV",
        "wasabi_bundle" => "Wasabi export",
        "generic_ledger" => "Generic ledger",
        _ => return None,
    })
}

/// Every connection of the current overview snapshot (raw reply), in the daemon's order.
pub fn all(app: &App) -> Vec<Conn> {
    app.read("ui.overview.snapshot")
        .and_then(|d| d.get("connections"))
        .and_then(Value::as_array)
        .map(|a| a.iter().map(Conn::parse).collect())
        .unwrap_or_default()
}

/// The connection with `id`, from the current snapshot.
pub fn find(app: &App, id: &str) -> Option<Conn> {
    app.read("ui.overview.snapshot")?
        .get("connections")?
        .as_array()?
        .iter()
        .find(|c| c.get("id").and_then(Value::as_str) == Some(id))
        .map(Conn::parse)
}

/// The raw overview snapshot (`data`), if loaded.
pub fn snapshot(app: &App) -> Option<&Value> {
    app.read("ui.overview.snapshot")
}

/// `statusDotStyles` (bg-emerald-500 / amber-500 / muted-foreground/50 / red-500).
pub fn status_dot(tk: &Tk, status: &str) -> Color {
    let h = |s: &str| rune::color::hex(s).expect("hex");
    match status {
        "synced" => h("#10b981"),
        "syncing" => h("#f59e0b"),
        "error" => h("#ef4444"),
        _ => tk.muted_fg.alpha(0.5),
    }
}

/// `connectionStatusStyles`: (fill, text, ring) per status and theme.
pub fn status_style(tk: &Tk, status: &str) -> (Color, Color, Color) {
    let h = |s: &str| rune::color::hex(s).expect("hex");
    match (status, tk.dark) {
        ("synced", false) => (h("#ecfdf5"), h("#047857"), h("#059669").alpha(0.2)),
        ("synced", true) => (
            h("#064e3b").alpha(0.3),
            h("#34d399"),
            h("#34d399").alpha(0.2),
        ),
        ("syncing", false) => (h("#fffbeb"), h("#b45309"), h("#d97706").alpha(0.2)),
        ("syncing", true) => (
            h("#78350f").alpha(0.3),
            h("#fbbf24"),
            h("#fbbf24").alpha(0.2),
        ),
        ("error", false) => (h("#fef2f2"), h("#b91c1c"), h("#dc2626").alpha(0.2)),
        ("error", true) => (
            h("#7f1d1d").alpha(0.3),
            h("#f87171"),
            h("#f87171").alpha(0.2),
        ),
        _ => (tk.muted, tk.muted_fg, tk.border),
    }
}
