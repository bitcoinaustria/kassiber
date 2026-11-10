//! `kassiber/btcpay/payment_methods.py`: BTCPay payment-method ids, their
//! rails and what each means for bookkeeping. Inputs are `str(value or "")`.
//! The Bark plugin's `BTC-ARK` was `BARK` before plugin 0.10, and Kassiber's
//! earlier canonical form of that bare id was `BARK-CHAIN`: both map to it.
use kassiber_envelope::{dict, s, AppError};
use kassiber_pyjson::Value;

pub const WALLET_HISTORY_IDS: [&str; 3] = ["BTC-CHAIN", "LBTC-CHAIN", "BTC-ARK"];
pub const BARK_PAYMENT_METHOD_ID: &str = "BTC-ARK";
/// Plugins before 0.10 named it `BARK`, which canonicalizes to this.
const LEGACY_BARK_ID: &str = "BARK-CHAIN";
pub const RAIL_ONCHAIN: &str = "onchain";
pub const RAIL_LIGHTNING: &str = "lightning";
pub const RAIL_LNURL: &str = "lnurl";
pub const RAIL_PLUGIN: &str = "plugin";

fn handler_rail(handler: &str) -> Option<&'static str> {
    Some(match handler {
        "CHAIN" | "ONCHAIN" | "BTCLIKE" => RAIL_ONCHAIN,
        "LN" | "LIGHTNINGNETWORK" | "LIGHTNINGLIKE" => RAIL_LIGHTNING,
        "LNURL" | "LNURLPAY" => RAIL_LNURL,
        _ => return None,
    })
}

fn canonical_handler(rail: &str) -> &'static str {
    match rail {
        RAIL_ONCHAIN => "CHAIN",
        RAIL_LIGHTNING => "LN",
        _ => "LNURL",
    }
}

pub fn settlement_hint(key: &str) -> &'static str {
    match key {
        "btc_onchain" => "Customers pay into this store's on-chain wallet. Kassiber can import BTCPay's wallet history (needs the wallet-history key) or recognise the watch-only wallet you already track for this store.",
        "liquid_onchain" => "Customers pay into this store's Liquid wallet. Kassiber imports it the same way as on-chain Bitcoin, or maps it to a Liquid wallet you track.",
        "lightning" => "Lightning payments settle in the node or wallet configured for this store: your own LND or Core Lightning node, or a plugin such as Boltz (Liquid), Breez, Blink, Strike, or Nostr Wallet Connect. Add that node or wallet as its own connection; Kassiber links BTCPay payments to it by payment hash.",
        "lnurl" => "LNURL and Lightning Address payments settle through the same Lightning connection as BOLT11 invoices. Kassiber keeps them as invoice records and links them to your Lightning node or wallet by payment hash.",
        "bark" => "Payments settle into the store's Bark (Ark) wallet. With Bark plugin 0.10 or later and a key that can view the wallet, Kassiber imports the whole wallet, fees included; older plugins book BTCPay's settled payments and payouts instead.",
        "plugin" => "This payment method comes from a BTCPay plugin Kassiber cannot import directly. Invoices stay available as provenance; add the account that receives the funds separately.",
        _ => "Kassiber tracks Bitcoin and Liquid Bitcoin only. Payments in this currency are skipped; invoices can still be kept as provenance.",
    }
}

/// `split_payment_method_id`.
pub fn split(value: &str) -> (String, String) {
    let raw = kassiber_pytext::strip(value);
    if raw.is_empty() {
        return (String::new(), String::new());
    }
    for separator in ['-', '_'] {
        if let Some((currency, handler)) = raw.split_once(separator) {
            return (
                kassiber_pytext::upper(kassiber_pytext::strip(currency)),
                kassiber_pytext::upper(kassiber_pytext::strip(handler)).replace(['-', '_'], ""),
            );
        }
    }
    (kassiber_pytext::upper(raw), String::new())
}

/// `canonical_payment_method_id`: legacy and differently-cased ids onto 2.x names.
pub fn canonical(value: &str) -> String {
    let canonical = canonical_id(value);
    if canonical == LEGACY_BARK_ID {
        BARK_PAYMENT_METHOD_ID.into()
    } else {
        canonical
    }
}

fn canonical_id(value: &str) -> String {
    let raw = kassiber_pytext::strip(value);
    if raw.is_empty() {
        return String::new();
    }
    let (currency, handler) = split(raw);
    if handler.is_empty() {
        let short = currency.chars().count() <= 5;
        let alpha = !currency.is_empty()
            && currency
                .chars()
                .all(|c| kassiber_pytext::is_alpha(c.into()));
        return if short && alpha {
            format!("{currency}-CHAIN")
        } else {
            kassiber_pytext::upper(raw)
        };
    }
    match handler_rail(&handler) {
        Some(rail) => format!("{currency}-{}", canonical_handler(rail)),
        None => kassiber_pytext::upper(raw),
    }
}

/// One `classify_payment_method` result.
pub struct Classified {
    pub payment_method_id: String,
    pub currency: String,
    pub rail: &'static str,
    pub chain: Option<&'static str>,
    pub bitcoin_asset: bool,
    pub wallet_history_supported: bool,
    pub settlement: &'static str,
    pub ledger_group: Option<String>,
    pub label: String,
}

