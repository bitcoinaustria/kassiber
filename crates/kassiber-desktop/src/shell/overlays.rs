//! Shell overlays: the lock screen, toasts, the book-refresh sync card, the route progress
//! line and the docked side panel.

use rune::{Color, El, LoopSpec, TextEdit, TextStyle, col, el, row, text};
use rune_kit::TextInput;
use serde_json::Value;

use super::glass;
use crate::app::{App, ToastTone};
use crate::i18n::t;
use crate::icons as I;
use crate::theme::Tk;
use crate::txdetail::widgets::{field, form_kit};
use crate::ui::{ic, outline_button};

/// The lock screen's state (LockScreen.tsx); reset by every lock and unlock.
pub struct LockUi {
    pub passphrase: TextEdit,
    pub revealed: bool,
    pub error: Option<String>,
    pub submitting: bool,
    /// Unlock attempts on this screen.
    pub attempt: u32,
    /// A book whose opening asked for its passphrase (`pendingProjectUnlock`): the unlock opens
    /// it with `ui.projects.select`.
    pub pending_project: Option<Value>,
    /// A Touch ID unlock runs (the macOS sheet, then the daemon): `touchIdSubmitting`.
    pub touch_submitting: bool,
    /// "Use Touch ID next time" as the user set it (None: its default).
    pub enroll_touch_id: Option<bool>,
}

impl Default for LockUi {
    fn default() -> Self {
        let mut passphrase = TextEdit::new("");
        passphrase.focused = true;
        LockUi {
            passphrase,
            revealed: false,
            error: None,
            submitting: false,
            attempt: 0,
            pending_project: None,
            touch_submitting: false,
            enroll_touch_id: None,
        }
    }
}

/// Lock Kassiber (`lockApp`): `daemon.lock` closes the database handle, every cached read and
/// screen goes, and the lock screen covers the shell. The user menu, ⌘L, the search palette,
/// the idle timer and "lock when the window closes" all land here.
pub fn lock(app: &mut App) {
    if app.locked {
        return;
    }
    app.lock_session(true);
}

/// The lock screen's submit: the passphrase (or, for a plaintext book, nothing) to the daemon.
/// When the unlock should also save it for Touch ID, the passphrase is kept until it succeeds.
pub fn unlock(app: &mut App) {
    if app.lock.touch_submitting {
        return;
    }
    crate::secrets::before_passphrase_unlock(app);
    app.submit_unlock();
}

fn ink3(tk: &Tk) -> Color {
    rune::color::hex(if tk.dark { "#a1a1aa" } else { "#71717a" }).expect("hex")
}

/// "Open other books" (`resetLocalUiSession`): forget this session and go to `/`, where the
/// Welcome screen picks a book; the books themselves are untouched.
pub fn open_other_books(app: &mut App) {
    if app.lock.submitting {
        return;
    }
    crate::onboarding::reset_local_session(app);
}

fn ink2(tk: &Tk) -> Color {
    rune::color::hex(if tk.dark { "#c7c7c9" } else { "#3f3f46" }).expect("hex")
}

/// The lock screen's split layout widths (inner, form column, artwork) when the window holds
/// the chain beside the form; None: the centred stack.
fn lock_split(app: &App, tk: &Tk) -> Option<(f32, f32, f32)> {
    let panel_w = app.shell.window_w - 2.0 * tk.r(0.375) - 2.0;
    let inner_w = panel_w.min(tk.r(80.0)) - 2.0 * tk.r(2.5);
    let gap = tk.r(3.0);
    let left_w = tk.r(24.0).max((inner_w - gap) * 0.38);
    let art_w = inner_w - left_w - gap;
    (art_w >= tk.r(22.0)).then_some((inner_w, left_w, art_w))
}

