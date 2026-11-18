//! components/kb/assets/*: `AssetScopeNotice` (+ importAssets) and `ImportAssetsSummary`, the
//! asset scope part of an exchange card and of a provider import's result
//! (docs/plan/24-crypto-assets.md).
//!
//! The notice names the other crypto assets a `bitcoin` wallet skipped, with **Also track these
//! assets**; says that a `crypto` wallet tracks them, with **Track Bitcoin only**; names the
//! non-crypto holdings no scope imports and the coins of an exchange Kassiber reads only Bitcoin
//! from; and the swaps the import paired. Changing the scope is a wallet change
//! (`ui.wallets.update` with `asset_scope`), so it asks for the passphrase or, in a plaintext
//! book, the typed acknowledgement, like any other wallet edit. Nothing leaves the machine:
//! after storing `crypto` the exchange card runs its own Sync now, and the import result
//! imports the same file again only when this dialog kept the path the user chose.
//!
//! Two surfaces keep their own state: the wallet detail's exchange card and the Add connection
//! dialog's import result. Tags `"wallets.assets.…"`.

pub mod model;

use rune::{El, TextEdit, col, row, text};
use serde_json::Value;

use crate::app::{App, Reply};
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::ic;
use crate::wallets::add::btcpay::form::{self, Variant, button, error_line, input, xs};
use model::{ImportAssets, PLAINTEXT_CHANGE_ACK, Rerun, Scope, Shown};

/// Where a notice is mounted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    /// `ExchangeSyncCard` on the wallet detail.
    Card,
    /// The Add connection dialog's import result.
    Import,
}

/// One notice's `useState`s.
#[derive(Default)]
pub struct Notice {
    /// The wallet the state below belongs to.
    pub wallet: String,
    /// The scope the open confirmation would store.
    pub confirming: Option<Scope>,
    pub secret: TextEdit,
    pub error: Option<String>,
    /// `crypto` is stored and the file has to be chosen again.
    pub pick_again: bool,
    /// The `ui.wallets.update` in flight.
    pub pending: bool,
    /// The import after it (the import result's same file again) in flight.
    pub reimport_pending: bool,
}

#[derive(Default)]
pub struct State {
    pub card: Notice,
    pub import: Notice,
}

fn notice(app: &App, s: Surface) -> &Notice {
    match s {
        Surface::Card => &app.wallets.assets.card,
        Surface::Import => &app.wallets.assets.import,
    }
}

fn notice_mut(app: &mut App, s: Surface) -> &mut Notice {
    match s {
        Surface::Card => &mut app.wallets.assets.card,
        Surface::Import => &mut app.wallets.assets.import,
    }
}

/// The surface's state for `wallet` (a fresh one once the wallet changes).
fn for_wallet<'a>(app: &'a App, s: Surface, wallet: &str) -> Option<&'a Notice> {
    let n = notice(app, s);
    (n.wallet == wallet).then_some(n)
}

/// Starts over (a new import result, another wallet).
pub fn reset(app: &mut App, s: Surface) {
    *notice_mut(app, s) = Notice::default();
}

fn begin(app: &mut App, s: Surface, wallet: &str, next: Scope) {
    let n = notice_mut(app, s);
    if n.wallet != wallet {
        *n = Notice {
            wallet: wallet.to_owned(),
            ..Notice::default()
        };
    }
    n.confirming = Some(next);
    n.secret = TextEdit::default();
    n.error = None;
}

/// Scene harness: the confirmation to track other assets, open for `wallet`.
pub fn scene_confirm(app: &mut App, s: Surface, wallet: &str) {
    begin(app, s, wallet, Scope::Crypto);
}

fn cancel(app: &mut App, s: Surface) {
    let n = notice_mut(app, s);
    n.confirming = None;
    n.error = None;
}

