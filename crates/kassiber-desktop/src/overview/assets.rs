//! The Holdings by Asset model: what the book holds per asset rather than per source.
//!
//! Generic on purpose. Bitcoin rows are bitcoin on one of its layers (on-chain BTC, Lightning,
//! L-BTC), read from the overview snapshot's per-connection balances. Other crypto assets a
//! `crypto`-scoped wallet keeps (docs/plan/24-crypto-assets.md) come from the snapshot's
//! `otherAssets`, one row per asset. Fiat cash and stablecoins slot in as further rows without
//! changing the card: each row carries its own unit, and the grouping keeps every total inside
//! one unit.
//!
//! Rules:
//! - A row's balance is an exact integer of the asset's smallest unit (`units`, `precision`
//!   decimals): msat for bitcoin on every layer (precision 11), cents for EUR (precision 2).
//! - Groups are by denomination. Bitcoin on every layer (native BTC, Lightning, pegged L-BTC) is
//!   one group with a combined BTC total. Each fiat currency is its own group. Stablecoins and
//!   anything else stand alone in "Other assets", which has no total: their units differ.
//! - A fiat value is only ever a price times a quantity the book holds: BTC-denominated rows at
//!   the snapshot's BTC market rate, other crypto assets at the value the daemon priced from
//!   the book's own rates (`otherAssets[].marketValue`), fiat cash at face value when it is the
//!   display currency.
//!   Anything else (no market rate, another fiat currency, an unknown asset) has no value and
//!   the card says "no price"; nothing is guessed or converted.
//! - Shares are within the group, by units, never across groups.

use crate::model::{Rail, Snapshot};

/// msat per BTC.
pub const MSAT_PER_BTC: i128 = 100_000_000_000;

/// What kind of unit an asset's balance is in.
// `Stablecoin` is the model's other mixed-unit path (listed, never totalled), which its tests
// exercise; no book source produces it yet, and removing it would remove that path.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitKind {
    /// The chain's own coin: BTC on-chain and over Lightning.
    NativeCoin,
    /// A 1:1 peg of a native coin on another layer: L-BTC.
    Pegged,
    /// Government currency held as cash or on a bank account.
    Fiat,
    /// A token that tracks a fiat currency. Never summed with that currency.
    Stablecoin,
    /// Anything else (an unknown Liquid asset, a CSV-imported coin).
    Other,
}

/// The unit a group totals in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Denomination {
    /// Bitcoin on any layer, totalled in msat.
    Btc,
    /// One fiat currency (ISO code), totalled in its minor unit.
    Fiat(String),
    /// No shared unit: listed, never totalled.
    Mixed,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssetRow {
    /// A stable id: `BTC`, `BTC-LN`, the Liquid policy asset id, an ISO code.
    pub asset_id: String,
    /// English display name (the view translates the known ones).
    pub name: String,
    /// Ticker as shown: `BTC`, `LN-BTC`, `L-BTC`, `EUR`.
    pub symbol: String,
    /// Decimal places of one `units` step (11 for msat, 2 for cents).
    pub precision: u8,
    pub kind: UnitKind,
    /// The layer logo for bitcoin rows; `None` draws the generic asset glyph.
    pub layer: Option<Rail>,
    /// The balance in the smallest unit, exact.
    pub units: i128,
    /// The value in the display fiat currency, or `None` when there is no price.
    pub fiat_value: Option<f64>,
    /// Percent of the group's units, rounded (set by `build`).
    pub share: f64,
    /// Sample data a scene injected, never the book's: the view badges it.
    pub synthetic: bool,
}

impl AssetRow {
    /// The balance as a decimal number of whole units (display only).
    pub fn amount(&self) -> f64 {
        self.units as f64 / 10f64.powi(self.precision as i32)
    }

