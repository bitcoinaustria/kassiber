//! The asset question the Add connection dialog asks before a new provider wallet's one import
//! (docs/reference/imports.md#choosing-the-assets-before-the-first-import): read from an import
//! or exchange preview, **Bitcoin and other crypto** (preselected) or **Bitcoin only**, naming
//! the tickers and entry count. The daemon reads the source and hands out a token bound to what
//! it read; the create and import refuse a source that changed since. This module only draws
//! the answer.

use rune::{El, col, el, row, text};
use serde_json::Value;

use crate::app::App;
use crate::i18n::{t, tc, tr};
use crate::theme::Tk;
use crate::wallets::add::btcpay::form::xs;
use crate::wallets::assets::model::{TickerSummary, ticker_summary};

/// Crypto assets other than Bitcoin in a preview, by ticker.
fn other_assets(preview: &Value) -> TickerSummary {
    let map = preview["other_assets"]
        .as_object()
        .map(|m| {
            m.iter()
                .filter_map(|(k, n)| n.as_u64().filter(|n| *n > 0).map(|n| (k.clone(), n)))
                .collect()
        })
        .unwrap_or_default();
    ticker_summary(&map)
}

/// The asset question for a new provider wallet, from an import or exchange preview.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    /// Crypto assets other than Bitcoin, by ticker.
    pub other: TickerSummary,
    /// Bitcoin only cannot import the source.
    pub bitcoin_refused: bool,
}

impl Choice {
    /// `None` when the preview asks nothing (no other crypto asset, or a source that reads only
    /// Bitcoin).
    pub fn of(preview: &Value) -> Option<Choice> {
        if preview["choice"].as_bool() != Some(true) {
            return None;
        }
        let other = other_assets(preview);
        (other.rows > 0).then(|| Choice {
            other,
            bitcoin_refused: preview["bitcoin_only"].as_bool() == Some(false),
        })
    }

    /// The scope a new wallet gets for the answer (`crypto` unless the user picked Bitcoin
    /// only, which a source Bitcoin only cannot import never keeps).
    pub fn scope(&self, bitcoin_only: bool) -> &'static str {
        if bitcoin_only && !self.bitcoin_refused {
            "bitcoin"
        } else {
            "crypto"
        }
    }
}

/// Other crypto assets the `crypto` scope cannot read from the source, and the daemon's reason:
/// there is nothing to choose, but the user is told why they stay out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CannotKeep {
    pub other: TickerSummary,
    pub reason: String,
}

impl CannotKeep {
    pub fn of(preview: &Value) -> Option<CannotKeep> {
        let reason = preview["crypto_error"].as_str()?.to_owned();
        let other = other_assets(preview);
        (other.rows > 0).then_some(CannotKeep { other, reason })
    }
}

/// File formats whose provider wallet may hold other crypto assets (Bitfinex and Bitcoin.de
/// read only Bitcoin; the exchange-evidence formats import into the book's provider wallet).
pub const FILE_FORMATS: [&str; 5] = [
    "ledgerlive_csv",
    "bitvavo_csv",
    "bitstamp_csv",
    "cointracking_csv",
    "blockpit_csv",
];

/// Whether a new exchange API connection is asked (every exchange but those read only Bitcoin).
pub fn exchange_asks(exchange: &str) -> bool {
    !matches!(exchange, "bitfinex" | "bitcoinde")
}

/// What a preview was run for: the file the dialog chose, or the pasted key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    File { format: String, path: String },
    Exchange(crate::wallets::add::exchange::model::Credentials),
}

impl Source {
    /// An exchange account's history rather than an export file.
    pub fn is_history(&self) -> bool {
        matches!(self, Source::Exchange(_))
    }
}

/// A preview's answer for the dialog's current source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ask {
    pub source: Source,
    /// `None`: nothing to ask, the wallet keeps the default scope.
    pub choice: Option<Choice>,
    /// The user picked Bitcoin only (the default is Bitcoin and other crypto).
    pub bitcoin_only: bool,
    /// The preview's `preview_token`: the create and import refuse a changed source with it.
    pub token: Option<String>,
    /// Other assets the source holds that cannot be kept, and why.
    pub cannot_keep: Option<CannotKeep>,
}