fn confirm(app: &mut App, s: Surface) {
    let encrypted = app.encrypted;
    let n = notice(app, s);
    let Some(next) = n.confirming else { return };
    if n.pending {
        return;
    }
    if let Some(issue) = model::auth_issue(encrypted, &n.secret.text) {
        let msg = tr(
            &format!("connections:assets.confirm.{issue}"),
            &[("ack", PLAINTEXT_CHANGE_ACK)],
        );
        notice_mut(app, s).error = Some(msg);
        return;
    }
    let args = model::update_args(&n.wallet, next, encrypted, &n.secret.text);
    let tag = match s {
        Surface::Card => "wallets.assets.card.update",
        Surface::Import => "wallets.assets.import.update",
    };
    if app.send("ui.wallets.update", args, tag).is_some() {
        let n = notice_mut(app, s);
        n.error = None;
        n.pending = true;
    }
}

/// The daemon's message for a failed request (`local_auth_denied` in the UI language).
fn failure(r: &Reply, fallback: &str) -> String {
    if r.error_detail["code"].as_str() == Some("local_auth_denied") {
        return t("connections:assets.confirm.errorDenied");
    }
    r.error
        .clone()
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| fallback.to_owned())
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    let (s, step) = match tag {
        "wallets.assets.card.update" => (Surface::Card, "update"),
        "wallets.assets.import.update" => (Surface::Import, "update"),
        "wallets.assets.import.reimport" => (Surface::Import, "reimport"),
        _ => return,
    };
    if step == "reimport" {
        notice_mut(app, s).reimport_pending = false;
        match &r.error {
            // The scope is stored; only the import after it failed.
            Some(_) => notice_mut(app, s).error = Some(failure(r, "")),
            None => app.wallets.add.last_import = Some(r.data.clone()),
        }
        app.refresh_reads();
        return;
    }
    let next = {
        let n = notice_mut(app, s);
        n.pending = false;
        n.confirming
    };
    if r.error.is_some() {
        notice_mut(app, s).error = Some(failure(r, ""));
        return;
    }
    let Some(next) = next else { return };
    {
        let n = notice_mut(app, s);
        n.confirming = None;
        n.secret = TextEdit::default();
    }
    app.refresh_reads();
    match s {
        Surface::Card => {
            crate::wallets::add::exchange::refresh_status(app);
            // Turning other assets on syncs again so the skipped rows arrive; turning them off
            // only affects later syncs.
            if next == Scope::Crypto {
                crate::wallets::add::exchange::run_sync(app);
            }
        }
        Surface::Import => {
            let replay = model::Replay::usable(app.wallets.add.last_import_replay.as_ref())
                .cloned()
                .filter(|_| next == Scope::Crypto);
            match replay {
                Some(replay) => {
                    if app
                        .send(
                            "ui.wallets.import_file",
                            replay.args(),
                            "wallets.assets.import.reimport",
                        )
                        .is_some()
                    {
                        notice_mut(app, s).reimport_pending = true;
                    }
                }
                None => {
                    notice_mut(app, s).pick_again = next == Scope::Crypto;
                    // The stored scope, without a second import.
                    if let Some(assets) = app
                        .wallets
                        .add
                        .last_import
                        .as_mut()
                        .and_then(|r| r.get_mut("assets"))
                        .filter(|a| a.is_object())
                    {
                        assets["scope"] = Value::from(next.as_str());
                    }
                }
            }
        }
    }
}

/// "ETH, SOL and 2 more".
pub fn ticker_text(tickers: &[String]) -> String {
    let (shown, more) = model::shown_tickers(tickers);
    let list = shown.join(", ");
    if more > 0 {
        tc("connections:assets.tickersMore", more, &[("list", &list)])
    } else {
        list
    }
}

fn rows_line(key: &str, s: &model::TickerSummary) -> String {
    tc(
        key,
        s.rows as usize,
        &[
            ("tickers", &ticker_text(&s.tickers)),
            ("rows", &crate::format::count(s.rows as usize)),
        ],
    )
}

/// What the notice is drawn from.
pub struct Input<'a> {
    pub surface: Surface,
    /// The wallet id (or label) `ui.wallets.update` names.
    pub wallet: &'a str,
    /// The wallet's current scope.
    pub scope: Scope,
    /// What the last import or sync left out, when it said.
    pub assets: Option<&'a ImportAssets>,
    pub swap_pairs: Option<u64>,
    pub rerun: Rerun,
    pub disabled: bool,
}