impl Classified {
    pub fn to_value(&self) -> Value {
        let text = |v: &Option<String>| v.as_deref().map_or(Value::Null, s);
        dict([
            ("ledger_group", text(&self.ledger_group)),
            ("payment_method_id", s(self.payment_method_id.as_str())),
            ("currency", s(self.currency.as_str())),
            ("rail", s(self.rail)),
            ("chain", self.chain.map_or(Value::Null, s)),
            ("bitcoin_asset", Value::Bool(self.bitcoin_asset)),
            (
                "wallet_history_supported",
                Value::Bool(self.wallet_history_supported),
            ),
            ("provenance_supported", Value::Bool(true)),
            ("settlement", s(self.settlement)),
            ("settlement_hint", s(settlement_hint(self.settlement))),
            ("label", s(self.label.as_str())),
        ])
    }
}

fn bitcoin_chain(currency: &str) -> Option<&'static str> {
    match currency {
        "BTC" => Some("bitcoin"),
        "LBTC" => Some("liquid"),
        _ => None,
    }
}

/// `classify_payment_method`.
pub fn classify(value: &str) -> Classified {
    let id = canonical(value);
    let (currency, handler) = split(&id);
    let rail = if handler.is_empty() {
        RAIL_PLUGIN
    } else {
        handler_rail(&handler).unwrap_or(RAIL_PLUGIN)
    };
    let chain = bitcoin_chain(&currency);
    let mut bitcoin_asset = chain.is_some();
    if [RAIL_LIGHTNING, RAIL_LNURL].contains(&rail) && currency != "BTC" {
        bitcoin_asset = false;
    }
    let settlement = if !bitcoin_asset {
        if rail != RAIL_PLUGIN {
            "non_bitcoin"
        } else {
            "plugin"
        }
    } else if rail == RAIL_ONCHAIN {
        if chain == Some("liquid") {
            "liquid_onchain"
        } else {
            "btc_onchain"
        }
    } else if rail == RAIL_LIGHTNING {
        "lightning"
    } else if rail == RAIL_LNURL {
        "lnurl"
    } else if id == BARK_PAYMENT_METHOD_ID {
        "bark"
    } else {
        "plugin"
    };
    Classified {
        ledger_group: ledger_group_for(&id, rail, bitcoin_asset),
        wallet_history_supported: WALLET_HISTORY_IDS.contains(&id.as_str()),
        label: label_for(&id, rail, &currency),
        chain: chain.filter(|_| bitcoin_asset),
        payment_method_id: id,
        currency,
        rail,
        bitcoin_asset,
        settlement,
    }
}

fn ledger_group_for(id: &str, rail: &str, bitcoin_asset: bool) -> Option<String> {
    if !bitcoin_asset || rail == RAIL_ONCHAIN {
        None
    } else if [RAIL_LIGHTNING, RAIL_LNURL].contains(&rail) {
        Some("lightning".into())
    } else {
        Some(canonical(id))
    }
}

/// `ledger_group(payment_method_id)`.
pub fn ledger_group(value: &str) -> Option<String> {
    classify(value).ledger_group
}

fn label_for(id: &str, rail: &str, currency: &str) -> String {
    if id == BARK_PAYMENT_METHOD_ID {
        return "Bark (Ark)".into();
    }
    match (currency, rail) {
        ("BTC", RAIL_ONCHAIN) => "Bitcoin on-chain".into(),
        ("BTC", RAIL_LIGHTNING) => "Lightning".into(),
        ("BTC", RAIL_LNURL) => "LNURL / Lightning Address".into(),
        ("LBTC", RAIL_ONCHAIN) => "Liquid Bitcoin".into(),
        (_, RAIL_ONCHAIN) => format!("{currency} on-chain"),
        (_, RAIL_PLUGIN) => format!("{id} (plugin)"),
        _ => id.to_owned(),
    }
}

/// `is_wallet_history_payment_method`.
pub fn is_wallet_history(value: &str) -> bool {
    WALLET_HISTORY_IDS.contains(&canonical(value).as_str())
}

/// `require_wallet_history_payment_method`; `shown` is `str(original)`.
pub fn require_wallet_history(value: &str, shown: &str) -> Result<String, AppError> {
    if is_wallet_history(value) {
        return Ok(canonical(value));
    }
    let rail = (!value.is_empty()).then(|| classify(value).rail);
    let hint = if rail.is_some_and(|r| [RAIL_LIGHTNING, RAIL_LNURL].contains(&r)) {
        "Lightning payments cannot be imported as wallet history. Keep this method as invoice provenance and add the Lightning node or wallet that receives the payments as its own connection."
    } else {
        "Use an on-chain method such as BTC-CHAIN or LBTC-CHAIN, or keep this method as invoice provenance only."
    };
    Err(AppError::new(
        "validation",
        format!("BTCPay payment method '{shown}' is not available through wallet-history sync"),
    )
    .with_hint(hint))
}

/// `require_settlement_payment_method`: an on-chain method whose provenance
/// may enrich a wallet Kassiber tracks.
pub fn require_settlement(value: &str, shown: &str) -> Result<String, AppError> {
    let method = require_wallet_history(value, shown)?;
    if method == BARK_PAYMENT_METHOD_ID {
        return Err(AppError::new(
            "validation",
            format!("BTCPay payment method '{shown}' cannot be mapped to another wallet"),
        )
        .with_hint("Bark payments settle into the store's own Bark wallet. Import it as wallet history (Bark plugin 0.10 or later) or book it as a payment ledger."));
    }
    Ok(method)
}

/// `asset_for_payment_method`.
pub fn asset(value: &str) -> Option<String> {
    let (currency, _) = split(&canonical(value));
    (!currency.is_empty()).then_some(currency)
}
