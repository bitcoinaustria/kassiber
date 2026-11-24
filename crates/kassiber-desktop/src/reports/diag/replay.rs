//! Logs → Export → Replay bundle (`ReplayBundleDialog`, docs/reference/replay-bundle.md): the
//! owner reads what the file holds, ticks that they understand it, and sets a passphrase twice.
//! The daemon writes a managed file under exports/diagnostics and returns its path; the app never
//! names one. The passphrase is cleared once the request settles and never leaves the request
//! (the daemon log ring redacts `passphrase`).

use rune::{Color, El, TextEdit, col, row, text};
use serde_json::{Value, json};

use crate::app::{App, Reply};
use crate::i18n::{t, tc};
use crate::icons as I;
use crate::settings::controls::{checkbox, input, labelled};
use crate::theme::Tk;
use crate::txdetail::widgets::{Size, Variant, button};
use crate::ui::ic;

pub const KIND: &str = "ui.diagnostics.replay_bundle.export";
const TAG: &str = "reports.diag.replay.export";
/// The daemon's minimum, checked again server-side.
pub const MIN_PASSPHRASE: usize = 8;

#[derive(Default)]
pub struct State {
    pub open: bool,
    pub understood: bool,
    pub passphrase: TextEdit,
    pub confirmation: TextEdit,
    pub pending: bool,
    pub export: Option<crate::app::Ticket>,
    /// The saved bundle's path, as the daemon reports it.
    pub saved: Option<String>,
    pub error: Option<String>,
}

impl State {
    /// A passphrase is typed but shorter than the minimum.
    pub fn too_short(&self) -> bool {
        let n = self.passphrase.text.chars().count();
        n > 0 && n < MIN_PASSPHRASE
    }

    pub fn mismatch(&self) -> bool {
        !self.confirmation.text.is_empty() && self.confirmation.text != self.passphrase.text
    }

    /// Consent and a confirmed passphrase; matching passphrases alone are not consent.
    pub fn ready(&self) -> bool {
        self.understood
            && self.passphrase.text.chars().count() >= MIN_PASSPHRASE
            && self.confirmation.text == self.passphrase.text
            && !self.pending
    }

    pub fn args(&self) -> Value {
        json!({"confirm": true, "passphrase": self.passphrase.text.as_str()})
    }
}

fn st(a: &mut App) -> &mut State {
    &mut a.reports.diag.replay
}

pub fn open(a: &mut App) {
    *st(a) = State {
        open: true,
        ..State::default()
    };
}

/// Closing forgets everything, the passphrase included.
pub fn close(a: &mut App) {
    *st(a) = State::default();
}

fn submit(a: &mut App) {
    if !a.reports.diag.replay.ready() {
        return;
    }
    let args = a.reports.diag.replay.args();
    let s = st(a);
    s.error = None;
    s.pending = true;
    st(a).export = Some(a.send_ticket(KIND, args, TAG));
}

pub fn on_reply(a: &mut App, tag: &str, r: &Reply) {
    if tag != TAG {
        return;
    }
    if !a
        .reports
        .diag
        .replay
        .export
        .as_ref()
        .is_some_and(|t| t.answers(a))
    {
        return;
    }
    let s = st(a);
    s.export = None;
    s.pending = false;
    s.passphrase = TextEdit::new("");
    s.confirmation = TextEdit::new("");
    match &r.error {
        Some(e) => s.error = Some(e.clone()),
        None => {
            s.saved = r
                .data
                .get("file")
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
    }
}

fn line(tk: &Tk, s: String, color: Color) -> El {
    text(s).font(tk.tsm(400.0)).color(color).wrap().w_full()
}

fn form(app: &App, tk: &Tk) -> El {
    let s = &app.reports.diag.replay;
    let mut pass = col().w_full().gap(tk.r(0.25)).child(labelled(
        tk,
        &t("review:logs.replayBundle.passphrase"),
        input(
            tk,
            &s.passphrase,
            "replay-bundle-passphrase",
            &t("review:logs.replayBundle.passphrase"),
            "",
            true,
            !s.pending,
            tk.r(2.25),
            false,
            |a| &mut a.reports.diag.replay.passphrase,
        ),
    ));
    if s.too_short() {
        pass = pass.child(
            text(tc(
                "review:logs.replayBundle.tooShort",
                MIN_PASSPHRASE,
                &[("count", &MIN_PASSPHRASE.to_string())],
            ))
            .font(tk.txs(400.0))
            .color(tk.destructive),
        );
    }
    let mut repeat = col().w_full().gap(tk.r(0.25)).child(labelled(
        tk,
        &t("review:logs.replayBundle.confirmPassphrase"),
        input(
            tk,
            &s.confirmation,
            "replay-bundle-confirm",
            &t("review:logs.replayBundle.confirmPassphrase"),
            "",
            true,
            !s.pending,
            tk.r(2.25),
            false,
            |a| &mut a.reports.diag.replay.confirmation,
        ),
    ));
    if s.mismatch() {
        repeat = repeat.child(
            text(t("review:logs.replayBundle.mismatch"))
                .font(tk.txs(400.0))
                .color(tk.destructive),
        );
    }
    let understood = t("review:logs.replayBundle.understood");
    let mut c = col()
        .w_full()
        .gap(tk.r(0.75))
        .child(
            row()
                .w_full()
                .gap(tk.r(0.5))
                .items_start()
                .padding(tk.r(0.75))
                .radius(tk.r(0.375))
                .border(1.0, tk.border)
                .border_layout()
                .child(ic(&I::SHIELD_ALERT, tk.r(1.0), tk.fg).a11y_hidden())
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(tk.r(0.25))
                        .child(line(tk, t("review:logs.replayBundle.contains"), tk.fg))
                        .child(line(tk, t("review:logs.replayBundle.excludes"), tk.fg)),
                ),
        )
        .child(
            row()
                .w_full()
                .gap(tk.r(0.5))
                .items_start()
                .child(checkbox(
                    tk,
                    "replay-bundle-understood",
                    s.understood,
                    &understood,
                    |a: &mut App| {
                        let s = &mut a.reports.diag.replay;
                        s.understood = !s.understood;
                    },
                ))
                .child(line(tk, understood.clone(), tk.fg)),
        )
        .child(pass)
        .child(repeat);
    if let Some(e) = &s.error {
        c = c.child(line(tk, e.clone(), tk.destructive).role("alert"));
    }
    c
}

