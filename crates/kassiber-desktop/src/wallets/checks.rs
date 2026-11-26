//! Balance checks (docs/reference/balance-checkpoints.md): the Balance checks card on a
//! connection's page (BalanceChecksCard.tsx), and the book-wide read behind the wallets list
//! mark, the `balance.differs` Needs attention row and the Reports notice
//! (useBalanceCheckMismatches.ts).
//!
//! Every read here is local: `ui.wallets.checkpoints.list` compares stored data and never
//! contacts a source. A difference is a review item ("the source may be incomplete"), never a
//! correction; adding or removing a balance changes no journal or tax result.

use rune::{El, Key, KeyEvent, TextEdit, col, row, text};
use serde_json::{Value, json};

use super::detail::Read;
use super::detail::ui::{self as dui, amber, emerald_text};
use crate::app::{App, Privacy, Reply, Ticket};
use crate::i18n::{has, t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::button::{Size, Variant, button};
use crate::ui::ic;
use crate::wallets::conn::Conn;

/// This feature's state (`app.wallets.checks`); a book switch starts it over.
#[derive(Default)]
pub struct State {
    /// The open book's latest checkpoints (`ui.wallets.checkpoints.list` without a wallet),
    /// once read.
    pub book: Option<Value>,
    pub card: Card,
    /// Add balance requests in flight. The dialog keeps its own request's ticket, so only that
    /// dialog takes the answer; these copies outlive a dialog closed or a card started over
    /// meanwhile, whose saved balance still has the screens read again.
    pub adds: Vec<Ticket>,
    /// Remove requests in flight, kept like `adds`: the card that sent one keeps its ticket.
    pub removes: Vec<Ticket>,
}

/// The card on one connection's page.
#[derive(Default)]
pub struct Card {
    /// The wallet the read below belongs to.
    pub wallet: String,
    /// View history: every checkpoint, each stored one with Remove.
    pub history: bool,
    pub read: Read<Value>,
    /// The book refresh cycle the read follows (`App::book_generation`).
    pub generation: u64,
    /// The asset a manual balance of this connection is in (`checkpoint_asset`).
    pub asset: &'static str,
    pub add: Option<AddForm>,
    pub removing: bool,
    pub remove_error: Option<String>,
    /// The Remove request this card sent (a card started over for another wallet has none).
    pub remove_ticket: Option<Ticket>,
}

/// The Add balance dialog.
#[derive(Default)]
pub struct AddForm {
    pub btc: TextEdit,
    pub date: TextEdit,
    pub note: TextEdit,
    pub error: Option<String>,
    pub pending: bool,
    /// The request this dialog sent.
    pub ticket: Option<Ticket>,
}

// ---- Model -----------------------------------------------------------------------------------

/// One wallet whose latest reported balance differs from the book.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WalletDiff {
    pub wallet_id: String,
    pub wallet: String,
    /// Each of its checkpoints only waits for the next sync (`after_last_sync`).
    pub pending: bool,
}

fn str_of<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

