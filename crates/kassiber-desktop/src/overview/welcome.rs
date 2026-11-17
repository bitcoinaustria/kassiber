//! `WelcomeSection.tsx`: the readiness pill and its detail, and the Add connection button; then
//! the home screen's Edit button (or, while editing, Add widget, Reset and Done; Rune only).

use rune::{El, row, text};

use crate::app::App;
use crate::i18n::t;
use crate::icons as I;
use crate::model::{self, ReadinessIcon, Snapshot, ToneKind};
use crate::theme::Tk;
use crate::ui::{ic, neutral_tone};

pub fn view(app: &App, tk: &Tk, s: &Snapshot, width: f32) -> El {
    let review = app.badges.as_ref().map(|b| b.total());
    // Blocking sources come from the shell's notification conditions; a failed book refresh
    // is the one this app tracks.
    let blocking = usize::from(app.shell.sync_failed);
    let r = model::readiness(s, blocking);
    let (r_title, r_detail) = super::l10n::readiness(&r, s, review, blocking);
    let tone = match r.tone {
        ToneKind::Good => tk.good,
        ToneKind::Warning => tk.warn,
        ToneKind::Alert => tk.alert,
        ToneKind::Neutral => neutral_tone(tk),
    };
    let glyph = match r.icon {
        ReadinessIcon::Plus => &I::PLUS,
        ReadinessIcon::WalletCards => &I::WALLET_CARDS,
        ReadinessIcon::RefreshCw => &I::REFRESH_CW,
        ReadinessIcon::ShieldAlert => &I::SHIELD_ALERT,
        ReadinessIcon::CheckCircle => &I::CIRCLE_CHECK,
    };
    // `sm` and up: one row; below it the button drops its label and the block stacks.
    let narrow = width < tk.r(40.0);
    let mut left = row()
        .key("readiness-row")
        .gap(tk.r(0.625))
        .center_y()
        .grow(1.0)
        .min_w(0.0);
    if !r.connect_source {
        let busy = r.reprocess && app.busy.processing;
        let title = if busy {
            t("overview:welcome.reprocessingJournals")
        } else {
            r_title.clone()
        };
        let mut glyph_el = ic(glyph, tk.r(1.0), tone.text);
        if busy {
            glyph_el = glyph_el.looping(rune::LoopSpec::rotation(1.0));
        }
        let mut p = row()
            .key("readiness")
            .h(tk.r(1.75))
            .px(tk.r(0.5))
            .gap(tk.r(0.375))
            .center_y()
            .radius(tk.r(0.375))
            .fill(tone.bg)
            .border(1.0, tone.border)
            .child(glyph_el)
            .child(text(title.clone()).font(tk.txs(500.0)).color(tone.text));
        if r.reprocess {
            p = p
                .role("button")
                .label(title)
                .focusable(!busy)
                .hover(tone.bg.alpha(tone.bg.a * 1.8))
                .disabled(busy);
            if !busy {
                p = p.on_press(|s: &mut App| s.process_journals());
            }
        } else if let Some(href) = r.href {
            p = p
                .role("link")
                .label(title)
                .focusable(true)
                .hover(tone.bg.alpha(tone.bg.a * 1.8))
                .on_press(move |s: &mut App| s.navigate(href));
        }
        left = left.child(p);
    }
    left = left.child(
        text(r_detail)
            .font(if narrow { tk.txs(400.0) } else { tk.tsm(400.0) })
            .color(tk.muted_fg)
            .truncate(),
    );

    let mut button = row()
        .key("add-connection")
        .h(tk.r(2.0))
        .px(tk.r(if narrow { 0.5 } else { 0.75 }))
        .gap(tk.r(0.5))
        .center_y()
        .radius(tk.r(0.375))
        .fill(tk.primary)
        .hover(tk.primary.mix(tk.background, 0.1))
        .role("button")
        .label(t("overview:welcome.addConnectionAria"))
        .focusable(true)
        .on_press(|s: &mut App| super::open_add_connection(s))
        .child(ic(&I::PLUS, tk.r(1.0), tk.primary_fg));
    if !narrow {
        button = button.child(
            text(t("overview:welcome.addConnection"))
                .font(tk.tsm(500.0))
                .color(tk.primary_fg),
        );
    }
    row()
        .key("welcome")
        .w_full()
        .gap(tk.r(0.75))
        .center_y()
        .justify_between()
        .child(left)
        .child(super::grid::header(app, tk, narrow, button))
}
