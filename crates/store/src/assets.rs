//! Which stored asset codes are Bitcoin, and when two assets' raw quantities
//! can be compared (docs/plan/23-crypto-assets.md).
//!
//! Every crypto asset is stored in the same integer unit (10^-11 of a whole
//! unit; msat for BTC), so two quantities always subtract, but only Bitcoin
//! and Liquid Bitcoin share a value. A pair between ETH and BTC is sized by
//! its own conversion relation; the difference of its raw quantities (a
//! pair's stored `swap_fee_msat`) means nothing and must not be shown as a
//! fee or added to a Bitcoin total.

/// The codes Bitcoin views and totals count: on-chain and Liquid Bitcoin.
pub const BITCOIN: [&str; 3] = ["BTC", "LBTC", "L-BTC"];

/// Whether a stored asset code is Bitcoin (BTC or Liquid BTC). Case and
/// surrounding spaces are ignored, as stored codes are compared elsewhere.
pub fn is_bitcoin(asset: &str) -> bool {
    let code = asset.trim().to_ascii_uppercase();
    BITCOIN.contains(&code.as_str())
}

/// Whether two assets' raw quantities measure the same thing: the same
/// asset, or both Bitcoin (a BTC/L-BTC peg or swap). Only then is the
/// difference of a pair's two legs a fee.
pub fn units_comparable(left: &str, right: &str) -> bool {
    let (l, r) = (
        left.trim().to_ascii_uppercase(),
        right.trim().to_ascii_uppercase(),
    );
    l == r || (is_bitcoin(&l) && is_bitcoin(&r))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitcoin_family_is_btc_and_liquid_btc_only() {
        for code in ["BTC", "btc", " LBTC ", "L-BTC"] {
            assert!(is_bitcoin(code), "{code}");
        }
        for code in ["ETH", "USDT", "", "SATS", "WBTC"] {
            assert!(!is_bitcoin(code), "{code}");
        }
    }

    #[test]
    fn only_the_same_asset_or_two_bitcoins_compare() {
        assert!(units_comparable("BTC", "BTC"));
        assert!(units_comparable("BTC", "LBTC"));
        assert!(units_comparable("ETH", "eth"));
        assert!(!units_comparable("BTC", "ETH"));
        assert!(!units_comparable("ETH", "SOL"));
        assert!(!units_comparable("LBTC", "USDT"));
    }
}