/// The lock screen (LockScreen.tsx's `LockedFrame` + form) inside the page panel, which the
/// shell already draws full width with the sidebar gone: the wordmark, the title and lead, the
/// passphrase field (or "Open books" for a plaintext book), the error line, and the way out.
pub fn lock_screen(app: &App, tk: &Tk) -> El {
    /// `WORDMARK_COLUMNS * MARK_CELL`: the form is exactly the wordmark's width.
    const MARK_CELL: f32 = 4.0;
    const MARK_WIDTH: f32 = 80.0 * MARK_CELL;
    let required = app.passphrase_required();
    let l = &app.lock;
    let busy = l.submitting || l.touch_submitting;
    let reason = match app.lock_reason() {
        crate::app::LockReason::Encrypted => "encrypted",
        crate::app::LockReason::DaemonAuth => "daemon-auth",
        crate::app::LockReason::Session => "session",
    };
    let title = t(if required {
        "chrome:lock.title"
    } else {
        "chrome:lock.sessionTitle"
    });
    let unlock_label = t(if l.submitting {
        "chrome:lock.unlocking"
    } else if required {
        "chrome:lock.unlock"
    } else {
        "chrome:lock.openBooks"
    });

    let mut form = col().key("sh-lock-form").w(MARK_WIDTH).gap(tk.r(0.75));
    if required {
        let show = t("common:passphraseReveal.show");
        let reveal = row()
            .key("sh-lock-reveal")
            .size(tk.r(2.0), tk.r(2.0))
            .center()
            .radius(tk.r(0.375))
            .hover(tk.accent)
            .role("button")
            .label(show.clone())
            .help(t(if l.revealed {
                "common:passphraseReveal.hide"
            } else {
                "common:passphraseReveal.show"
            }))
            .checked(l.revealed)
            .focusable(!busy)
            .disabled(busy)
            .on_press(|a: &mut App| a.lock.revealed = !a.lock.revealed)
            .child(ic(
                if l.revealed { &I::EYE_OFF } else { &I::EYE },
                tk.r(1.0),
                tk.muted_fg,
            ));
        let mut submit = row()
            .key("sh-lock-submit")
            .size(tk.r(2.0), tk.r(2.0))
            .center()
            .radius(tk.r(0.375))
            .fill(tk.primary)
            .hover(tk.primary_fg.alpha(0.1))
            .role("button")
            .label(unlock_label.clone())
            .help(unlock_label.clone())
            .focusable(!busy)
            .disabled(busy)
            .on_press(unlock);
        submit = if l.submitting {
            submit.opacity(0.5).child(
                ic(&I::LOADER_CIRCLE, tk.r(1.0), tk.primary_fg).looping(LoopSpec::rotation(1.0)),
            )
        } else if busy {
            submit
                .opacity(0.5)
                .child(ic(&I::ARROW_RIGHT, tk.r(1.0), tk.primary_fg))
        } else {
            submit.child(ic(&I::ARROW_RIGHT, tk.r(1.0), tk.primary_fg))
        };
        let trailing = row()
            .gap(tk.r(0.125))
            .center_y()
            .pr(tk.r(0.375))
            .child(reveal)
            .child(submit);
        let mut k = form_kit(tk);
        k.c.text_faint = tk.muted_fg;
        let mut input = TextInput::new(&l.passphrase)
            .label(t("chrome:lock.passphrase"))
            .placeholder(t("chrome:lock.passphrase"))
            .height(tk.r(2.75))
            .width(None)
            .text_style(tk.tsm(400.0))
            .leading_icon(&I::LOCK_KEYHOLE)
            .leading_icon_size(tk.r(1.0))
            .secure()
            .revealed(l.revealed)
            .invalid(l.error.is_some())
            .disabled(busy)
            .trailing(trailing)
            .on_edit(|a: &mut App| &mut a.lock.passphrase)
            .on_change(|a: &mut App, _tx: &mut rune::Tx| a.lock.error = None)
            .on_submit(|a: &mut App, _tx: &mut rune::Tx| unlock(a))
            .build(&k)
            .key("sh-lock-passphrase");
        if busy {
            input = input.opacity(0.5);
        }
        form = form.child(field(tk, input));
    } else {
        let mut open = row()
            .key("sh-lock-open")
            .w_full()
            .h(tk.r(2.5))
            .gap(tk.r(0.5))
            .center()
            .radius(tk.r(0.375))
            .fill(tk.primary)
            .hover(tk.primary_fg.alpha(0.1))
            .role("button")
            .label(unlock_label.clone())
            .focusable(!busy)
            .disabled(busy)
            .default_button()
            .autofocus()
            .on_press(unlock);
        if l.submitting {
            open = open.opacity(0.5).child(
                ic(&I::LOADER_CIRCLE, tk.r(1.0), tk.primary_fg).looping(LoopSpec::rotation(1.0)),
            );
        }
        open = open.child(text(unlock_label).font(tk.tsm(500.0)).color(tk.primary_fg));
        if !l.submitting {
            open = open.child(ic(&I::ARROW_RIGHT, tk.r(1.0), tk.primary_fg));
        }
        form = form.child(open);
    }
    // A line stays reserved for the error, so a refusal does not jolt the form.
    let err_line = row().key("sh-lock-error").w_full().min_h(tk.r(1.25));
    // Under the field it belongs to: left in the split layout, centred in the stack.
    let split_at = lock_split(app, tk);
    let mut err_line = if split_at.is_some() {
        err_line
    } else {
        err_line.justify_center()
    };
    if let Some(e) = &l.error {
        err_line = err_line.child(
            row()
                .gap(tk.r(0.375))
                .role("alert")
                .label(e.clone())
                .child(ic(&I::CIRCLE_ALERT, tk.r(0.875), tk.destructive).mt(tk.r(0.125)))
                .child(
                    text(e.clone())
                        .font(tk.txs(400.0).lh(tk.r(1.25)))
                        .color(tk.destructive)
                        .wrap()
                        .max_w(MARK_WIDTH - tk.r(1.5)),
                ),
        );
    }
    form = form.child(err_line);
    form = touch_id_controls(app, tk, form, busy);
    // While Touch ID runs, its waiting line stands in for the form.
    let form = if l.touch_submitting {
        col().key("sh-lock-form").w(MARK_WIDTH).child(
            row()
                .key("sh-lock-touch-wait")
                .w_full()
                .min_h(tk.r(2.75))
                .gap(tk.r(0.5))
                .center()
                .role("status")
                .label(t("chrome:lock.waitingForTouchId"))
                .child(ic(&I::FINGERPRINT, tk.r(1.25), ink2(tk)))
                .child(
                    text(t("chrome:lock.waitingForTouchId"))
                        .font(tk.tsm(400.0))
                        .color(ink2(tk)),
                ),
        )
    } else {
        form
    };

    let lead_key = if l.touch_submitting {
        "chrome:lock.touchIdLead".to_owned()
    } else {
        format!("chrome:lock.lead.{reason}")
    };
    // The split layout's header: the same lines, left-aligned beside the artwork.
    let header_left = col()
        .w_full()
        .gap(tk.r(0.75))
        .child(
            text(title.clone())
                .font(
                    TextStyle::new(tk.r(2.25), 600.0)
                        .lh(tk.r(2.5))
                        .tracking(-tk.r(2.25) * 0.025),
                )
                .color(tk.fg)
                .role("heading")
                .level(1)
                .wrap(),
        )
        .child(
            text(t(&lead_key))
                .font(tk.tsm(400.0).lh(tk.r(1.4219)))
                .color(ink2(tk))
                .wrap(),
        );
    let header = col()
        .items_center()
        .gap(tk.r(0.75))
        .child(
            text(title)
                .font(
                    TextStyle::new(tk.r(1.5), 600.0)
                        .lh(tk.r(1.875))
                        .tracking(-tk.r(1.5) * 0.025),
                )
                .color(tk.fg)
                .role("heading")
                .level(1)
                .wrap()
                .align(rune::Align::Center),
        )
        .child(
            text(t(&lead_key))
                .font(tk.tsm(400.0).lh(tk.r(1.4219)))
                .color(ink2(tk))
                .wrap()
                .align(rune::Align::Center),
        );
    let mark = {
        // The pre-alpha chip sits by the wordmark here too (an owner request).
        let mark =
            super::wordmark::animated(app, super::wordmark::Site::Lock, MARK_CELL, tk.fg, false);
        if app.prefs.alpha_visible {
            let tag = super::titlebar::alpha_block(tk, MARK_CELL / 2.0)
                .label(t("chrome:preAlpha.label"))
                .role("img");
            row()
                .items_end()
                .gap(MARK_CELL * 2.0)
                .child(mark)
                .child(tag.mb(MARK_CELL * 2.0))
        } else {
            mark
        }
    };
    let art_busy = l.submitting || l.touch_submitting;
    let stack = |mark: El, art_el: Option<El>| {
        col()
            .key("sh-lock-center")
            .w_full()
            .max_w(tk.r(28.0))
            .items_center()
            .gap(tk.r(2.5))
            .py(tk.r(3.0))
            .child(mark)
            .children(art_el)
    };
    // The split layout, as the start page's: the form on the left, the encrypted
    // chain filling the right, a ruled footer. Narrow windows keep the centred stack.
    let pad = tk.r(2.5);
    let gap = tk.r(3.0);
    let (inner_w, left_w, art_w) = split_at.unwrap_or_default();
    let (center, body) = match split_at {
        Some(_) => {
            let mode = crate::art::calm::Mode::Locked { busy: art_busy };
            let left = col()
                .key("sh-lock-center")
                .w(left_w)
                .items_start()
                .gap(tk.r(2.0))
                .child(mark)
                .child(header_left)
                .child(form);
            let art_h = tk.r(30.0).min(app.shell.window_h * 0.6);
            let art = crate::art::calm::draw(tk, app.clock, art_w, art_h, mode);
            (
                None,
                Some(row().w(inner_w).gap(gap).center_y().child(left).child(art)),
            )
        }
        None => (Some(stack(mark, None).child(header).child(form)), None),
    };
    let footer_label = t("chrome:lock.openOtherBooks");
    let footer = row()
        .key("sh-lock-other")
        .gap(tk.r(0.375))
        .px(tk.r(0.25))
        .py(tk.r(0.125))
        .center_y()
        .radius(tk.r(0.375))
        .role("button")
        .label(footer_label.clone())
        .focusable(!l.submitting)
        .disabled(l.submitting)
        .on_press(open_other_books)
        .child(ic(&I::ARROW_LEFT, tk.r(0.875), ink2(tk)))
        .child(text(footer_label).font(tk.tsm(500.0)).color(ink2(tk)));
    // With developer tools on, the demo book opens from here too (a separate local copy; Leave
    // demo comes back to this lock screen's books).
    let demo = app.prefs.developer_tools.then(|| {
        let busy = app.demo.busy() || l.submitting;
        let label = t(if app.demo.busy() {
            "onboarding:start.tryDemo.titleOpening"
        } else {
            "onboarding:start.tryDemo.title"
        });
        let mut d = row()
            .key("sh-lock-demo")
            .gap(tk.r(0.375))
            .px(tk.r(0.25))
            .py(tk.r(0.125))
            .center_y()
            .radius(tk.r(0.375))
            .role("button")
            .label(label.clone())
            .help(t("onboarding:start.tryDemo.tooltip"))
            .focusable(!busy)
            .disabled(busy)
            .child(ic(&I::FLASK_CONICAL, tk.r(0.875), ink2(tk)))
            .child(text(label).font(tk.tsm(500.0)).color(ink2(tk)));
        if busy {
            d = d.opacity(0.5);
        } else {
            d = d.on_press(crate::demo::start);
        }
        d
    });
    // `m-auto`: centred in the panel, scrolling to the wordmark in a short window.
    let panel_h = (app.shell.window_h - 40.0 - tk.r(0.375) - 2.0).max(tk.r(30.0));
    let footer = if l.submitting {
        footer.opacity(0.5)
    } else {
        footer
    };
    if let Some(body) = body {
        return col()
            .key("sh-lock-screen")
            .w_full()
            .h(panel_h)
            .px(pad)
            .items_center()
            .child(el().grow(1.0))
            .child(body)
            .child(el().grow(1.0))
            .child(
                row()
                    .w(inner_w)
                    .center_y()
                    .justify_between()
                    .pt(tk.r(1.5))
                    .mb(tk.r(2.0))
                    .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
                    .border_layout()
                    .child(footer)
                    .children(demo),
            );
    }
    let center = center.unwrap_or_else(el);
    col()
        .key("sh-lock-screen")
        .w_full()
        .h(panel_h)
        .px(tk.r(1.5))
        .items_center()
        .child(el().grow(1.0))
        .child(center)
        .child(el().grow(1.0))
        .child(
            row()
                .w_full()
                .justify_center()
                .pb(tk.r(1.5))
                .gap(tk.r(1.5))
                .child(footer)
                .children(demo),
        )
}