fn saved(tk: &Tk, file: &str) -> El {
    col()
        .w_full()
        .gap(tk.r(0.5))
        .role("status")
        .child(
            text(t("review:logs.replayBundle.saved"))
                .font(tk.tsm(500.0))
                .color(tk.fg),
        )
        .child(
            text(file.to_owned())
                .font(tk.txs(400.0).mono())
                .color(tk.fg)
                .wrap_anywhere()
                .wrap()
                .w_full(),
        )
        .child(line(
            tk,
            t("review:logs.replayBundle.sharePassphrase"),
            tk.muted_fg,
        ))
}

/// The modal, pushed onto the overlay layer while open.
pub fn dialog(app: &App, tk: &Tk, layer: &mut Vec<El>) {
    let s = &app.reports.diag.replay;
    if !s.open {
        return;
    }
    let dw = tk.r(32.0);
    let footer = row()
        .w_full()
        .px(tk.r(1.25))
        .py(tk.r(1.0))
        .gap(tk.r(0.5))
        .justify_end()
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border);
    let (body, footer) = match &s.saved {
        Some(file) => (
            saved(tk, file),
            footer.child(
                button(
                    tk,
                    "replay-bundle-done",
                    Variant::Default,
                    Size::Default,
                    None,
                    &t("common:actions.close"),
                    true,
                )
                .on_press(close),
            ),
        ),
        None => {
            let ready = s.ready();
            let mut export = button(
                tk,
                "replay-bundle-export",
                Variant::Default,
                Size::Default,
                None,
                &t("review:logs.replayBundle.export"),
                ready,
            );
            if ready {
                export = export.on_press(submit);
            }
            (
                form(app, tk),
                footer
                    .child(
                        button(
                            tk,
                            "replay-bundle-cancel",
                            Variant::Outline,
                            Size::Default,
                            None,
                            &t("common:actions.cancel"),
                            true,
                        )
                        .on_press(close),
                    )
                    .child(export),
            )
        }
    };
    let d = col()
        .key("replay-bundle")
        .w(dw)
        .radius(tk.r(0.5))
        .fill(tk.background)
        .border(1.0, tk.border)
        .clip_rounded(tk.r(0.5))
        .role("dialog")
        .label(t("review:logs.replayBundle.title"))
        .description(t("review:logs.replayBundle.description"))
        .modal(true, close)
        .on_press(|_a: &mut App| {})
        .child(
            col()
                .w_full()
                .px(tk.r(1.25))
                .py(tk.r(1.0))
                .pr(tk.r(3.0))
                .gap(tk.r(0.5))
                .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
                .child(
                    text(t("review:logs.replayBundle.title"))
                        .font(tk.tlg(600.0).lh(tk.r(1.125)))
                        .color(tk.fg),
                )
                .child(
                    text(t("review:logs.replayBundle.description"))
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg)
                        .wrap()
                        .w_full(),
                ),
        )
        .child(col().w_full().px(tk.r(1.25)).py(tk.r(1.0)).child(body))
        .child(footer)
        .child(
            row()
                .key("replay-bundle-close")
                .absolute(dw - tk.r(1.0) - tk.r(1.0), tk.r(1.0))
                .size(tk.r(1.0), tk.r(1.0))
                .center()
                .opacity(0.7)
                .role("button")
                .label(t("common:actions.close"))
                .focusable(true)
                .on_press(close)
                .child(ic(&I::X, tk.r(1.0), tk.fg)),
        );
    layer.push(
        row()
            .key("replay-bundle-wrap")
            .label(t("common:actions.close"))
            .pin(
                Some(0.0),
                Some(crate::shell::window_top_inset()),
                Some(0.0),
                Some(0.0),
            )
            .center()
            .fill(Color::BLACK.alpha(0.5))
            .on_press(close)
            .child(d),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(understood: bool, pass: &str, repeat: &str) -> State {
        State {
            open: true,
            understood,
            passphrase: TextEdit::new(pass),
            confirmation: TextEdit::new(repeat),
            ..State::default()
        }
    }

    #[test]
    fn exports_only_after_consent_and_a_confirmed_passphrase() {
        assert!(!with(false, "", "").ready());
        let short = with(true, "short", "short");
        assert!(short.too_short());
        assert!(!short.ready());
        let off = with(true, "long enough passphrase", "long enough passphrasX");
        assert!(off.mismatch());
        assert!(!off.ready());
        // Matching passphrases alone are not consent.
        assert!(!with(false, "long enough passphrase", "long enough passphrase").ready());
        let ok = with(true, "long enough passphrase", "long enough passphrase");
        assert!(ok.ready() && !ok.too_short() && !ok.mismatch());
        assert_eq!(
            ok.args(),
            json!({"confirm": true, "passphrase": "long enough passphrase"})
        );
        // A request in flight is not ready again.
        assert!(
            !State {
                pending: true,
                ..ok
            }
            .ready()
        );
    }

    /// An export still running when the dialog closes answers nobody: the reopened dialog keeps
    /// its own passphrase and never shows the earlier file as its result.
    #[test]
    fn a_reply_lands_only_in_the_dialog_that_sent_it() {
        let mut app = crate::app::tests::quiet_app();
        open(&mut app);
        let (earlier, ticket) = crate::app::tests::inject_ticket(&mut app, KIND, TAG);
        st(&mut app).export = Some(ticket);
        st(&mut app).pending = true;
        close(&mut app);
        open(&mut app);
        st(&mut app).passphrase = TextEdit::new("new passphrase");
        crate::app::tests::answer(
            &mut app,
            &earlier,
            KIND,
            json!({"file": "/old.kassiber-replay"}),
        );
        app.pump();
        let s = &app.reports.diag.replay;
        assert_eq!(
            (s.saved.as_deref(), s.passphrase.text.as_str()),
            (None, "new passphrase")
        );
        // The dialog's own request lands.
        let (current, ticket) = crate::app::tests::inject_ticket(&mut app, KIND, TAG);
        st(&mut app).export = Some(ticket);
        st(&mut app).pending = true;
        crate::app::tests::answer(
            &mut app,
            &current,
            KIND,
            json!({"file": "/new.kassiber-replay"}),
        );
        app.pump();
        let s = &app.reports.diag.replay;
        assert_eq!(s.saved.as_deref(), Some("/new.kassiber-replay"));
        assert!(!s.pending && s.export.is_none() && s.passphrase.text.is_empty());
    }

    /// Consent belongs to the book it was given for: leaving Logs or switching books forgets
    /// the dialog, passphrase and confirmation included.
    #[test]
    fn leaving_logs_or_switching_books_forgets_the_consent() {
        let consented = |app: &mut App| {
            open(app);
            let s = st(app);
            s.understood = true;
            s.passphrase = TextEdit::new("long enough passphrase");
            s.confirmation = TextEdit::new("long enough passphrase");
            assert!(app.reports.diag.replay.ready());
        };
        let mut app = crate::app::tests::quiet_app();
        consented(&mut app);
        super::super::leave(&mut app, "/logs");
        assert!(!app.reports.diag.replay.open && !app.reports.diag.replay.understood);
        consented(&mut app);
        app.reset_book_state();
        let s = &app.reports.diag.replay;
        assert!(!s.open && !s.understood && s.passphrase.text.is_empty());
    }

    #[test]
    fn the_minimum_counts_characters_not_bytes() {
        // Eight characters, more than eight bytes.
        let s = with(true, "äöüßäöüß", "äöüßäöüß");
        assert!(!s.too_short());
        assert!(s.ready());
        assert!(with(true, "äöüßäöü", "").too_short());
    }

    #[test]
    fn every_dialog_string_has_a_bundle_key() {
        for k in [
            "title",
            "description",
            "contains",
            "excludes",
            "understood",
            "passphrase",
            "confirmPassphrase",
            "mismatch",
            "export",
            "saved",
            "sharePassphrase",
        ] {
            assert!(
                crate::i18n::has(&format!("review:logs.replayBundle.{k}")),
                "{k}"
            );
        }
        assert!(crate::i18n::has("review:logs.exportMenu.replayBundle"));
    }
}