/// The checkpoints a report lists.
fn checkpoints(report: &Value) -> impl Iterator<Item = &Value> {
    report
        .get("checkpoints")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

/// `useBalanceCheckMismatches`: the checkpoints that differ, and the deferred chain comparisons
/// that wait for the next sync (shown too), grouped by wallet in the order the report lists
/// them.
pub fn wallet_diffs(report: &Value) -> Vec<WalletDiff> {
    let mut out: Vec<WalletDiff> = Vec::new();
    for c in checkpoints(report) {
        let waits = str_of(c, "reason") == "after_last_sync";
        if str_of(c, "status") != "differs" && !waits {
            continue;
        }
        let id = str_of(c, "wallet_id");
        match out.iter_mut().find(|d| d.wallet_id == id) {
            Some(d) => d.pending &= waits,
            None => out.push(WalletDiff {
                wallet_id: id.to_owned(),
                wallet: str_of(c, "wallet").to_owned(),
                pending: waits,
            }),
        }
    }
    out
}

/// The open book's differing wallets (empty until the book-wide read answered).
pub fn book_diffs(app: &App) -> Vec<WalletDiff> {
    app.wallets
        .checks
        .book
        .as_ref()
        .map(wallet_diffs)
        .unwrap_or_default()
}

/// Whether the wallets list marks connection `id`.
pub fn differs(app: &App, id: &str) -> bool {
    book_diffs(app).iter().any(|d| d.wallet_id == id)
}

/// The `balance.differs` row (notificationProjection.ts): (title, body, details). It names the
/// wallet, never an amount; it says "waits for the next sync" only when every wallet in it
/// waits. Wallet names follow the privacy mode as the other rows' labels do.
/// A wallet name as the privacy mode shows it: hidden under Hide all, and under Hide amounts
/// when it carries digits (a name can hold a balance).
fn masked_name(name: &str, privacy: Privacy) -> String {
    match privacy {
        Privacy::HideAll => crate::theme::MASK.to_owned(),
        Privacy::HideAmounts if name.chars().any(|c| c.is_ascii_digit()) => {
            crate::theme::MASK.to_owned()
        }
        _ => name.to_owned(),
    }
}

/// `diffs` with each wallet name as the privacy mode shows it.
pub fn shown(diffs: &[WalletDiff], privacy: Privacy) -> Vec<WalletDiff> {
    diffs
        .iter()
        .map(|d| WalletDiff {
            wallet: masked_name(&d.wallet, privacy),
            ..d.clone()
        })
        .collect()
}

/// The open book's differing wallets for the Reports notice, named as the privacy mode allows.
pub fn readiness_diffs(app: &App) -> Vec<WalletDiff> {
    shown(&book_diffs(app), app.prefs.privacy)
}

pub fn condition_text(diffs: &[WalletDiff], privacy: Privacy) -> (String, String, Vec<String>) {
    let shown = |l: &str| masked_name(l, privacy);
    let pending = diffs.iter().all(|d| d.pending);
    let base = if pending {
        "chrome:notifications.condition.balanceWaits"
    } else {
        "chrome:notifications.condition.balanceDiffers"
    };
    let name = (diffs.len() == 1 && privacy != Privacy::HideAll).then(|| shown(&diffs[0].wallet));
    let title = match &name {
        Some(n) => tr(&format!("{base}.titleNamed"), &[("name", n)]),
        None => tc(&format!("{base}.title"), diffs.len(), &[]),
    };
    let details = if privacy == Privacy::HideAll || diffs.len() < 2 {
        Vec::new()
    } else {
        diffs.iter().map(|d| shown(&d.wallet)).collect()
    };
    (title, t(&format!("{base}.body")), details)
}

/// Where the row's Open connection goes: the wallet, or the list when several differ.
pub fn condition_route(diffs: &[WalletDiff]) -> String {
    match diffs {
        [one] => format!("/connections/{}", one.wallet_id),
        _ => "/connections".to_owned(),
    }
}

/// The Reports notice (`buildReportReadiness`): (title, detail), naming each wallet once. Pass
/// the names through `shown` first (`readiness_diffs`).
pub fn readiness_text(diffs: &[WalletDiff]) -> Option<(String, String)> {
    if diffs.is_empty() {
        return None;
    }
    let mut names: Vec<&str> = Vec::new();
    for d in diffs {
        if !names.contains(&d.wallet.as_str()) {
            names.push(&d.wallet);
        }
    }
    let wallets = names.join(", ");
    let key = if diffs.iter().all(|d| d.pending) {
        "reports:readiness.balanceWaits"
    } else {
        "reports:readiness.balanceDiffers"
    };
    Some((
        t(&format!("{key}.title")),
        tr(&format!("{key}.detail"), &[("wallets", &wallets)]),
    ))
}

/// A reported, booked or difference amount: `0.5 BTC`, signed for a difference.
pub fn amount(value: Option<&str>, asset: &str, signed: bool, hidden: bool) -> String {
    match value.filter(|v| !v.is_empty()) {
        None => "—".to_owned(),
        Some(_) if hidden => crate::ui::hidden_amount(),
        Some(v) if signed && !v.starts_with('-') => format!("+{v} {asset}"),
        Some(v) => format!("{v} {asset}"),
    }
}

/// The checkpoints the card shows: only this wallet's (an answer for another wallet the page
/// showed before is not this one's).
fn card_rows(card: &Card) -> Vec<&Value> {
    card.read
        .ready()
        .map(|r| {
            checkpoints(r)
                .filter(|c| str_of(c, "wallet_id") == card.wallet)
                .collect()
        })
        .unwrap_or_default()
}

/// The asset of a manual balance on connection `c`: LBTC on Liquid, BTC everywhere else (a
/// Lightning node's balance is BTC). Decided by the connection's chain, or its asset when it
/// names no chain, never by the user's label (`Conn::asset_label` also reads the label, which
/// is fine for display but not for a stored asset).
pub fn checkpoint_asset(c: &Conn) -> &'static str {
    let liquid = |v: &str| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "liquid" | "liquidv1" | "lbtc" | "l-btc"
        )
    };
    fn named(v: &Option<String>) -> Option<&str> {
        v.as_deref().filter(|v| !v.trim().is_empty())
    }
    match named(&c.chain).or_else(|| named(&c.asset)) {
        Some(v) if liquid(v) => "LBTC",
        _ => "BTC",
    }
}

/// The Add balance request: the trimmed fields, the connection's asset, the note only when
/// given.
pub fn add_args(wallet: &str, asset: &str, f: &AddForm) -> Value {
    let mut args = json!({
        "wallet": wallet,
        "btc": f.btc.text.trim(),
        "date": f.date.text.trim(),
    });
    if !asset.is_empty() {
        args["asset"] = json!(asset);
    }
    let note = f.note.text.trim();
    if !note.is_empty() {
        args["note"] = json!(note);
    }
    args
}

// ---- Reads and writes ------------------------------------------------------------------------

/// The book-wide read, with the shell's reads and each book refresh cycle's.
pub fn read_book(app: &mut App) {
    app.send(
        "ui.wallets.checkpoints.list",
        json!({}),
        "wallets.checks.book",
    );
}

/// The card's read for `wallet` (another wallet starts the card over).
pub fn read_card(app: &mut App, wallet: &str) {
    let card = &mut app.wallets.checks.card;
    if card.wallet != wallet {
        *card = Card {
            wallet: wallet.to_owned(),
            ..Card::default()
        };
    }
    card.generation = app.book_generation;
    let history = card.history;
    app.send(
        "ui.wallets.checkpoints.list",
        json!({"wallet": wallet, "history": history}),
        "wallets.checks.card",
    );
}