/// `ImportRootRestoreScreen` (LockScreen.tsx): the imported book this launch should open could
/// not be opened. `LockedFrame` with the wordmark, "Opening local books", the error, and "Back to
/// setup" (`resetLocalUiSession`). Opening itself runs before the first frame here, so the web's
/// indeterminate "opening" bar has no frame to show in.
pub fn import_root_screen(app: &App, tk: &Tk) -> El {
    const MARK_CELL: f32 = 4.0;
    const MARK_WIDTH: f32 = 80.0 * MARK_CELL;
    let error = app.onboarding.import_root_error.clone().unwrap_or_default();
    let header = col()
        .items_center()
        .gap(tk.r(0.75))
        .child(
            text(t("chrome:importRoot.title"))
                .font(
                    TextStyle::new(tk.r(1.5), 600.0)
                        .lh(tk.r(1.875))
                        .tracking(-tk.r(1.5) * 0.025),
                )
                .color(tk.fg)
                .role("heading")
                .level(1)
                .wrap()
                .align(rune::Align::Center),
        )
        .child(
            text(t("chrome:importRoot.body"))
                .font(tk.tsm(400.0).lh(tk.r(1.4219)))
                .color(ink2(tk))
                .wrap()
                .align(rune::Align::Center),
        );
    // `LockError`: `flex items-start justify-center gap-1.5 text-xs leading-5 text-destructive`.
    let alert = row()
        .key("sh-import-root-error")
        .w(MARK_WIDTH)
        .gap(tk.r(0.375))
        .justify_center()
        .role("alert")
        .label(error.clone())
        .child(ic(&I::CIRCLE_ALERT, tk.r(0.875), tk.destructive).mt(tk.r(0.125)))
        .child(
            text(error)
                .font(tk.txs(400.0).lh(tk.r(1.25)))
                .color(tk.destructive)
                .wrap()
                .max_w(MARK_WIDTH - tk.r(1.5)),
        );
    let center = col()
        .key("sh-import-root-center")
        .w_full()
        .max_w(tk.r(28.0))
        .items_center()
        .gap(tk.r(2.5))
        .py(tk.r(3.0))
        .child(super::wordmark::animated(
            app,
            super::wordmark::Site::Lock,
            MARK_CELL,
            tk.fg,
            false,
        ))
        .child(header)
        .child(alert);
    let back = t("chrome:importRoot.backToSetup");
    let footer = row()
        .key("sh-import-root-back")
        .gap(tk.r(0.375))
        .px(tk.r(0.25))
        .py(tk.r(0.125))
        .center_y()
        .radius(tk.r(0.375))
        .role("button")
        .label(back.clone())
        .focusable(true)
        .autofocus()
        .on_press(crate::onboarding::reset_local_session)
        .child(ic(&I::ARROW_LEFT, tk.r(0.875), ink2(tk)))
        .child(text(back).font(tk.tsm(500.0)).color(ink2(tk)));
    let panel_h = (app.shell.window_h - 40.0 - tk.r(0.375) - 2.0).max(tk.r(30.0));
    col()
        .key("sh-import-root-screen")
        .w_full()
        .h(panel_h)
        .px(tk.r(1.5))
        .items_center()
        .child(el().grow(1.0))
        .child(center)
        .child(el().grow(1.0))
        .child(row().w_full().justify_center().pb(tk.r(1.5)).child(footer))
}