/// `AssetScopeNotice`; `None` when there is nothing to say.
pub fn view(app: &App, tk: &Tk, i: Input) -> Option<El> {
    let st = for_wallet(app, i.surface, i.wallet);
    let shown = Shown::of(i.scope, i.assets, i.swap_pairs);
    let error = st.and_then(|n| n.error.clone());
    let pick_again = st.is_some_and(|n| n.pick_again);
    if shown.is_empty(i.scope) && !pick_again && error.is_none() {
        return None;
    }
    let confirming = st.and_then(|n| n.confirming);
    let busy = i.disabled || st.is_some_and(|n| n.pending || n.reimport_pending);
    let surface = i.surface;
    let wallet = i.wallet.to_owned();
    let muted = |s: String| xs(tk, s, tk.muted_fg);
    let headline = |s: String| {
        row()
            .gap(tk.r(0.375))
            .center_y()
            .child(ic(&I::COINS, tk.r(0.875), tk.fg))
            .child(text(s).font(tk.txs(500.0)).color(tk.fg).wrap().min_w(0.0))
    };

    let mut c = col()
        .key(match surface {
            Surface::Card => "asset-scope-card",
            Surface::Import => "asset-scope-import",
        })
        .w_full()
        .min_w(0.0)
        .padding(tk.r(0.75))
        .gap(tk.r(0.5))
        .radius(tk.r(0.375))
        .border(1.0, tk.border.alpha(tk.border.a * 0.6))
        .border_layout()
        .role("group")
        .label(t("connections:assets.track"));
    if shown.offer_tracking {
        c = c.child(
            col()
                .w_full()
                .gap(tk.r(0.25))
                .child(headline(rows_line(
                    "connections:assets.found",
                    &shown.skipped,
                )))
                .child(muted(t("connections:assets.foundBody"))),
        );
    }
    if i.scope == Scope::Crypto {
        c = c.child(headline(t("connections:assets.tracked")));
        if pick_again {
            c = c.child(muted(t("connections:assets.pickAgain")).role("status"));
        }
    }
    if shown.excluded.rows > 0 {
        c = c.child(muted(rows_line(
            "connections:assets.excluded",
            &shown.excluded,
        )));
    }
    if shown.unread.rows > 0 {
        c = c.child(muted(rows_line("connections:assets.unread", &shown.unread)));
    }
    if shown.pairs > 0 {
        c = c.child(muted(tc(
            "connections:assets.swapPairs",
            shown.pairs as usize,
            &[("pairs", &crate::format::count(shown.pairs as usize))],
        )));
    }

    if let Some(next) = confirming {
        let encrypted = app.encrypted;
        let secret = st.map(|n| &n.secret);
        let empty = TextEdit::default();
        let field_label = if encrypted {
            t("connections:assets.confirm.passphrase")
        } else {
            tr(
                "connections:assets.confirm.ack",
                &[("ack", PLAINTEXT_CHANGE_ACK)],
            )
        };
        let (title, body, submit) = match next {
            Scope::Crypto => (
                t("connections:assets.confirm.trackTitle"),
                t(&format!(
                    "connections:assets.confirm.trackBody.{}",
                    i.rerun.key()
                )),
                t(&format!(
                    "connections:assets.confirm.trackButton.{}",
                    i.rerun.key()
                )),
            ),
            Scope::Bitcoin => (
                t("connections:assets.confirm.untrackTitle"),
                t("connections:assets.confirm.untrackBody"),
                t("connections:assets.confirm.untrackButton"),
            ),
        };
        let mut submit_btn = button(
            tk,
            &format!("asset-scope-confirm-{}", surface_key(surface)),
            Variant::Secondary,
            true,
            None,
            &submit,
            !busy,
        );
        if !busy {
            submit_btn = submit_btn.on_press(move |a: &mut App| confirm(a, surface));
        }
        let mut form_box = col()
            .w_full()
            .padding(tk.r(0.625))
            .gap(tk.r(0.5))
            .radius(tk.r(0.375))
            .fill(tk.muted.alpha(0.4))
            .role("group")
            .label(title.clone())
            .child(text(title).font(tk.txs(500.0)).color(tk.fg).wrap())
            .child(muted(body))
            .child(form::setup_field(
                tk,
                &field_label,
                input(
                    tk,
                    &format!("asset-scope-secret-{}", surface_key(surface)),
                    secret.unwrap_or(&empty),
                    move |a: &mut App| &mut notice_mut(a, surface).secret,
                    &field_label,
                    (!encrypted).then_some(PLAINTEXT_CHANGE_ACK),
                    encrypted,
                    |_| {},
                ),
                None,
                None,
            ));
        if let Some(e) = &error {
            form_box = form_box.child(error_line(tk, e.clone(), true));
        }
        form_box = form_box.child(
            row().gap(tk.r(0.5)).child(submit_btn).child(
                button(
                    tk,
                    &format!("asset-scope-cancel-{}", surface_key(surface)),
                    Variant::Ghost,
                    true,
                    None,
                    &t("connections:assets.confirm.cancel"),
                    true,
                )
                .on_press(move |a: &mut App| cancel(a, surface)),
            ),
        );
        return Some(c.child(form_box));
    }
    if let Some(e) = error {
        c = c.child(error_line(tk, e, true));
    }
    let action = if shown.offer_tracking {
        Some((
            Scope::Crypto,
            Variant::Outline,
            t("connections:assets.track"),
        ))
    } else if i.scope == Scope::Crypto {
        Some((
            Scope::Bitcoin,
            Variant::Ghost,
            t("connections:assets.untrack"),
        ))
    } else {
        None
    };
    if let Some((next, variant, label)) = action {
        let mut b = button(
            tk,
            &format!("asset-scope-{}-{}", next.as_str(), surface_key(surface)),
            variant,
            true,
            None,
            &label,
            !busy,
        );
        if !busy {
            b = b.on_press(move |a: &mut App| begin(a, surface, &wallet, next));
        }
        c = c.child(row().child(b));
    }
    Some(c)
}