/// Every frame on a connection's page: read when the card has no read for it, and again after
/// a book refresh cycle; keep the connection's asset for Add balance.
pub fn poll(app: &mut App, c: &Conn) -> bool {
    let card = &app.wallets.checks.card;
    let read = card.wallet != c.id || card.generation != app.book_generation;
    if read {
        read_card(app, &c.id);
    }
    app.wallets.checks.card.asset = checkpoint_asset(c);
    read
}

fn toggle_history(a: &mut App) {
    a.wallets.checks.card.history = !a.wallets.checks.card.history;
    a.wallets.checks.card.remove_error = None;
    let wallet = a.wallets.checks.card.wallet.clone();
    read_card(a, &wallet);
}

fn open_add(a: &mut App) {
    a.wallets.checks.card.add = Some(AddForm::default());
}

fn close_add(a: &mut App) {
    a.wallets.checks.card.add = None;
}

fn can_save(f: &AddForm) -> bool {
    !f.pending && !f.btc.text.trim().is_empty() && !f.date.text.trim().is_empty()
}

fn submit_add(a: &mut App) {
    let wallet = a.wallets.checks.card.wallet.clone();
    let asset = a.wallets.checks.card.asset;
    let Some(f) = a.wallets.checks.card.add.as_mut() else {
        return;
    };
    if !can_save(f) {
        return;
    }
    f.pending = true;
    f.error = None;
    let args = add_args(&wallet, asset, f);
    let ticket = a.send_ticket("ui.wallets.checkpoints.add", args, "wallets.checks.add");
    a.wallets.checks.adds.push(ticket.clone());
    if let Some(f) = a.wallets.checks.card.add.as_mut() {
        f.ticket = Some(ticket);
    }
}

fn remove(a: &mut App, id: String) {
    let card = &mut a.wallets.checks.card;
    if card.removing {
        return;
    }
    card.removing = true;
    card.remove_error = None;
    let ticket = a.send_ticket(
        "ui.wallets.checkpoints.remove",
        json!({"id": id}),
        "wallets.checks.remove",
    );
    a.wallets.checks.removes.push(ticket.clone());
    a.wallets.checks.card.remove_ticket = Some(ticket);
}

/// A write changed the book: the page and every screen read again (the web invalidates every
/// daemon query after a mutation).
fn after_write(a: &mut App) {
    let wallet = a.wallets.checks.card.wallet.clone();
    if !wallet.is_empty() {
        read_card(a, &wallet);
    }
    // On a Lightning node page `detail::reload` has no wallet page to re-read (and skips the
    // book-wide reads when none was opened): refresh the list mark, Needs attention and
    // Reports readiness directly.
    if super::detail::shows_wallet(a) {
        super::detail::reload(a);
    } else {
        a.refresh_reads();
    }
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    match tag {
        "wallets.checks.book" => {
            // A failed read decides nothing (an older book may not have the table).
            app.wallets.checks.book = r.error.is_none().then(|| r.data.clone());
        }
        "wallets.checks.card" => {
            app.wallets.checks.card.read = match &r.error {
                Some(e) => Read::Failed(e.clone()),
                None => Read::Ready(r.data.clone()),
            };
        }
        "wallets.checks.add" => {
            // Only the dialog that sent the request takes its answer: one closed meanwhile, or
            // reopened as a new dialog, is not this request's (Codex 4710534560).
            let Some(at) = app.wallets.checks.adds.iter().position(|m| m.answers(app)) else {
                return;
            };
            let ticket = app.wallets.checks.adds.swap_remove(at);
            let card = &mut app.wallets.checks.card;
            let mine = card.add.as_ref().and_then(|f| f.ticket.as_ref()) == Some(&ticket);
            match &r.error {
                Some(e) => {
                    if let Some(f) = card.add.as_mut().filter(|_| mine) {
                        f.pending = false;
                        f.error = Some(e.clone());
                    }
                }
                None => {
                    if mine {
                        card.add = None;
                    }
                    after_write(app);
                }
            }
        }
        "wallets.checks.remove" => {
            // Only the card that sent the request takes its answer: one started over for
            // another connection meanwhile is not this request's (Codex 4710647956).
            let Some(at) = app
                .wallets
                .checks
                .removes
                .iter()
                .position(|m| m.answers(app))
            else {
                return;
            };
            let ticket = app.wallets.checks.removes.swap_remove(at);
            let card = &mut app.wallets.checks.card;
            if card.remove_ticket.as_ref() == Some(&ticket) {
                card.removing = false;
                card.remove_ticket = None;
                if let Some(e) = &r.error {
                    card.remove_error = Some(e.clone());
                }
            }
            if r.error.is_none() {
                after_write(app);
            }
        }
        _ => {}
    }
}

/// Escape closes the Add balance dialog.
pub fn on_key(app: &mut App, ev: &KeyEvent) -> bool {
    if matches!(ev.key, Key::Escape) && app.wallets.checks.card.add.is_some() {
        close_add(app);
        return true;
    }
    false
}

// ---- View ------------------------------------------------------------------------------------

/// A checkpoint's status as a label in its ink (`crate::ui::tag`; the app draws no pills).
fn status_badge(tk: &Tk, status: &str) -> El {
    let (_, _, amber_ink) = amber(tk);
    let (key, ink) = match status {
        "matches" => ("matches", emerald_text(tk)),
        "differs" => ("differs", amber_ink),
        _ => ("unknown", tk.muted_fg),
    };
    crate::ui::tag(
        tk,
        ink,
        None,
        t(&format!("connections:balanceChecks.status.{key}")),
    )
}