/// Below the error line (LockScreen.tsx): "Unlock with Touch ID" when a passphrase is saved for
/// it, why Touch ID is not on offer when the user turned it on, and "Use Touch ID next time".
fn touch_id_controls(app: &App, tk: &Tk, mut form: El, busy: bool) -> El {
    if crate::secrets::can_use(app) {
        let label = t("chrome:lock.unlockWithTouchId");
        let mut b = row()
            .key("sh-lock-touch")
            .w_full()
            .h(tk.r(2.5))
            .gap(tk.r(0.5))
            .center()
            .radius(tk.r(0.375))
            .fill(if tk.dark {
                tk.input.alpha(tk.input.a * 0.3)
            } else {
                tk.background
            })
            .border(1.0, tk.input)
            .border_layout()
            .hover(tk.accent)
            .role("button")
            .label(label.clone())
            .focusable(!busy)
            .disabled(busy)
            .on_press(crate::secrets::start_touch_unlock)
            .child(ic(&I::FINGERPRINT, tk.r(1.0), tk.fg))
            .child(text(label).font(tk.tsm(500.0)).color(tk.fg));
        if busy {
            b = b.opacity(0.5);
        }
        form = form.child(b);
    }
    if let Some(note) = crate::secrets::lock_note(app) {
        form = form.child(
            text(note)
                .font(tk.txs(400.0).lh(tk.r(1.25)))
                .color(ink3(tk))
                .wrap()
                .max_w(320.0)
                .key("sh-lock-touch-note"),
        );
    }
    if crate::secrets::can_enroll(app) {
        let title = t("chrome:lock.useTouchIdNextTime");
        let on = crate::secrets::enroll_switch(app);
        let switch = crate::ui::switch(tk, "sh-lock-touch-enroll", on, &title)
            .disabled(app.lock.submitting)
            .focusable(!app.lock.submitting);
        let switch = if app.lock.submitting {
            switch.opacity(0.5)
        } else {
            switch.on_press(move |a: &mut App| a.lock.enroll_touch_id = Some(!on))
        };
        form = form.child(
            row()
                .key("sh-lock-touch-enroll-card")
                .w_full()
                .mt(tk.r(0.5))
                .px(tk.r(1.0))
                .py(tk.r(0.75))
                .gap(tk.r(1.0))
                .center_y()
                .justify_between()
                .radius(tk.r(0.75))
                .fill(tk.card.alpha(tk.card.a * 0.4))
                .border(1.0, tk.border.alpha(tk.border.a * 0.7))
                .border_layout()
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(tk.r(0.25))
                        .child(text(title).font(tk.tsm(500.0)).color(tk.fg).wrap())
                        .child(
                            text(t("chrome:lock.useTouchIdNextTimeBody"))
                                .font(tk.txs(400.0).lh(tk.r(1.125)))
                                .color(ink3(tk))
                                .wrap(),
                        ),
                )
                .child(switch),
        );
    }
    form
}