    pub fn denomination(&self) -> Denomination {
        match self.kind {
            UnitKind::NativeCoin | UnitKind::Pegged => Denomination::Btc,
            UnitKind::Fiat => Denomination::Fiat(self.symbol.clone()),
            UnitKind::Stablecoin | UnitKind::Other => Denomination::Mixed,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssetGroup {
    pub denomination: Denomination,
    /// Largest first.
    pub rows: Vec<AssetRow>,
    /// The group total in its unit; `None` for `Mixed`.
    pub total_units: Option<i128>,
    /// The group's value in the display currency; `None` unless every row is priced.
    pub total_value: Option<f64>,
}

/// Which bitcoin layer a connection's balance sits on (`connectionAssetLabel` in
/// lib/connectionDisplay.ts): Liquid signals first, then the Lightning kinds and signals, then
/// the observed assets (only once the config says nothing), else on-chain BTC.
pub fn layer_of(kind: &str, signals: &[&str], observed: &[String]) -> Rail {
    let has = |words: &[&str]| {
        signals.iter().any(|v| {
            let v = v.trim().to_lowercase();
            !v.is_empty()
                && words.iter().any(|w| {
                    v.split(|c: char| !c.is_ascii_alphanumeric())
                        .any(|p| p == *w)
                        || v.contains(w)
                })
        })
    };
    if has(&["liquid", "liquidv1", "lbtc", "l-btc"]) {
        return Rail::Liquid;
    }
    if matches!(kind, "core-ln" | "lnd" | "nwc" | "phoenix")
        || has(&["lightning", "ln-btc", "btc-ln", "btc_lightning"])
    {
        return Rail::Lightning;
    }
    for a in observed {
        match a.trim().to_uppercase().as_str() {
            "LBTC" | "L-BTC" => return Rail::Liquid,
            "BTC" | "SATS" => return Rail::Onchain,
            _ => {}
        }
    }
    Rail::Onchain
}

/// One source's balance as the snapshot gives it.
pub struct SourceBalance<'a> {
    pub kind: &'a str,
    pub chain: &'a str,
    pub network: &'a str,
    pub label: &'a str,
    pub policy_asset: Option<&'a str>,
    pub observed: Vec<String>,
    /// BTC as the daemon reports it (a float with at most 8 decimals for chain balances).
    pub balance_btc: f64,
}

/// A BTC float from the daemon as exact msat (the daemon derives it from integer msat, so
/// rounding at msat recovers the integer).
pub fn btc_to_msat(btc: f64) -> i128 {
    if btc.is_finite() {
        (btc * MSAT_PER_BTC as f64).round() as i128
    } else {
        0
    }
}

/// The sources' balances folded into one row per bitcoin layer.
pub fn bitcoin_rows(sources: &[SourceBalance], btc_rate: Option<f64>) -> Vec<AssetRow> {
    let mut onchain = 0i128;
    let mut lightning = 0i128;
    let mut liquid = 0i128;
    let mut policy: Option<String> = None;
    for s in sources {
        let msat = btc_to_msat(s.balance_btc);
        if msat <= 0 {
            continue;
        }
        let signals = [s.chain, s.network, s.policy_asset.unwrap_or(""), s.label];
        match layer_of(s.kind, &signals, &s.observed) {
            Rail::Liquid => {
                liquid += msat;
                if policy.is_none() {
                    policy = s.policy_asset.filter(|p| !p.is_empty()).map(str::to_owned);
                }
            }
            Rail::Lightning => lightning += msat,
            _ => onchain += msat,
        }
    }
    let value = |units: i128| btc_rate.map(|r| units as f64 / MSAT_PER_BTC as f64 * r);
    let row =
        |id: String, name: &str, symbol: &str, kind: UnitKind, layer: Rail, units: i128| AssetRow {
            asset_id: id,
            name: name.into(),
            symbol: symbol.into(),
            precision: 11,
            kind,
            layer: Some(layer),
            units,
            fiat_value: value(units),
            share: 0.0,
            synthetic: false,
        };
    let mut out = Vec::new();
    if onchain > 0 {
        out.push(row(
            "BTC".into(),
            "Bitcoin",
            "BTC",
            UnitKind::NativeCoin,
            Rail::Onchain,
            onchain,
        ));
    }
    if lightning > 0 {
        out.push(row(
            "BTC-LN".into(),
            "Lightning",
            "LN-BTC",
            UnitKind::NativeCoin,
            Rail::Lightning,
            lightning,
        ));
    }
    if liquid > 0 {
        out.push(row(
            policy.unwrap_or_else(|| "L-BTC".into()),
            "Liquid Bitcoin",
            "L-BTC",
            UnitKind::Pegged,
            Rail::Liquid,
            liquid,
        ));
    }
    out
}

/// The snapshot's sources (the typed model plus `policyAsset`/`observedAssets` from the raw
/// reply, which the shared model doesn't keep).
pub fn sources<'a>(s: &'a Snapshot, raw: Option<&'a serde_json::Value>) -> Vec<SourceBalance<'a>> {
    let raw_conns = raw
        .and_then(|r| r.get("connections"))
        .and_then(|c| c.as_array());
    s.connections
        .iter()
        .map(|c| {
            let extra = raw_conns.and_then(|l| {
                l.iter()
                    .find(|r| r.get("id").and_then(|v| v.as_str()) == Some(c.id.as_str()))
            });
            SourceBalance {
                kind: &c.kind,
                chain: &c.chain,
                network: c.network.as_deref().unwrap_or(""),
                label: &c.label,
                policy_asset: extra
                    .and_then(|e| e.get("policyAsset"))
                    .and_then(|v| v.as_str()),
                observed: extra
                    .and_then(|e| e.get("observedAssets"))
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default(),
                balance_btc: c.balance,
            }
        })
        .collect()
}

/// The snapshot's `otherAssets` (one per crypto asset other than Bitcoin, in whole units of
/// 10^-11, as every crypto quantity is stored) as `Other` rows: listed apart, never totalled
/// with Bitcoin, valued only when the daemon gave a market value.
pub fn other_rows(raw: Option<&serde_json::Value>) -> Vec<AssetRow> {
    let Some(list) = raw
        .and_then(|r| r.get("otherAssets"))
        .and_then(|v| v.as_array())
    else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|a| {
            let code = a.get("asset")?.as_str()?.trim();
            let units = i128::from(a.get("quantityUnits")?.as_i64()?);
            (!code.is_empty()).then(|| AssetRow {
                asset_id: code.to_owned(),
                name: code.to_owned(),
                symbol: code.to_owned(),
                precision: 11,
                kind: UnitKind::Other,
                layer: None,
                units,
                fiat_value: a
                    .get("marketValue")
                    .and_then(|v| v.as_f64())
                    .filter(|v| v.is_finite()),
                share: 0.0,
                synthetic: false,
            })
        })
        .collect()
}

/// The BTC market rate the card may value with: none while the snapshot says it is missing.
pub fn btc_rate(s: &Snapshot) -> Option<f64> {
    let r = s.rate();
    (!s.fiat.completeness.market_rate_missing && r.is_finite() && r > 0.0).then_some(r)
}

/// Rows grouped by denomination (Bitcoin, then fiat currencies A–Z, then Other), each group
/// largest first with its shares and total. Rows with no balance are dropped.
pub fn build(rows: Vec<AssetRow>) -> Vec<AssetGroup> {
    let mut groups: Vec<AssetGroup> = Vec::new();
    for r in rows.into_iter().filter(|r| r.units > 0) {
        let d = r.denomination();
        match groups.iter_mut().find(|g| g.denomination == d) {
            Some(g) => g.rows.push(r),
            None => groups.push(AssetGroup {
                denomination: d,
                rows: vec![r],
                total_units: None,
                total_value: None,
            }),
        }
    }
    let rank = |d: &Denomination| match d {
        Denomination::Btc => (0, String::new()),
        Denomination::Fiat(c) => (1, c.clone()),
        Denomination::Mixed => (2, String::new()),
    };
    groups.sort_by_key(|g| rank(&g.denomination));
    for g in &mut groups {
        g.rows
            .sort_by(|a, b| b.units.cmp(&a.units).then_with(|| a.symbol.cmp(&b.symbol)));
        let shared_unit = g.denomination != Denomination::Mixed;
        let total: i128 = g.rows.iter().map(|r| r.units).sum();
        for r in &mut g.rows {
            r.share = if shared_unit && total > 0 {
                (r.units as f64 / total as f64 * 100.0).round()
            } else {
                0.0
            };
        }
        g.total_units = shared_unit.then_some(total);
        g.total_value = shared_unit
            .then(|| g.rows.iter().map(|r| r.fiat_value).sum::<Option<f64>>())
            .flatten();
    }
    groups
}

/// A fiat cash row; valued at face only in the display currency.
pub fn fiat_cash(
    code: &str,
    name: &str,
    cents: i128,
    display_currency: &str,
    synthetic: bool,
) -> AssetRow {
    AssetRow {
        asset_id: code.to_uppercase(),
        name: name.into(),
        symbol: code.to_uppercase(),
        precision: 2,
        kind: UnitKind::Fiat,
        layer: None,
        units: cents,
        fiat_value: code
            .eq_ignore_ascii_case(display_currency)
            .then_some(cents as f64 / 100.0),
        share: 0.0,
        synthetic,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src<'a>(kind: &'a str, chain: &'a str, btc: f64) -> SourceBalance<'a> {
        SourceBalance {
            kind,
            chain,
            network: "",
            label: "",
            policy_asset: None,
            observed: vec![],
            balance_btc: btc,
        }
    }

    #[test]
    fn layers_follow_connection_asset_label() {
        assert_eq!(
            layer_of(
                "descriptor",
                &["liquid", "elementsregtest", "", "Treasury"],
                &[]
            ),
            Rail::Liquid
        );
        assert_eq!(layer_of("lnd", &["", "", "", "Node"], &[]), Rail::Lightning);
        assert_eq!(
            layer_of(
                "custom",
                &["", "", "", "Import"],
                &["LBTC".into(), "BTC".into()]
            ),
            Rail::Liquid
        );
        assert_eq!(
            layer_of(
                "address",
                &["bitcoin", "regtest", "", "Cold"],
                &["BTC".into()]
            ),
            Rail::Onchain
        );
        // A Lightning kind wins over the observed BTC rows it stores.
        assert_eq!(
            layer_of("phoenix", &["", "", "", ""], &["BTC".into()]),
            Rail::Lightning
        );
    }

    #[test]
    fn bitcoin_layers_fold_into_exact_msat_rows() {
        let sources = [
            src("address", "bitcoin", 34.01445239),
            src("address", "bitcoin", 0.0008),
            src("descriptor", "liquid", 0.0060009),
            src("lnd", "bitcoin", 0.00000001),
            src("address", "bitcoin", 0.0),
        ];
        let rows = bitcoin_rows(&sources, Some(50_000.0));
        let by = |s: &str| rows.iter().find(|r| r.symbol == s).unwrap();
        assert_eq!(by("BTC").units, 3_401_525_239_000);
        assert_eq!(by("L-BTC").units, 600_090_000);
        assert_eq!(by("LN-BTC").units, 1_000);
        assert_eq!(by("L-BTC").kind, UnitKind::Pegged);
    }

    #[test]
    fn other_crypto_assets_list_apart_with_their_own_unit() {
        let raw = serde_json::json!({"otherAssets": [
            {"asset": "ETH", "quantity": "3", "quantityUnits": 300_000_000_000_i64, "marketValue": 6000.0},
            {"asset": "SOL", "quantity": "0.5", "quantityUnits": 50_000_000_000_i64, "marketValue": null},
            {"asset": "", "quantityUnits": 1},
            {"asset": "BAD", "quantityUnits": "1"}
        ]});
        let rows = other_rows(Some(&raw));
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].units, rows[0].precision), (300_000_000_000, 11));
        assert_eq!(rows[0].fiat_value, Some(6000.0));
        assert_eq!(rows[1].fiat_value, None);
        let mut all = bitcoin_rows(&[src("address", "bitcoin", 1.0)], Some(60_000.0));
        all.extend(rows);
        let groups = build(all);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].total_units, Some(MSAT_PER_BTC));
        assert_eq!(groups[1].denomination, Denomination::Mixed);
        assert_eq!(groups[1].total_units, None);
        assert_eq!(groups[1].rows[0].symbol, "ETH");
        assert!(other_rows(Some(&serde_json::json!({}))).is_empty());
        assert!(other_rows(None).is_empty());
    }

    #[test]
    fn groups_never_mix_units() {
        let mut rows = bitcoin_rows(
            &[
                src("address", "bitcoin", 1.5),
                src("descriptor", "liquid", 0.5),
            ],
            Some(60_000.0),
        );
        rows.push(fiat_cash("EUR", "Euro cash", 125_000, "EUR", true));
        rows.push(fiat_cash("USD", "US dollar cash", 9_900, "EUR", true));
        let groups = build(rows);
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].denomination, Denomination::Btc);
        assert_eq!(groups[0].total_units, Some(2 * MSAT_PER_BTC));
        assert_eq!(groups[0].rows[0].share, 75.0);
        assert_eq!(groups[0].total_value, Some(120_000.0));
        assert_eq!(groups[1].denomination, Denomination::Fiat("EUR".into()));
        assert_eq!(groups[1].total_units, Some(125_000));
        assert_eq!(groups[1].total_value, Some(1_250.0));
        // Another currency is never converted: no price, and its group has no value.
        assert_eq!(groups[2].denomination, Denomination::Fiat("USD".into()));
        assert_eq!(groups[2].rows[0].fiat_value, None);
        assert_eq!(groups[2].total_value, None);
    }

    #[test]
    fn no_market_rate_means_no_price() {
        let groups = build(bitcoin_rows(&[src("address", "bitcoin", 1.0)], None));
        assert_eq!(groups[0].rows[0].fiat_value, None);
        assert_eq!(groups[0].total_value, None);
        assert_eq!(groups[0].total_units, Some(MSAT_PER_BTC));
    }

    #[test]
    fn mixed_assets_have_no_total_or_share() {
        let other = AssetRow {
            asset_id: "unknown".into(),
            name: "Unknown asset".into(),
            symbol: "?".into(),
            precision: 0,
            kind: UnitKind::Other,
            layer: None,
            units: 42,
            fiat_value: None,
            share: 0.0,
            synthetic: true,
        };
        let groups = build(vec![other]);
        assert_eq!(groups[0].denomination, Denomination::Mixed);
        assert_eq!(groups[0].total_units, None);
        assert_eq!(groups[0].rows[0].share, 0.0);
    }

    #[test]
    fn empty_balances_build_no_groups() {
        assert!(build(bitcoin_rows(&[src("address", "bitcoin", 0.0)], Some(1.0))).is_empty());
    }
}