fn line(tk: &Tk, s: String) -> El {
    text(s)
        .font(tk.txs(400.0))
        .color(tk.muted_fg)
        .wrap()
        .w_full()
}

fn fact(tk: &Tk, label: String, value: String, strong: bool) -> El {
    row()
        .gap(tk.r(0.75))
        .child(
            text(label)
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .w(tk.r(5.5)),
        )
        .child(
            text(value)
                .font(
                    tk.txs(if strong { 500.0 } else { 400.0 })
                        .features("'tnum' 1"),
                )
                .color(tk.fg),
        )
}

fn checkpoint_row(app: &App, tk: &Tk, i: usize, c: &Value, removable: bool, last: bool) -> El {
    let hidden = app.prefs.hide_sensitive();
    let status = str_of(c, "status");
    let asset = str_of(c, "asset");
    let origin = str_of(c, "origin");
    let origin_key = format!("connections:balanceChecks.origin.{origin}");
    let origin_label = if has(&origin_key) {
        t(&origin_key)
    } else {
        origin.to_owned()
    };
    let as_of =
        crate::settings::certtrust::date_time(str_of(c, "as_of"), crate::i18n::lang(), |s| {
            crate::localtime::offset_secs(s as i64)
        });
    let head = row()
        .w_full()
        .gap(tk.r(0.5))
        .center_y()
        .wrap()
        .child(text(origin_label).font(tk.tsm(500.0)).color(tk.fg))
        .child(status_badge(tk, status))
        .child(
            text(tr("connections:balanceChecks.asOf", &[("date", &as_of)]))
                .font(tk.txs(400.0))
                .color(tk.muted_fg),
        );
    let mut facts = col()
        .gap(tk.r(0.125))
        .child(fact(
            tk,
            t("connections:balanceChecks.reported"),
            amount(c["reported_btc"].as_str(), asset, false, hidden),
            false,
        ))
        .child(fact(
            tk,
            t("connections:balanceChecks.book"),
            amount(c["book_btc"].as_str(), asset, false, hidden),
            false,
        ));
    let difference = c["difference_btc"].as_str().filter(|d| !d.is_empty());
    if status == "differs"
        && let Some(d) = difference
    {
        facts = facts.child(fact(
            tk,
            t("connections:balanceChecks.difference"),
            amount(Some(d), asset, true, hidden),
            true,
        ));
    }
    let mut words = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(tk.r(0.25))
        .child(head)
        .child(facts);
    if status == "differs" {
        let mut hint = t("connections:balanceChecks.differsHint");
        let scope_key = format!("connections:balanceChecks.scope.{}", str_of(c, "scope"));
        if !str_of(c, "scope").is_empty() && has(&scope_key) {
            hint.push(' ');
            hint.push_str(&t(&scope_key));
        }
        words = words.child(line(tk, hint));
        let notes_wait = c
            .get("notes")
            .and_then(Value::as_array)
            .is_some_and(|n| n.iter().any(|x| x == "after_last_sync"));
        if notes_wait {
            words = words.child(line(tk, t("connections:balanceChecks.incompleteUntilSync")));
        }
    }
    if status != "matches" && status != "differs" {
        let reason_key = format!("connections:balanceChecks.reason.{}", str_of(c, "reason"));
        let reason = if has(&reason_key) {
            t(&reason_key)
        } else {
            str_of(c, "message").to_owned()
        };
        if !reason.is_empty() {
            words = words.child(line(tk, reason));
        }
    }
    // A note can quote a balance (`Statement balance: 0.5 BTC`): masked like the amounts.
    let note = str_of(c, "note");
    if !note.is_empty() {
        words = words.child(
            crate::ui::sensitive(note.to_owned(), tk.txs(400.0), tk.muted_fg, hidden)
                .wrap()
                .w_full(),
        );
    }
    let mut r = row()
        .key(format!("wd-checks-row{i}"))
        .w_full()
        .py(tk.r(0.75))
        .gap(tk.r(0.75))
        .items_start()
        .role("listitem")
        .child(words);
    if !last {
        r = r
            .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
            .border_layout();
    }
    let id = c["id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    if removable
        && c["stored"] == true
        && let Some(id) = id
    {
        let enabled = !app.wallets.checks.card.removing;
        let mut b = button(
            tk,
            format!("wd-checks-remove{i}"),
            Variant::Ghost,
            Size::Sm,
            Some(&I::TRASH_2),
            &t("connections:balanceChecks.remove"),
            enabled,
        );
        if enabled {
            b = b.on_press(move |a: &mut App| remove(a, id.clone()));
        }
        r = r.child(b);
    }
    r
}

/// `BalanceChecksCard`: below the hero (and an exchange's sync card), above the notices.
pub fn card(app: &App, tk: &Tk, c: &Conn) -> El {
    let card = &app.wallets.checks.card;
    let history = card.history;
    let mut history_btn = button(
        tk,
        "wd-checks-history",
        Variant::Outline,
        Size::Sm,
        Some(&I::HISTORY),
        &t(if history {
            "connections:balanceChecks.latestOnly"
        } else {
            "connections:balanceChecks.history"
        }),
        true,
    )
    .checked(history)
    .on_press(toggle_history);
    if history {
        history_btn = history_btn.fill(tk.muted);
    }
    let head = dui::strip(tk)
        .items_start()
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(tk.r(0.25))
                .child(
                    row()
                        .gap(tk.r(0.5))
                        .center_y()
                        .child(ic(&I::SCALE, tk.r(1.0), tk.fg))
                        .child(dui::card_title(tk, &t("connections:balanceChecks.title"))),
                )
                .child(line(tk, t("connections:balanceChecks.description"))),
        )
        .child(
            row().gap(tk.r(0.5)).child(history_btn).child(
                button(
                    tk,
                    "wd-checks-add",
                    Variant::Default,
                    Size::Sm,
                    Some(&I::PLUS),
                    &t("connections:balanceChecks.addButton"),
                    true,
                )
                .on_press(open_add),
            ),
        );
    let mut body = col().w_full().px(tk.r(1.0)).pb(tk.r(0.25));
    let rows = card_rows(card);
    let mine = card.wallet == c.id;
    let state = |s: String| line(tk, s).py(tk.r(0.75));
    body = match &card.read {
        _ if !mine => body.child(state(t("connections:balanceChecks.loading"))),
        Read::Loading => body.child(state(t("connections:balanceChecks.loading"))),
        Read::Failed(_) => {
            body.child(state(t("connections:balanceChecks.unavailable")).role("status"))
        }
        Read::Ready(_) if rows.is_empty() => {
            body.child(state(t("connections:balanceChecks.empty")))
        }
        Read::Ready(report) => {
            let journals = report["journals"]["status"].as_str().unwrap_or("");
            if journals == "stale" || journals == "not_processed" {
                body = body.child(
                    line(tk, t("connections:balanceChecks.journalsStale"))
                        .pt(tk.r(0.75))
                        .role("status"),
                );
            }
            let n = rows.len();
            let mut list = col()
                .w_full()
                .role("list")
                .label(t("connections:balanceChecks.title"));
            for (i, cp) in rows.into_iter().enumerate() {
                list = list.child(checkpoint_row(app, tk, i, cp, history, i + 1 == n));
            }
            body.child(list)
        }
    };
    if let Some(e) = &card.remove_error {
        body = body.child(dui::error_line(tk, e).pb(tk.r(0.75)));
    }
    dui::card(tk)
        .key("wd-checks")
        .role("region")
        .label(t("connections:balanceChecks.title"))
        .child(head)
        .child(body)
}