impl Ask {
    /// The answer for `source` from a preview's data.
    pub fn of(source: Source, preview: &Value) -> Ask {
        Ask {
            source,
            choice: Choice::of(preview),
            bitcoin_only: false,
            token: preview["preview_token"].as_str().map(str::to_owned),
            cannot_keep: CannotKeep::of(preview),
        }
    }

    /// The `asset_scope` the new wallet is created with, when the user was asked.
    pub fn scope(&self) -> Option<&'static str> {
        self.choice.as_ref().map(|c| c.scope(self.bitcoin_only))
    }
}

/// One answer of the question (the custody link question's radio pattern).
fn option(
    tk: &Tk,
    key: &'static str,
    title: String,
    hint: String,
    checked: bool,
    enabled: bool,
    pick: impl Fn(&mut App) + 'static,
) -> El {
    let dot = row()
        .size(tk.r(0.875), tk.r(0.875))
        .mt(tk.r(0.25))
        .center()
        .radius(tk.r(0.4375))
        .border(1.0, if checked { tk.primary } else { tk.input })
        .child(if checked {
            crate::ui::dot(tk.r(0.4375), tk.primary)
        } else {
            el()
        });
    let mut opt = row()
        .key(key)
        .w_full()
        .min_w(0.0)
        .gap(tk.r(0.5))
        .items_start()
        .px(tk.r(0.75))
        .py(tk.r(0.5))
        .radius(tk.r(0.5))
        .border(1.0, if checked { tk.primary } else { tk.border })
        .role("radio")
        .checked(checked)
        .label(format!("{title}. {hint}"))
        .focusable(enabled)
        .disabled(!enabled)
        .child(dot.a11y_hidden())
        .child(
            col()
                .min_w(0.0)
                .grow(1.0)
                .gap(tk.r(0.125))
                .child(text(title).font(tk.tsm(500.0)).color(tk.fg).wrap())
                .child(text(hint).font(tk.txs(400.0)).color(tk.muted_fg).wrap()),
        );
    if checked {
        opt = opt.fill(tk.primary.alpha(0.05));
    }
    if enabled {
        opt.hover(tk.muted.alpha(0.4)).on_press(pick)
    } else {
        opt.opacity(0.6)
    }
}

/// The key of the answer that takes focus when the question appears.
pub const FIRST_ANSWER: &str = "asset-choice-crypto";

/// The question: **Bitcoin and other crypto** (the default) or **Bitcoin only**, naming the
/// tickers and entry count of the export, or of the account's history.
pub fn choice_view(
    tk: &Tk,
    c: &Choice,
    history: bool,
    bitcoin_only: bool,
    pick: impl Fn(&mut App, bool) + Clone + 'static,
) -> El {
    let tickers = crate::wallets::assets::ticker_text(&c.other.tickers);
    let found = tc(
        if history {
            "connections:reimport.choice.foundHistory"
        } else {
            "connections:reimport.choice.found"
        },
        c.other.rows as usize,
        &[
            ("tickers", &tickers),
            ("rows", &crate::format::count(c.other.rows as usize)),
        ],
    );
    let bitcoin_on = bitcoin_only && !c.bitcoin_refused;
    let (crypto_pick, bitcoin_pick) = (pick.clone(), pick);
    let bitcoin_hint = match (c.bitcoin_refused, history) {
        (true, true) => t("connections:reimport.choice.bitcoinRefusedHistory"),
        (true, false) => t("connections:reimport.choice.bitcoinRefused"),
        _ => t("connections:reimport.choice.bitcoinBody"),
    };
    col()
        .key("asset-choice")
        .w_full()
        .padding(tk.r(0.75))
        .gap(tk.r(0.5))
        .radius(tk.r(0.375))
        .border(1.0, tk.border.alpha(tk.border.a * 0.6))
        .border_layout()
        .child(
            text(t("connections:reimport.choice.title"))
                .font(tk.tsm(500.0))
                .color(tk.fg)
                .role("heading"),
        )
        .child(xs(tk, found.clone(), tk.fg))
        .child(
            col()
                .w_full()
                .gap(tk.r(0.5))
                .role("radiogroup")
                .label(format!(
                    "{} {found}",
                    t("connections:reimport.choice.title")
                ))
                .focus_group()
                .child(option(
                    tk,
                    FIRST_ANSWER,
                    t("connections:reimport.choice.crypto"),
                    t("connections:reimport.choice.cryptoBody"),
                    !bitcoin_on,
                    true,
                    move |a| crypto_pick(a, false),
                ))
                .child(option(
                    tk,
                    "asset-choice-bitcoin",
                    t("connections:reimport.choice.bitcoin"),
                    bitcoin_hint,
                    bitcoin_on,
                    !c.bitcoin_refused,
                    move |a| bitcoin_pick(a, true),
                )),
        )
        .child(xs(tk, t("connections:reimport.choice.stocks"), tk.muted_fg))
}