fn surface_key(s: Surface) -> &'static str {
    match s {
        Surface::Card => "card",
        Surface::Import => "import",
    }
}

/// `ImportAssetsSummary`: the notice under an import result, for the wallet it imported into.
pub fn import_view(app: &App, tk: &Tk, result: &Value) -> Option<El> {
    let parsed = model::read(result.get("assets"));
    let pairs = result.get("swap_pairs").and_then(Value::as_u64);
    let replay = app.wallets.add.last_import_replay.as_ref();
    let wallet = replay
        .map(|r| r.wallet.clone())
        .or_else(|| {
            result
                .get("wallet")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .filter(|w| !w.is_empty())?;
    if parsed.is_none() && pairs.unwrap_or(0) == 0 {
        return None;
    }
    view(
        app,
        tk,
        Input {
            surface: Surface::Import,
            wallet: &wallet,
            scope: parsed.as_ref().map(|a| a.scope).unwrap_or_default(),
            assets: parsed.as_ref(),
            swap_pairs: pairs,
            rerun: if model::Replay::usable(replay).is_some() {
                Rerun::Import
            } else {
                Rerun::PickFile
            },
            disabled: false,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn reply(data: Value, error: Option<&str>, code: Option<&str>) -> Reply {
        Reply {
            kind: "ui.wallets.update".into(),
            data,
            error: error.map(str::to_owned),
            error_detail: code.map_or(Value::Null, |c| json!({"code": c})),
            elapsed: std::time::Duration::ZERO,
        }
    }

    #[test]
    fn a_denied_passphrase_reads_in_the_ui_language() {
        let r = reply(Value::Null, Some("denied"), Some("local_auth_denied"));
        assert_eq!(failure(&r, ""), t("connections:assets.confirm.errorDenied"));
        let r = reply(Value::Null, Some("wallet not found"), Some("not_found"));
        assert_eq!(failure(&r, ""), "wallet not found");
    }

    #[test]
    fn tickers_name_five_then_count_the_rest() {
        let many: Vec<String> = ["A", "B", "C", "D", "E", "F", "G"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        assert_eq!(ticker_text(&many[..2]), "A, B");
        let text = ticker_text(&many);
        assert!(
            text.starts_with("A, B, C, D, E") && text.contains('2'),
            "{text}"
        );
    }
}