/// `RouteTopProgressLine`: a 3 px line over the top of the page panel while reads load or a job
/// runs; a percentage fills it, otherwise a third of it slides. Its label is an off-screen live
/// region unless the sync card announces instead.
pub fn route_progress(app: &App, tk: &Tk, announce: bool) -> El {
    let maint = app.shell.maint.as_ref().filter(|m| !m.failed);
    let reduced = app.shell.marks.reduced_motion();
    let mut line = el()
        .key("sh-progress")
        .absolute(0.0, 0.0)
        .w_full()
        .h(3.0)
        .clip_rounded(0.0);
    let fill = match maint.filter(|m| m.determinate()) {
        Some(m) => {
            let pct = m.value.unwrap_or(0.0).clamp(0.0, 100.0) as f32 / 100.0;
            row()
                .key("sh-progress-fill")
                .w_full()
                .h(3.0)
                .child(
                    el().h(3.0)
                        .grow(pct.max(0.0001))
                        .fill(tk.primary.alpha(0.8)),
                )
                .child(el().h(3.0).grow((1.0 - pct).max(0.0001)))
        }
        None if reduced => el().w_full().h(3.0).fill(tk.primary.alpha(0.8)),
        None => {
            let w = tk.r(20.0);
            el().w(w)
                .h(3.0)
                .fill(tk.primary.alpha(0.8))
                .looping(LoopSpec::slide(0.9, [-w * 1.2, 0.0], [w * 6.0, 0.0]))
        }
    };
    line = line.child(fill.a11y_hidden());
    if let (true, Some(m)) = (announce, maint) {
        line = line.child(el().size(1.0, 1.0).role("status").label(m.route_label()));
    }
    line
}