/// The Add balance dialog.
pub fn mount(app: &App, tk: &Tk, layer: &mut Vec<El>) {
    let Some(f) = app.wallets.checks.card.add.as_ref() else {
        return;
    };
    fn form(a: &mut App) -> &mut AddForm {
        a.wallets
            .checks
            .card
            .add
            .get_or_insert_with(Default::default)
    }
    let amount_label = t("connections:balanceChecks.add.amount");
    let date_label = t("connections:balanceChecks.add.date");
    let note_label = t("connections:balanceChecks.add.note");
    let mut items = vec![
        dui::dialog_header(
            tk,
            &t("connections:balanceChecks.add.title"),
            &t("connections:balanceChecks.add.body"),
        ),
        dui::group(
            tk,
            vec![
                dui::label(tk, &amount_label),
                dui::input_submit(
                    tk,
                    "wd-checks-btc",
                    &f.btc,
                    &amount_label,
                    "0.12345678",
                    |a| &mut form(a).btc,
                    false,
                    submit_add,
                ),
            ],
        ),
        dui::group(
            tk,
            vec![
                dui::label(tk, &date_label),
                dui::input_submit(
                    tk,
                    "wd-checks-date",
                    &f.date,
                    &date_label,
                    "YYYY-MM-DD",
                    |a| &mut form(a).date,
                    false,
                    submit_add,
                ),
                dui::helper(tk, &t("connections:balanceChecks.add.dateHint")),
            ],
        ),
        dui::group(
            tk,
            vec![
                dui::label(tk, &note_label),
                dui::input_submit(
                    tk,
                    "wd-checks-note",
                    &f.note,
                    &note_label,
                    "",
                    |a| &mut form(a).note,
                    false,
                    submit_add,
                ),
            ],
        ),
    ];
    if let Some(e) = &f.error {
        items.push(dui::error_line(tk, e));
    }
    let ok = can_save(f);
    let mut save = dui::btn(
        tk,
        "wd-checks-save",
        Variant::Default,
        Size::Default,
        None,
        &t("connections:balanceChecks.add.save"),
        ok,
    );
    if ok {
        save = save.on_press(submit_add);
    }
    items.push(dui::dialog_footer(
        tk,
        vec![
            dui::btn(
                tk,
                "wd-checks-cancel",
                Variant::Outline,
                Size::Default,
                None,
                &t("connections:balanceChecks.add.cancel"),
                true,
            )
            .on_press(close_add),
            save,
        ],
    ));
    dui::dialog(
        app,
        tk,
        layer,
        "wd-checks-add-dialog",
        tk.r(28.0),
        &t("connections:balanceChecks.add.title"),
        col().w_full().gap(tk.r(1.0)).children(items),
        close_add,
    );
}

/// The wallets list's mark beside a differing wallet's name.
pub fn mark(tk: &Tk) -> El {
    let (_, _, ink) = amber(tk);
    let label = t("connections:balanceChecks.indicator");
    row().role("img").label(label.clone()).help(label).child(ic(
        &I::TRIANGLE_ALERT,
        tk.r(0.875),
        ink,
    ))
}