/// Other assets a source holds that only Bitcoin is imported without, and the daemon's reason.
pub fn cannot_keep_view(tk: &Tk, c: &CannotKeep) -> El {
    let tickers = crate::wallets::assets::ticker_text(&c.other.tickers);
    col()
        .key("asset-choice-cannot-keep")
        .w_full()
        .padding(tk.r(0.75))
        .gap(tk.r(0.25))
        .radius(tk.r(0.375))
        .border(1.0, tk.border.alpha(tk.border.a * 0.6))
        .border_layout()
        .role("status")
        .child(xs(
            tk,
            tc(
                "connections:reimport.choice.found",
                c.other.rows as usize,
                &[
                    ("tickers", &tickers),
                    ("rows", &crate::format::count(c.other.rows as usize)),
                ],
            ),
            tk.fg,
        ))
        .child(xs(
            tk,
            tr(
                "connections:reimport.choice.cryptoRefused",
                &[("reason", &c.reason)],
            ),
            tk.muted_fg,
        ))
}

/// "Checking which assets …" while the preview runs.
pub fn checking_view(tk: &Tk, history: bool) -> El {
    xs(
        tk,
        t(if history {
            "connections:reimport.choice.checkingHistory"
        } else {
            "connections:reimport.choice.checking"
        }),
        tk.muted_fg,
    )
    .role("status")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_choice_needs_other_crypto_and_defaults_to_keeping_it() {
        let p =
            json!({"choice": true, "bitcoin_only": true, "other_assets": {"ETH": 30, "SOL": 9}});
        let c = Choice::of(&p).unwrap();
        assert_eq!(c.other.tickers, ["ETH", "SOL"]);
        assert_eq!(c.other.rows, 39);
        assert_eq!((c.scope(false), c.scope(true)), ("crypto", "bitcoin"));
        // A source Bitcoin only cannot import keeps the other assets whatever was picked.
        let refused =
            Choice::of(&json!({"choice": true, "bitcoin_only": false, "other_assets": {"SOL": 1}}))
                .unwrap();
        assert_eq!(refused.scope(true), "crypto");
        // Nothing to ask: a Bitcoin-only history, or a source that reads only Bitcoin.
        assert_eq!(
            Choice::of(&json!({"choice": false, "other_assets": {"ETH": 1}})),
            None
        );
        assert_eq!(
            Choice::of(&json!({"choice": true, "other_assets": {}})),
            None
        );
    }

    #[test]
    fn an_unkeepable_asset_is_explained_and_the_token_is_kept() {
        let source = Source::File {
            format: "cointracking_csv".into(),
            path: "/h.csv".into(),
        };
        let p = json!({"choice": false, "other_assets": {"SOL": 2}, "crypto_error": "csv row 1 has a fiat fee", "preview_token": "abc"});
        let ask = Ask::of(source, &p);
        assert_eq!(ask.choice, None);
        assert_eq!(ask.scope(), None);
        assert_eq!(ask.token.as_deref(), Some("abc"));
        let c = ask.cannot_keep.unwrap();
        assert_eq!(
            (c.other.rows, c.reason.as_str()),
            (2, "csv row 1 has a fiat fee")
        );
        assert_eq!(
            tr(
                "connections:reimport.choice.cryptoRefused",
                &[("reason", &c.reason)]
            ),
            "These assets cannot be tracked from this export, so only Bitcoin is imported: csv row 1 has a fiat fee"
        );
        assert!(
            !Source::File {
                format: String::new(),
                path: String::new()
            }
            .is_history()
        );
        assert!(
            Source::Exchange(crate::wallets::add::exchange::model::Credentials::default())
                .is_history()
        );
    }
}