/// FirstSyncCard's header: the working K (an X once the refresh failed) in a `size-10
/// rounded-lg` primary-tinted tile, beside the `text-sm` title and the `text-xs` body (an
/// alert when it carries the failure).
fn sync_card_header(
    app: &App,
    tk: &Tk,
    failed: bool,
    title: String,
    body: String,
    alert: bool,
) -> El {
    let (tint, icon) = if failed {
        (tk.destructive, ic(&I::X, tk.r(1.25), tk.destructive))
    } else {
        (
            tk.primary,
            super::wordmark::working_mark(app, 2.0, tk.primary),
        )
    };
    let tile = row()
        .w(tk.r(2.5))
        .h(tk.r(2.5))
        .center()
        .radius(tk.r(0.5))
        .fill(tint.alpha(tint.a * 0.1))
        .child(icon)
        .a11y_hidden();
    let mut body_el = text(body.clone())
        .font(tk.txs(400.0).lh(tk.r(0.75) * 1.625))
        .color(tk.muted_fg)
        .wrap_anywhere();
    if alert {
        body_el = body_el.role("alert").label(body);
    }
    let words = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(tk.r(0.125))
        .child(
            text(title)
                .font(tk.tsm(600.0))
                .color(tk.fg)
                .role("heading")
                .level(2),
        )
        .child(body_el);
    row()
        .w_full()
        .gap(tk.r(0.75))
        .items_start()
        .child(tile)
        .child(words)
}

/// `splitSyncProgressLabel`: "Wallet: Phase · 200 addresses probed" → the phase part and the
/// numeric detail after the last " · " (only when it has a digit).
pub(crate) fn split_progress_label(label: &str) -> (String, Option<String>) {
    match label.rfind(" · ") {
        Some(i) => {
            let detail = label[i + " · ".len()..].trim();
            if detail.is_empty() || !detail.chars().any(|c| c.is_ascii_digit()) {
                (label.to_owned(), None)
            } else {
                (label[..i].trim().to_owned(), Some(detail.to_owned()))
            }
        }
        None => (label.to_owned(), None),
    }
}