// ---- Scenes ----------------------------------------------------------------------------------

/// SYNTHETIC: a differing exchange balance and a matching chain one on the open wallet's card,
/// and the same wallet in the book-wide read (the wallets list mark, Needs attention, Reports).
/// Nothing is written to the book.
pub fn scene_sample(a: &mut App, history: bool) {
    let id = a.wallets.detail.wallet.clone();
    let label = crate::wallets::conn::find(a, &id)
        .map(|c| c.label)
        .unwrap_or_default();
    let rows = json!([
        {"id": "cp-1", "stored": true, "wallet_id": id, "wallet": label, "asset": "BTC",
         "as_of": "2026-11-13T12:34:00Z", "origin": "manual", "scope": null,
         "note": "Statement 2026-11", "reported_btc": "0.5", "book_btc": "0.4",
         "difference_btc": "0.1", "status": "differs", "reason": "", "message": "",
         "latest": true},
        {"id": null, "stored": false, "wallet_id": id, "wallet": label, "asset": "BTC",
         "as_of": "2026-11-12T08:00:00Z", "origin": "chain", "scope": "chain_utxo_set",
         "note": null, "reported_btc": "0.4", "book_btc": "0.4", "difference_btc": "0",
         "status": "matches", "reason": "", "message": "", "latest": true},
    ]);
    let report = json!({"journals": {"status": "current"}, "checkpoints": rows});
    let card = &mut a.wallets.checks.card;
    card.wallet = id;
    card.history = history;
    card.generation = a.book_generation;
    card.read = Read::Ready(report.clone());
    a.wallets.checks.book = Some(report);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(rows: Value) -> Value {
        json!({"journals": {"status": "current"}, "checkpoints": rows})
    }

    #[test]
    fn differing_wallets_group_and_wait_only_when_every_check_waits() {
        let r = report(json!([
            {"wallet_id": "w1", "wallet": "Kraken", "status": "differs", "reason": ""},
            {"wallet_id": "w2", "wallet": "Cold", "status": "matches", "reason": ""},
            {"wallet_id": "w3", "wallet": "Node", "status": "unknown", "reason": "after_last_sync"},
            {"wallet_id": "w1", "wallet": "Kraken", "status": "unknown", "reason": "after_last_sync"},
            {"wallet_id": "w4", "wallet": "Old", "status": "unknown", "reason": "journals_stale"},
        ]));
        assert_eq!(
            wallet_diffs(&r),
            vec![
                WalletDiff {
                    wallet_id: "w1".into(),
                    wallet: "Kraken".into(),
                    pending: false
                },
                WalletDiff {
                    wallet_id: "w3".into(),
                    wallet: "Node".into(),
                    pending: true
                },
            ]
        );
        assert!(wallet_diffs(&json!({})).is_empty());
    }

    fn diff(id: &str, name: &str, pending: bool) -> WalletDiff {
        WalletDiff {
            wallet_id: id.into(),
            wallet: name.into(),
            pending,
        }
    }

    #[test]
    fn the_condition_names_one_wallet_and_counts_several() {
        crate::i18n::set_lang(crate::i18n::Lang::En);
        let one = [diff("w1", "Kraken 2", false)];
        let (title, body, details) = condition_text(&one, Privacy::Normal);
        assert_eq!(
            title,
            "Kraken 2: the reported balance differs from the book"
        );
        assert_eq!(
            body,
            "The source may be incomplete. The book is not changed."
        );
        assert!(details.is_empty());
        assert_eq!(condition_route(&one), "/connections/w1");
        let (masked, _, _) = condition_text(&one, Privacy::HideAmounts);
        assert!(masked.starts_with(crate::theme::MASK), "{masked}");
        let (all_hidden, _, _) = condition_text(&one, Privacy::HideAll);
        assert_eq!(all_hidden, "A reported balance differs from the book");

        let two = [diff("w1", "Kraken", true), diff("w3", "Node", false)];
        let (title, _, details) = condition_text(&two, Privacy::Normal);
        assert_eq!(
            title,
            "2 wallets report a balance that differs from the book"
        );
        assert_eq!(details, ["Kraken", "Node"]);
        assert_eq!(condition_route(&two), "/connections");
        assert!(condition_text(&two, Privacy::HideAll).2.is_empty());

        let waiting = [diff("w3", "Node", true)];
        let (title, _, _) = condition_text(&waiting, Privacy::Normal);
        assert_eq!(title, "Node: a balance check waits for the next sync");
    }

    #[test]
    fn readiness_waits_only_when_every_wallet_waits() {
        crate::i18n::set_lang(crate::i18n::Lang::En);
        assert!(readiness_text(&[]).is_none());
        let (title, detail) =
            readiness_text(&[diff("w1", "Kraken", false), diff("w3", "Node", true)]).unwrap();
        assert_eq!(title, "Reported balance differs");
        assert!(detail.starts_with("Kraken, Node: "), "{detail}");
        let (title, _) = readiness_text(&[diff("w3", "Node", true)]).unwrap();
        assert_eq!(title, "Balance check waits for a sync");
    }

    #[test]
    fn amounts_stay_exact_and_hide() {
        assert_eq!(amount(Some("0.5"), "BTC", false, false), "0.5 BTC");
        assert_eq!(
            amount(Some("0.00000001"), "LBTC", true, false),
            "+0.00000001 LBTC"
        );
        assert_eq!(amount(Some("-0.1"), "BTC", true, false), "-0.1 BTC");
        assert_eq!(amount(None, "BTC", false, false), "—");
        assert_ne!(amount(Some("0.5"), "BTC", false, true), "0.5 BTC");
    }

    #[test]
    fn add_sends_trimmed_fields_and_a_note_only_when_given() {
        let mut f = AddForm {
            btc: TextEdit::new(" 0.12345678 "),
            date: TextEdit::new("2024-12-31 "),
            ..AddForm::default()
        };
        assert_eq!(
            add_args("w1", "BTC", &f),
            json!({"wallet": "w1", "btc": "0.12345678", "date": "2024-12-31", "asset": "BTC"})
        );
        assert!(can_save(&f));
        f.note = TextEdit::new(" statement ");
        assert_eq!(add_args("w1", "BTC", &f)["note"], "statement");
        f.pending = true;
        assert!(!can_save(&f));
        assert!(!can_save(&AddForm::default()));
    }

    /// A Liquid connection's balance is LBTC; every other connection's (a Lightning node
    /// included) is BTC. The dialog sends it, so the checkpoint compares with the right postings.
    #[test]
    fn add_sends_the_connection_s_asset() {
        let liquid = Conn::parse(&json!({"id": "l1", "kind": "descriptor", "chain": "liquid"}));
        let node = Conn::parse(&json!({"id": "n1", "kind": "core-ln"}));
        let chain = Conn::parse(&json!({"id": "w1", "kind": "descriptor"}));
        assert_eq!(checkpoint_asset(&liquid), "LBTC");
        assert_eq!(checkpoint_asset(&node), "BTC");
        assert_eq!(checkpoint_asset(&chain), "BTC");
        // Codex 4710534574: the user's label never decides the stored asset.
        let named = Conn::parse(
            &json!({"id": "w2", "kind": "descriptor", "chain": "bitcoin", "label": "Liquid savings"}),
        );
        assert_eq!(checkpoint_asset(&named), "BTC");
        let unnamed =
            Conn::parse(&json!({"id": "w3", "kind": "descriptor", "label": "L-BTC stash"}));
        assert_eq!(checkpoint_asset(&unnamed), "BTC");
        let f = AddForm {
            btc: TextEdit::new("0.5"),
            date: TextEdit::new("2024-12-31"),
            ..AddForm::default()
        };
        assert_eq!(add_args("l1", "LBTC", &f)["asset"], "LBTC");
        // The card keeps the asset of the connection its page shows.
        let mut app = crate::app::tests::quiet_app();
        app.wallets.checks.card.wallet = "l1".into();
        app.wallets.checks.card.generation = app.book_generation;
        poll(&mut app, &liquid);
        assert_eq!(app.wallets.checks.card.asset, "LBTC");
    }

    /// The Reports notice names wallets as the privacy mode allows: none under Hide all, and
    /// under Hide amounts not a name that carries digits.
    #[test]
    fn readiness_names_follow_the_privacy_mode() {
        crate::i18n::set_lang(crate::i18n::Lang::En);
        let diffs = [diff("w1", "Kraken", false), diff("w2", "Cold 21", false)];
        let (_, normal) = readiness_text(&shown(&diffs, Privacy::Normal)).unwrap();
        assert!(normal.starts_with("Kraken, Cold 21: "), "{normal}");
        let (_, amounts) = readiness_text(&shown(&diffs, Privacy::HideAmounts)).unwrap();
        assert!(
            amounts.starts_with(&format!("Kraken, {}: ", crate::theme::MASK)),
            "{amounts}"
        );
        let (_, all) = readiness_text(&shown(&diffs, Privacy::HideAll)).unwrap();
        assert!(
            all.starts_with(&format!("{}: ", crate::theme::MASK)),
            "{all}"
        );
        assert!(!all.contains("Kraken") && !all.contains("Cold"), "{all}");
        // The page reads them through the app's mode.
        let mut app = crate::app::tests::quiet_app();
        app.wallets.checks.book = Some(report(json!([
            {"wallet_id": "w1", "wallet": "Kraken", "status": "differs", "reason": ""},
        ])));
        app.prefs.privacy = Privacy::HideAll;
        assert_eq!(readiness_diffs(&app)[0].wallet, crate::theme::MASK);
    }

    #[test]
    fn the_card_shows_only_its_wallets_rows() {
        let card = Card {
            wallet: "w1".into(),
            read: Read::Ready(report(json!([
                {"wallet_id": "w1", "status": "matches"},
                {"wallet_id": "w2", "status": "differs"},
            ]))),
            ..Card::default()
        };
        assert_eq!(card_rows(&card).len(), 1);
    }

    /// Codex 4659580909: adding or removing a balance on a Lightning node page (no wallet page
    /// open) starts a book refresh, which re-reads the book-wide checks, so the list mark, Needs attention and Reports
    /// readiness follow the change.
    #[test]
    fn a_write_off_the_wallet_page_re_reads_the_book_wide_checks() {
        let mut app = crate::app::tests::quiet_app();
        app.wallets.checks.card.wallet = "node-1".into();
        let removal = sent(&mut app, |a| remove(a, "c1".into()));
        let before: Vec<String> = app.sent_args.keys().cloned().collect();
        let reply = Reply {
            kind: "ui.wallets.checkpoints.remove".into(),
            data: json!({}),
            error: None,
            error_detail: Value::Null,
            elapsed: Default::default(),
        };
        app.deliver_as("wallets.checks.remove", &removal, reply);
        let reads: Vec<&Value> = app
            .sent_args
            .iter()
            .filter(|(id, _)| !before.contains(id))
            .map(|(_, args)| args)
            .collect();
        // A book refresh cycle starts (its revision read first); the cycle re-reads the
        // book-wide checks with the shell's other reads.
        assert!(
            reads.iter().any(|args| **args == json!({"revision": true})),
            "a book refresh: {reads:?}"
        );
        assert!(
            reads.iter().any(|args| args["wallet"] == "node-1"),
            "the card's read: {reads:?}"
        );
    }

    fn sent(app: &mut App, send: fn(&mut App)) -> String {
        let before: Vec<String> = app.pending.keys().cloned().collect();
        send(app);
        app.pending
            .keys()
            .find(|id| !before.contains(id))
            .cloned()
            .expect("a request")
    }

    fn add_reply(error: Option<&str>) -> Reply {
        Reply {
            kind: "ui.wallets.checkpoints.add".into(),
            data: json!({}),
            error: error.map(str::to_owned),
            error_detail: Value::Null,
            elapsed: Default::default(),
        }
    }

    /// Codex 4710534560: a balance saved from a dialog that was closed and reopened answers
    /// into neither the new dialog's draft nor its error; the new dialog's own answer does.
    #[test]
    fn a_closed_dialogs_answer_leaves_the_reopened_dialog_alone() {
        let mut app = crate::app::tests::quiet_app();
        app.wallets.checks.card.wallet = "w1".into();
        let filled = |a: &mut App| {
            open_add(a);
            let f = a.wallets.checks.card.add.as_mut().unwrap();
            f.btc = TextEdit::new("0.5");
            f.date = TextEdit::new("2024-12-31");
        };
        filled(&mut app);
        let first = sent(&mut app, submit_add);
        close_add(&mut app);
        filled(&mut app);
        let second = sent(&mut app, submit_add);
        app.deliver_as("wallets.checks.add", &first, add_reply(Some("refused")));
        let f = app
            .wallets
            .checks
            .card
            .add
            .as_ref()
            .expect("the new dialog");
        assert!(
            f.pending && f.error.is_none(),
            "the old failure is not the new dialog's"
        );
        app.deliver_as("wallets.checks.add", &second, add_reply(None));
        assert!(
            app.wallets.checks.card.add.is_none(),
            "its own answer closes it"
        );

        filled(&mut app);
        let third = sent(&mut app, submit_add);
        close_add(&mut app);
        open_add(&mut app);
        app.deliver_as("wallets.checks.add", &third, add_reply(None));
        assert!(
            app.wallets.checks.card.add.is_some(),
            "the old success does not close the new dialog"
        );
        assert!(app.wallets.checks.adds.is_empty());
    }

    /// Codex 4710647956: a removal answered after the page moved to another connection leaves
    /// that card's pending removal and error alone; the book still reads again on success.
    #[test]
    fn a_removal_answers_only_the_card_that_sent_it() {
        let mut app = crate::app::tests::quiet_app();
        app.wallets.checks.card.wallet = "w1".into();
        let first = sent(&mut app, |a| remove(a, "c1".into()));
        read_card(&mut app, "w2");
        let second = sent(&mut app, |a| remove(a, "c2".into()));
        app.deliver_as("wallets.checks.remove", &first, add_reply(Some("refused")));
        let card = &app.wallets.checks.card;
        assert!(
            card.removing && card.remove_error.is_none(),
            "the old failure is not the new card's"
        );
        let before: Vec<String> = app.sent_args.keys().cloned().collect();
        app.deliver_as("wallets.checks.remove", &second, add_reply(None));
        assert!(!app.wallets.checks.card.removing, "its own answer ends it");
        assert!(
            app.sent_args
                .iter()
                .any(|(id, args)| !before.contains(id) && args["wallet"] == "w2"),
            "a successful removal reads the card again"
        );
        assert!(app.wallets.checks.removes.is_empty());
    }

    /// Codex 4710534589: every scope `kassiber_wallets::checkpoints::scope_note` explains has
    /// its translation under the same name, in both languages.
    #[test]
    fn every_backend_scope_has_its_note() {
        const SCOPES: [&str; 10] = [
            "spot_free_plus_locked",
            "wallets_balance_after_latest_transaction",
            "spot_available_plus_in_order",
            "account_available_plus_reserved",
            "exchange_and_funding_wallets",
            "account_statement_balance_after_last_booking",
            "kraken_ledger_balance",
            "file_running_balance",
            "chain_utxo_set",
            "cln_bookkeeper_accounts",
        ];
        let bundles = [
            (
                "en",
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../assets/i18n/locales/en/connections.json"
                )),
            ),
            (
                "de",
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../assets/i18n/locales/de/connections.json"
                )),
            ),
        ];
        for (lang, bundle) in bundles {
            let bundle: Value = serde_json::from_str(bundle).expect("a locale bundle");
            for scope in SCOPES {
                let note = bundle.pointer(&format!("/balanceChecks/scope/{scope}"));
                assert!(
                    note.and_then(Value::as_str).is_some_and(|n| !n.is_empty()),
                    "{lang}: balanceChecks.scope.{scope}"
                );
            }
        }
    }
}