/// The sync card while a book refresh runs or after it failed (FirstSyncCard.tsx), over the
/// page panel only: header, the status line with its percentage, the bar, the phase checklist,
/// and "Continue in background" (or "Close" once it failed).
/// `at`: the page panel's rect when the card goes to the window's overlay layer.
pub fn sync_card(app: &App, tk: &Tk, at: Option<[f32; 4]>) -> El {
    use super::maint::{MILESTONES, active_milestone, phase_label};
    let Some(m) = app.shell.maint.as_ref() else {
        return el();
    };
    let failed = m.failed;
    let reduced = app.shell.marks.reduced_motion();
    // A book's very first sync says so (`isFirstSync`); the title is always the progress's own.
    let first = crate::onboarding::is_first_sync(app);
    let body = if failed {
        if m.body.is_empty() {
            t("chrome:firstSync.bodyFailed")
        } else {
            m.body.clone()
        }
    } else {
        t(if first {
            "chrome:firstSync.body"
        } else {
            "chrome:firstSync.bodyRefresh"
        })
    };
    let determinate = m.determinate();
    let value = m.value.unwrap_or(0.0).clamp(0.0, 100.0);
    let status = if m.label.trim().is_empty() {
        if m.body.is_empty() {
            t("chrome:firstSync.preparing")
        } else {
            m.body.clone()
        }
    } else {
        m.label.trim().to_owned()
    };
    let (main, detail) = split_progress_label(&status);
    let amount = if determinate {
        format!("{}%", value.round())
    } else {
        t("chrome:firstSync.working")
    };
    // Only the concise status line is a live region: "<label> — <pct>%".
    let status_row = row()
        .key("sh-sync-status")
        .w_full()
        .h(14.0)
        .gap(tk.r(0.75))
        .center_y()
        .justify_between()
        .role("status")
        .label(format!("{status} — {amount}"))
        .child(
            text(main)
                .font(tk.txs(500.0))
                .color(tk.primary)
                .truncate()
                .grow(1.0)
                .min_w(0.0)
                .help(status.clone()),
        )
        .child(
            row()
                .gap(tk.r(0.5))
                .center_y()
                .children(detail.map(|d| {
                    text(d)
                        .font(tk.txs(500.0).features("'tnum' 1"))
                        .color(tk.muted_fg)
                        .truncate()
                        .max_w(tk.r(14.0))
                }))
                .child(
                    text(amount)
                        .font(tk.txs(500.0).features("'tnum' 1"))
                        .color(tk.muted_fg),
                ),
        );
    // `h-1.5 rounded-full bg-primary/15`; the fill `bg-primary/80` (`bg-destructive/80` failed).
    let ink = if failed {
        tk.destructive.alpha(tk.destructive.a * 0.8)
    } else {
        tk.primary.alpha(0.8)
    };
    let track = el()
        .w_full()
        .h(tk.r(0.375))
        .radius(tk.r(0.1875))
        .fill(tk.primary.alpha(0.15))
        .clip_rounded(tk.r(0.1875))
        .a11y_hidden();
    let bar = if determinate {
        let pct = value as f32 / 100.0;
        track.child(
            row()
                .key("sh-sync-fill")
                .w_full()
                .h_full()
                .child(
                    el().h_full()
                        .grow(pct.max(0.0001))
                        .radius(tk.r(0.1875))
                        .fill(ink),
                )
                .child(el().h_full().grow((1.0 - pct).max(0.0001))),
        )
    } else if failed || reduced {
        track.child(el().w_full().h_full().radius(tk.r(0.1875)).fill(ink))
    } else {
        let w = tk.r(14.0);
        track.child(
            el().w(w)
                .h_full()
                .radius(tk.r(0.1875))
                .fill(ink)
                .looping(LoopSpec::slide(0.9, [-w, 0.0], [tk.r(42.0), 0.0])),
        )
    };
    let progress = col()
        .w_full()
        .pt(tk.r(1.0))
        .gap(tk.r(0.375))
        .child(status_row)
        .child(bar);
    // The checklist: done phases struck through, the active one spinning, a failed one an X.
    let active = active_milestone(value / 100.0, determinate);
    let failed_index = if failed {
        MILESTONES
            .iter()
            .position(|p| Some(*p) == m.phase.as_deref().map(super::maint::milestone_phase))
            .unwrap_or(active.min(MILESTONES.len() - 1))
    } else {
        usize::MAX
    };
    let mut list = col()
        .key("sh-sync-milestones")
        .w_full()
        .pt(tk.r(1.0))
        .gap(tk.r(0.5))
        .role("list");
    for (i, phase) in MILESTONES.iter().enumerate() {
        let done = determinate && i < if failed { failed_index } else { active };
        let this_failed = failed && i == failed_index;
        let is_active = !failed && i == active;
        let dot = row()
            .size(tk.r(1.0), tk.r(1.0))
            .center()
            .radius(tk.r(0.5))
            .a11y_hidden();
        let dot = if this_failed {
            dot.fill(tk.destructive)
                .child(ic(&I::X, tk.r(0.75), Color::WHITE))
        } else if done {
            dot.fill(tk.primary.alpha(0.8))
                .child(ic(&I::CHECK, tk.r(0.75), tk.background))
        } else if is_active {
            let spin = ic(&I::LOADER_CIRCLE, tk.r(0.875), tk.primary);
            dot.child(if reduced {
                spin
            } else {
                spin.looping(LoopSpec::rotation(1.0))
            })
        } else {
            dot.child(
                el().size(tk.r(0.375), tk.r(0.375))
                    .radius(tk.r(0.1875))
                    .fill(tk.muted_fg.alpha(tk.muted_fg.a * 0.4)),
            )
        };
        let (style, color) = if this_failed {
            (tk.txs(500.0), tk.destructive)
        } else if done {
            (tk.txs(400.0), tk.muted_fg)
        } else if is_active {
            (tk.txs(500.0), tk.fg)
        } else {
            (tk.txs(400.0), tk.muted_fg.alpha(tk.muted_fg.a * 0.7))
        };
        let mut label = text(phase_label(phase)).font(style).color(color).truncate();
        if done {
            let n = phase_label(phase).len();
            label = label.strike(0, n, Some(tk.muted_fg.alpha(tk.muted_fg.a * 0.4)));
        }
        list = list.child(
            row()
                .key(format!("sh-sync-m{i}"))
                .w_full()
                .gap(tk.r(0.625))
                .center_y()
                .role("listitem")
                .child(dot)
                .child(label),
        );
    }
    let close_label = t(if failed {
        "chrome:firstSync.closeFailure"
    } else {
        "chrome:firstSync.continueInBackground"
    });
    // `Button variant="secondary" size="sm" className="rounded-full text-xs"`.
    let close = row()
        .key("sh-sync-close")
        .h(tk.r(2.0))
        .px(tk.r(0.75))
        .center()
        .radius(tk.r(1.0))
        .fill(tk.muted)
        .hover(tk.accent)
        .role("button")
        .label(close_label.clone())
        .focusable(true)
        .on_press(|s: &mut App| {
            // A failure's card goes for good; a running one only steps back to the line.
            if s.shell.maint.as_ref().is_some_and(|m| m.failed) {
                super::maint::clear(s, super::maint::Job::BookRefresh);
            } else {
                s.shell.sync_card_open = false;
            }
        })
        .child(text(close_label).font(tk.txs(500.0)).color(tk.fg));
    let footer = col()
        .w_full()
        .mt(tk.r(1.25))
        .pt(tk.r(1.0))
        .gap(tk.r(0.625))
        .items_center()
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border.alpha(tk.border.a * 0.6))
        .border_layout()
        .child(
            text(t(if failed {
                "chrome:firstSync.failedHint"
            } else {
                "chrome:firstSync.keepUsing"
            }))
            .font(tk.txs(400.0).lh(tk.r(0.75) * 1.375))
            .color(tk.muted_fg)
            .wrap()
            .align(rune::Align::Center),
        )
        .child(close);
    // FirstSyncCard's `kb-glass-dialog` surface (the shared dialog glass).
    let card = super::glass_surface(
        tk,
        col()
            .key("sh-sync-card")
            .w_full()
            .max_w(tk.r(42.0))
            .padding(tk.r(1.25))
            .role("dialog")
            .label(m.title.clone())
            .child(sync_card_header(
                app,
                tk,
                failed,
                m.title.clone(),
                body,
                failed && !m.body.is_empty(),
            ))
            .child(progress)
            .child(list)
            .child(footer),
    );
    // The wash and the card fade in side by side under a wrapper that lands at once, so the
    // card's blur sees the page from the first frame (see `super::layer_root`).
    let frame = col().key("sh-sync-layer");
    let frame = match at {
        Some([x, y, w, h]) => frame.absolute(x, y).size(w, h).clip_rounded(tk.r(1.0)),
        None => frame.absolute(0.0, 0.0).w_full().h_full(),
    };
    super::layer_root(
        frame
            .items_center()
            .pt(tk.r(4.0))
            .px(tk.r(1.0))
            .child(
                el().key("sh-sync-wash")
                    .absolute(0.0, 0.0)
                    .w_full()
                    .h_full()
                    .fill(tk.glass_backdrop)
                    .a11y_hidden(),
            )
            .child(card),
    )
}

pub fn toasts(app: &App, tk: &Tk, layer: &mut Vec<El>) {
    // The region stays mounted while unlocked, as Radix's toast viewport does, so each glass toast
    // is itself what enters and leaves: its own fade carries its blur both ways.
    if app.locked {
        return;
    }
    let mut stack = col()
        .key("sh-toasts")
        .w(tk.r(20.0))
        .gap(tk.r(0.5))
        .role("region")
        .label(t("chrome:feedback.label"));
    let dismiss = t("chrome:feedback.dismiss");
    for toast in &app.toasts {
        let (glyph, color) = match toast.tone {
            ToastTone::Error => (&I::CIRCLE_ALERT, tk.alert.solid),
            ToastTone::Warning => (&I::TRIANGLE_ALERT, tk.warn.solid),
            ToastTone::Success => (&I::CIRCLE_CHECK, tk.good.solid),
            ToastTone::Info => (&I::INFO, tk.muted_fg),
        };
        let id = toast.id;
        let mut texts = col().grow(1.0).min_w(0.0).gap(tk.r(0.125)).child(
            text(toast.title.clone())
                .font(tk.tsm(500.0))
                .color(tk.fg)
                .wrap(),
        );
        if let Some(b) = &toast.body {
            texts = texts.child(
                text(b.clone())
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg)
                    .wrap(),
            );
        }
        // `toastopen`: text actions on one line under the body. The announcement (the status
        // region) stays the title and body alone; the actions follow it.
        let acted = !toast.actions.is_empty();
        if acted {
            let links = crate::toastopen::links(
                tk,
                &toast.actions,
                |a| crate::toastopen::toast_key(a, id),
                tk.muted_fg,
                tk.fg,
            );
            texts = col()
                .grow(1.0)
                .min_w(0.0)
                .gap(tk.r(0.375))
                .child(texts.w_full().role("status"))
                .child(links);
        }
        let mut r = row()
            .key(format!("sh-toast{id}"))
            .w_full()
            .padding(tk.r(0.75))
            .gap(tk.r(0.5))
            .items_start()
            .child(ic(glyph, tk.r(1.0), color))
            .child(texts);
        if !acted {
            r = r.role("status");
        } else {
            r = r.on_hover_change(move |s: &mut App, inside: bool, _tx: &mut rune::Tx| {
                if let Some(t) = s.toasts.iter_mut().find(|t| t.id == id) {
                    t.hovered = inside;
                }
            });
        }
        if toast.repeat > 1 {
            r = r.child(
                row()
                    .px(tk.r(0.25))
                    .radius(tk.r(0.25))
                    .fill(tk.muted)
                    .child(
                        text(format!("×{}", toast.repeat))
                            .font(tk.txs(500.0))
                            .color(tk.muted_fg),
                    ),
            );
        }
        r = r.child(
            row()
                .key(format!("sh-toast-x-{id}"))
                .size(tk.r(1.25), tk.r(1.25))
                .center()
                .radius(tk.r(0.25))
                .hover(tk.accent)
                .role("button")
                .label(dismiss.clone())
                .focusable(true)
                .on_press(move |s: &mut App| s.toasts.retain(|x| x.id != id))
                .child(ic(&I::X, tk.r(0.875), tk.muted_fg)),
        );
        stack = stack.child(glass(tk, r));
    }
    layer.push(stack.pin(None, None, Some(tk.r(1.0)), Some(tk.r(1.0))));
    let _ = outline_button;
}
