//! Stack goal (Rune only, an owner-requested widget): progress to a stack size the owner sets,
//! what is still to go, and the pace of the last twelve months.
//!
//! The target lives with the home screen's layout (`overviewLayout.stackGoal.sats` in the
//! desktop prefs; `grid::model::Layout`), validated on read and on save: a whole number of sats
//! from 1 sat to 21 M BTC. The stack and the pace come from the overview snapshot already on
//! screen (`ui.overview.snapshot`: `balanceSummary.totalBtc` for the stack, the daily
//! `portfolioSeries[].balanceBtc` for the pace: the balance now less the balance a year ago,
//! i.e. the net acquisitions of the last twelve months). No other read, nothing on the network.

use rune::{Color, El, LineCap, Shape, TextEdit, Tx, canvas, col, el, row, text};

use super::age::{self, years_before};
use super::grid::frame::{self, Slot};
use super::grid::model::{GOAL_MAX_SATS, WidgetId};
use super::holdings_model as hm;
use super::stats::mask;
use crate::app::App;
use crate::format;
use crate::i18n::{Lang, lang, t};
use crate::icons as I;
use crate::model::Snapshot;
use crate::settings::controls;
use crate::theme::Tk;
use crate::ui::{ic, sensitive};

pub const SATS: f64 = 100_000_000.0;

/// Why a typed target is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GoalError {
    Empty,
    NotANumber,
    TooPrecise,
    Zero,
    TooLarge,
}

/// Reads a target in BTC: `1`, `0.5`, `1,5`, `₿ 2`, `0.21 BTC`. At most eight decimals, more
/// than zero, at most 21 M BTC. Gives sats.
pub fn parse_goal(raw: &str) -> Result<u64, GoalError> {
    let mut s: String = raw.trim().to_owned();
    for p in ["₿", "BTC", "btc", "Btc"] {
        s = s
            .trim_start_matches(p)
            .trim_end_matches(p)
            .trim()
            .to_owned();
    }
    let s: String = s
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '_' && *c != '\u{2009}' && *c != '\u{202f}')
        .collect();
    if s.is_empty() {
        return Err(GoalError::Empty);
    }
    let s = match (s.contains('.'), s.contains(',')) {
        (true, true) => return Err(GoalError::NotANumber),
        (false, true) => s.replace(',', "."),
        _ => s,
    };
    let (int, frac) = s.split_once('.').unwrap_or((&s, ""));
    if (int.is_empty() && frac.is_empty())
        || !int.chars().all(|c| c.is_ascii_digit())
        || !frac.chars().all(|c| c.is_ascii_digit())
    {
        return Err(GoalError::NotANumber);
    }
    if frac.len() > 8 {
        return Err(GoalError::TooPrecise);
    }
    let whole: u64 = if int.is_empty() {
        0
    } else {
        int.trim_start_matches('0').parse::<u64>().or_else(|_| {
            if int.chars().all(|c| c == '0') {
                Ok(0)
            } else {
                Err(GoalError::TooLarge)
            }
        })?
    };
    let frac_sats: u64 = format!("{frac:0<8}")
        .parse()
        .map_err(|_| GoalError::NotANumber)?;
    let sats = whole
        .checked_mul(100_000_000)
        .and_then(|w| w.checked_add(frac_sats))
        .ok_or(GoalError::TooLarge)?;
    match sats {
        0 => Err(GoalError::Zero),
        s if s > GOAL_MAX_SATS => Err(GoalError::TooLarge),
        s => Ok(s),
    }
}

/// A target as the field shows it: `1`, `0.5`, `0.00021`.
pub fn goal_text(sats: u64) -> String {
    let whole = sats / 100_000_000;
    let frac = sats % 100_000_000;
    if frac == 0 {
        whole.to_string()
    } else {
        format!("{whole}.{}", format!("{frac:08}").trim_end_matches('0'))
    }
}

/// A round target just above the stack: what the sheet offers and a gallery preview shows.
pub fn suggest(balance_btc: f64) -> u64 {
    const STEPS: [f64; 16] = [
        0.01, 0.05, 0.1, 0.21, 0.5, 1.0, 2.1, 5.0, 10.0, 21.0, 50.0, 100.0, 210.0, 500.0, 1000.0,
        2100.0,
    ];
    let b = balance_btc.max(0.0);
    let btc = STEPS
        .into_iter()
        .find(|s| *s > b + 1e-9)
        .unwrap_or_else(|| ((b / 1000.0).floor() + 1.0) * 1000.0);
    ((btc * SATS).round() as u64).clamp(1, GOAL_MAX_SATS)
}

/// The net acquisitions of the last twelve months: the balance today less the balance a year ago.
pub fn pace(points: &[(i64, f64)], today: format::Day) -> f64 {
    let at = |day: i64| {
        points
            .iter()
            .take_while(|(d, _)| *d <= day)
            .last()
            .map(|(_, b)| *b)
            .unwrap_or(0.0)
    };
    at(today.epoch_days()) - at(years_before(today, 1).epoch_days())
}

/// Whole months to the goal at a yearly pace; `None` when the pace does not grow the stack.
pub fn months_to_go(remaining_btc: f64, pace_per_year: f64) -> Option<u32> {
    if remaining_btc <= 0.0 {
        return Some(0);
    }
    if pace_per_year <= 0.0 {
        return None;
    }
    let m = (remaining_btc / (pace_per_year / 12.0)).ceil();
    (m.is_finite() && m < 12_000.0).then_some(m.max(1.0) as u32)
}

// ---- Copy (the web has no goal; English and German here) ---------------------------------------

fn l(en: &str, de: &str) -> String {
    match lang() {
        Lang::De => de.to_owned(),
        Lang::En => en.to_owned(),
    }
}

pub fn title() -> String {
    l("Stack goal", "Stapelziel")
}
pub fn description() -> String {
    l(
        "Progress to a stack size you set, and the pace to get there.",
        "Fortschritt zu einem selbst gesetzten Bestandsziel und das Tempo dorthin.",
    )
}
pub fn edit_goal() -> String {
    l("Edit goal…", "Ziel bearbeiten …")
}
pub fn set_goal() -> String {
    l("Set goal…", "Ziel festlegen …")
}
fn no_goal() -> String {
    l(
        "Set a target for your stack.",
        "Lege ein Ziel für deinen Bestand fest.",
    )
}
fn of_goal(goal: &str) -> String {
    match lang() {
        Lang::De => format!("von {goal}"),
        Lang::En => format!("of {goal}"),
    }
}
fn to_go(amount: &str) -> String {
    match lang() {
        Lang::De => format!("noch {amount}"),
        Lang::En => format!("{amount} to go"),
    }
}
fn reached() -> String {
    l("Goal reached", "Ziel erreicht")
}
fn pace_line(amount: &str) -> String {
    match lang() {
        Lang::De => format!("{amount} in den letzten 12 Monaten"),
        Lang::En => format!("{amount} in the last 12 months"),
    }
}
fn eta(months: u32) -> String {
    let (y, m) = (months / 12, months % 12);
    match lang() {
        Lang::De => match (y, m) {
            (0, 1) => "≈ 1 Monat in diesem Tempo".into(),
            (0, m) => format!("≈ {m} Monate in diesem Tempo"),
            (1, 0) => "≈ 1 Jahr in diesem Tempo".into(),
            (y, 0) => format!("≈ {y} Jahre in diesem Tempo"),
            (y, m) => format!("≈ {y} J. {m} M. in diesem Tempo"),
        },
        Lang::En => match (y, m) {
            (0, 1) => "≈ 1 month at this pace".into(),
            (0, m) => format!("≈ {m} months at this pace"),
            (1, 0) => "≈ 1 year at this pace".into(),
            (y, 0) => format!("≈ {y} years at this pace"),
            (y, m) => format!("≈ {y} yr {m} mo at this pace"),
        },
    }
}
fn no_pace() -> String {
    l(
        "No net acquisitions in the last 12 months",
        "Keine Nettozukäufe in den letzten 12 Monaten",
    )
}
fn progress_aria(p: &str) -> String {
    match lang() {
        Lang::De => format!("Fortschritt {p}"),
        Lang::En => format!("Progress {p}"),
    }
}
fn sheet_hint() -> String {
    l(
        "The stack size to aim for, in bitcoin. Saved on this device.",
        "Die angestrebte Bestandsgröße in Bitcoin. Wird auf diesem Gerät gespeichert.",
    )
}
fn field_label() -> String {
    l("Target (BTC)", "Ziel (BTC)")
}
fn remove_goal() -> String {
    l("Remove goal", "Ziel entfernen")
}
pub fn error_text(e: GoalError) -> String {
    match e {
        GoalError::Empty => l("Enter a target.", "Gib ein Ziel ein."),
        GoalError::NotANumber => l(
            "Enter an amount like 1 or 0.5.",
            "Gib einen Betrag wie 1 oder 0,5 ein.",
        ),
        GoalError::TooPrecise => l(
            "Use at most eight decimals (1 sat).",
            "Höchstens acht Nachkommastellen (1 Sat).",
        ),
        GoalError::Zero => l(
            "The target must be more than zero.",
            "Das Ziel muss größer als null sein.",
        ),
        GoalError::TooLarge => l(
            "The target cannot exceed 21 million BTC.",
            "Das Ziel kann 21 Millionen BTC nicht übersteigen.",
        ),
    }
}
fn saved() -> String {
    l("Stack goal saved.", "Stapelziel gespeichert.")
}
fn removed() -> String {
    l("Stack goal removed.", "Stapelziel entfernt.")
}

/// An amount still to go: sats below 0.1 BTC (`12,345 sats`), bitcoin above.
fn amount_left(btc: f64) -> String {
    if btc < 0.1 {
        let sats = (btc * SATS).round().max(0.0) as usize;
        format!("{} sats", format::count(sats))
    } else {
        format::btc(btc, 3, false)
    }
}

/// A target as a figure: `₿ 1`, `₿ 0.5`.
fn goal_figure(sats: u64) -> String {
    format!("₿ {}", goal_text(sats))
}

fn pct(p: f64) -> String {
    let v = p * 100.0;
    if v >= 100.0 {
        percent(100.0, 0)
    } else if v <= 0.0 {
        percent(0.0, 0)
    } else if v > 0.0 && v < 0.1 {
        format!("<{}", percent(0.1, 1))
    } else if v < 10.0 {
        percent(v, 1)
    } else {
        percent(v.floor(), 0)
    }
}

/// A percentage as the UI language writes it (`Intl.NumberFormat(…, {style: "percent"})`):
/// en `2.5%`, de-AT `2,5 %` (decimal comma, a no-break space before the sign).
pub(super) fn percent(v: f64, decimals: usize) -> String {
    let n = format!("{v:.decimals$}");
    match crate::i18n::lang() {
        crate::i18n::Lang::De => format!("{}\u{a0}%", n.replace('.', ",")),
        crate::i18n::Lang::En => format!("{n}%"),
    }
}

// ---- The widget ----------------------------------------------------------------------------------

/// A progress ring, `d` across: the track, the arc from twelve o'clock (`None`: no arc, the
/// proportion hidden).
fn ring(tk: &Tk, d: f32, progress: Option<f64>, ink: Color) -> El {
    let w = (d * 0.11).clamp(4.0, 9.0);
    let r = (d - w) / 2.0;
    let c = d / 2.0;
    let arc = |from: f32, to: f32| -> Vec<[f32; 2]> {
        let n = ((to - from).abs() / std::f32::consts::TAU * 96.0)
            .ceil()
            .max(2.0) as usize;
        (0..=n)
            .map(|i| from + (to - from) * i as f32 / n as f32)
            .map(|a| [c + r * a.cos(), c + r * a.sin()])
            .collect()
    };
    let top = -std::f32::consts::FRAC_PI_2;
    let mut shape = Shape::new().stroke_with(
        arc(top, top + std::f32::consts::TAU),
        true,
        w,
        LineCap::Round,
        tk.fg.alpha(if tk.dark { 0.12 } else { 0.09 }),
    );
    if let Some(p) = progress.filter(|p| *p > 0.0) {
        shape = shape.stroke_with(
            arc(top, top + std::f32::consts::TAU * p.min(1.0) as f32),
            false,
            w,
            LineCap::Round,
            ink,
        );
    }
    canvas(shape).size(d, d).inert().a11y_hidden()
}

fn orange(tk: &Tk) -> Color {
    rune::color::hex("#f7931a").unwrap_or(tk.fg)
}

/// The content height at `size`: a ring big enough to read with its two lines (Small), or the
/// ring beside the stack, what is left and the pace (Medium).
pub fn natural(tk: &Tk, size: super::grid::model::Size) -> f32 {
    if size == super::grid::model::Size::Small {
        return tk.r(5.0) + tk.r(0.375) + tk.txs(600.0).size * 1.5 + tk.t2xs(500.0).size * 1.5;
    }
    frame::label(tk).size * 1.5
        + tk.r(0.125)
        + tk.r(1.75) * 1.15
        + tk.r(0.75)
        + tk.txs(600.0).size * 1.5
        + tk.r(0.25)
        + tk.t2xs(400.0).size * 1.5
}

/// The widget's content for its slot. `goal` is the owner's target; a gallery preview without
/// one shows a round target above the stack.
pub fn body(app: &App, tk: &Tk, s: &Snapshot, x: &hm::Extras, slot: &Slot) -> El {
    let hide = app.prefs.hide_sensitive();
    let hide_shares = app.prefs.hide_shares();
    let stack = hm::display_balance_btc(s, x).max(0.0);
    let goal = app
        .ov
        .layout
        .goal_sats()
        .or_else(|| slot.pv.then(|| suggest(stack)));
    let Some(goal) = goal else {
        let mut b =
            crate::ui::outline_button(tk, &slot.key("goal-set"), Some(&I::TARGET), &set_goal());
        if !slot.pv {
            b = super::grid::press(b, WidgetId::Goal, slot.pv, open_sheet);
        }
        return col()
            .key(slot.key("goal-empty"))
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .center()
            .gap(tk.r(0.625))
            .child(
                text(no_goal())
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg)
                    .align(rune::Align::Center)
                    .wrap(),
            )
            .child(b);
    };
    let goal_btc = goal as f64 / SATS;
    let progress = (stack / goal_btc).clamp(0.0, 1.0);
    let done = stack + 1e-9 >= goal_btc;
    let remaining = (goal_btc - stack).max(0.0);
    let ink = if done { tk.good.solid } else { orange(tk) };
    let pct_text = mask(pct(progress), hide_shares);
    let shown_progress = (!hide_shares).then_some(progress);
    let pace = pace(&age::points(s), age::today(app));
    let left_text = if done {
        reached()
    } else {
        to_go(&amount_left(remaining))
    };

    if !slot.wide() {
        // Small: the ring with the percent inside, the target and what is left under it.
        let d = (slot.h - tk.r(2.5)).min(slot.w * 0.62).max(tk.r(3.0));
        let ring_el = el()
            .size(d, d)
            .role("img")
            .label(progress_aria(&pct_text))
            .child(ring(tk, d, shown_progress, ink).absolute(0.0, 0.0))
            .child(col().absolute(0.0, 0.0).size(d, d).center().child(if done {
                ic(&I::CHECK, d * 0.3, ink)
            } else {
                sensitive(
                    pct_text.clone(),
                    frame::figure(tk, (d / tk.r(1.0) * 0.24).clamp(1.0, 1.75)),
                    tk.fg,
                    hide_shares,
                )
                .a11y_hidden()
            }));
        return col()
            .w_full()
            .h_full()
            .min_h(0.0)
            .gap(tk.r(0.375))
            .items_center()
            .justify_between()
            .child(ring_el)
            .child(
                col()
                    .w_full()
                    .items_center()
                    .min_w(0.0)
                    .child(
                        sensitive(
                            mask(left_text, hide && !done),
                            tk.txs(600.0).features("'tnum' 1"),
                            if done { tk.good.solid } else { tk.fg },
                            hide && !done,
                        )
                        .truncate(),
                    )
                    .child(
                        sensitive(
                            mask(of_goal(&goal_figure(goal)), hide),
                            tk.t2xs(500.0).features("'tnum' 1"),
                            tk.muted_fg,
                            hide,
                        )
                        .truncate(),
                    ),
            );
    }

    // Medium: the ring beside the stack, the target, the bar of what is left and the pace.
    let d = slot.h.min(slot.w * 0.36).max(tk.r(3.0));
    let ring_el = el()
        .key(slot.key("goal-ring"))
        .size(d, d)
        .role("img")
        .label(progress_aria(&pct_text))
        .child(ring(tk, d, shown_progress, ink).absolute(0.0, 0.0))
        .child(
            col()
                .absolute(0.0, 0.0)
                .size(d, d)
                .center()
                .gap(1.0)
                .child(if done {
                    ic(&I::CHECK, d * 0.26, ink)
                } else {
                    sensitive(
                        pct_text.clone(),
                        frame::figure(tk, (d / tk.r(1.0) * 0.2).clamp(1.0, 1.75)),
                        tk.fg,
                        hide_shares,
                    )
                    .a11y_hidden()
                }),
        );
    let pace_text = if pace > 0.0 {
        let mut s = pace_line(&mask(format::btc(pace, 3, true), hide));
        if !done
            && let Some(m) = months_to_go(remaining, pace)
            && !hide_shares
        {
            s = format!("{s} · {}", eta(m));
        }
        s
    } else {
        no_pace()
    };
    // The figures on the left (the owner's pick of four layouts): the stack large with the
    // target beside it, then what is left and the pace at the foot; the ring on the right.
    let figures = row()
        .min_w(0.0)
        .gap(tk.r(0.375))
        .items_end()
        .child(
            sensitive(
                mask(format::btc(stack, 3, false), hide),
                frame::figure(tk, 1.75),
                tk.fg,
                hide,
            )
            .truncate()
            .shrink(),
        )
        .child(
            text(of_goal(&if hide {
                crate::theme::MASK.to_owned()
            } else {
                goal_figure(goal)
            }))
            .font(frame::label(tk))
            .color(tk.muted_fg)
            .single_line()
            .mb(tk.r(0.3125)),
        );
    let foot = col()
        .w_full()
        .min_w(0.0)
        .gap(tk.r(0.125))
        .child(
            sensitive(
                mask(left_text, hide && !done),
                tk.tsm(600.0).features("'tnum' 1"),
                if done { tk.good.solid } else { tk.fg },
                hide && !done,
            )
            .truncate(),
        )
        .child(
            text(pace_text.clone())
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .truncate()
                .help(pace_text),
        );
    let left = col()
        .grow(1.0)
        .w(0.0)
        .min_w(0.0)
        .h_full()
        .justify_between()
        .gap(tk.r(0.5))
        .child(figures)
        .child(foot);
    row()
        .key(slot.key("goal-body"))
        .w_full()
        .h_full()
        .min_h(0.0)
        .gap(tk.r(1.5))
        .center_y()
        .child(left)
        .child(ring_el)
}

// ---- The sheet -------------------------------------------------------------------------------------

/// Opens the goal sheet on the current target (or a round one above the stack).
pub fn open_sheet(a: &mut App) {
    let sats = a.ov.layout.goal_sats().unwrap_or_else(|| {
        let stack = a
            .snapshot()
            .map(|s| hm::display_balance_btc(s, &hm::Extras::parse(a.read("ui.overview.snapshot"))))
            .unwrap_or(0.0);
        suggest(stack)
    });
    let mut e = TextEdit::new(goal_text(sats));
    e.focused = true;
    a.ov.goal_edit = e;
    a.ov.goal_error = None;
    a.ov.goal_open = true;
    a.ov.menu_open = false;
}

fn close(a: &mut App) {
    a.ov.goal_open = false;
    a.ov.goal_error = None;
}

/// Saves the typed target; a refused one stays open with the reason.
pub fn save(a: &mut App, tx: &mut Tx) {
    match parse_goal(&a.ov.goal_edit.text) {
        Ok(sats) => {
            a.ov.layout.set_goal_sats(Some(sats));
            close(a);
            tx.announce(saved(), rune::LiveRegion::Polite);
        }
        Err(e) => {
            a.ov.goal_error = Some(e);
            tx.announce(error_text(e), rune::LiveRegion::Assertive);
        }
    }
}

fn clear(a: &mut App, tx: &mut Tx) {
    a.ov.layout.set_goal_sats(None);
    close(a);
    tx.announce(removed(), rune::LiveRegion::Polite);
}

pub fn mount_sheet(app: &App, tk: &Tk, layer: &mut Vec<El>) {
    if !app.ov.goal_open {
        return;
    }
    let input = controls::input(
        tk,
        &app.ov.goal_edit,
        "goal-input",
        &field_label(),
        "1.0",
        false,
        true,
        tk.r(2.25),
        false,
        |a| &mut a.ov.goal_edit,
    );
    let mut field =
        col()
            .w_full()
            .gap(tk.r(0.375))
            .child(controls::labelled(tk, &field_label(), input));
    // After a refused save, the reason for what the field holds now (gone once it is valid).
    if app.ov.goal_error.is_some()
        && let Err(e) = parse_goal(&app.ov.goal_edit.text)
    {
        field = field.child(controls::error_line(tk, error_text(e)));
    }
    let mut buttons = row().w_full().gap(tk.r(0.5)).center_y();
    if app.ov.layout.goal_sats().is_some() {
        buttons = buttons.child(
            controls::btn_lg(
                tk,
                "goal-remove",
                &remove_goal(),
                controls::Btn::Ghost,
                true,
            )
            .on_press_tx(clear),
        );
    }
    buttons = buttons
        .child(el().grow(1.0))
        .child(
            controls::btn_lg(
                tk,
                "goal-cancel",
                &t("common:actions.cancel"),
                controls::Btn::Outline,
                true,
            )
            .on_press(close),
        )
        .child(
            controls::btn_lg(
                tk,
                "goal-save",
                &t("common:actions.save"),
                controls::Btn::Primary,
                true,
            )
            .on_press_tx(save),
        );
    let content = col()
        .w_full()
        .gap(tk.r(1.0))
        .child(
            col()
                .gap(tk.r(0.375))
                .child(controls::dialog_title(tk, title()))
                .child(
                    text(sheet_hint())
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg)
                        .wrap(),
                ),
        )
        .child(field)
        .child(buttons);
    controls::dialog(
        app,
        tk,
        layer,
        "ov-goal-sheet",
        tk.r(26.0),
        content,
        close,
        true,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentages_follow_the_ui_language() {
        use crate::i18n::{Lang, set_lang};
        set_lang(Lang::De);
        let de = [pct(0.025), pct(0.0004), pct(0.5), pct(1.2), pct(0.0)];
        set_lang(Lang::En);
        let en = [pct(0.025), pct(0.0004), pct(0.5), pct(1.2), pct(0.0)];
        assert_eq!(
            de,
            [
                "2,5\u{a0}%",
                "<0,1\u{a0}%",
                "50\u{a0}%",
                "100\u{a0}%",
                "0\u{a0}%"
            ]
        );
        assert_eq!(en, ["2.5%", "<0.1%", "50%", "100%", "0%"]);
    }

    #[test]
    fn targets_parse_in_bitcoin_with_either_decimal_mark() {
        assert_eq!(parse_goal("1"), Ok(100_000_000));
        assert_eq!(parse_goal("1.0"), Ok(100_000_000));
        assert_eq!(parse_goal(" 0,5 "), Ok(50_000_000));
        assert_eq!(parse_goal("₿ 2"), Ok(200_000_000));
        assert_eq!(parse_goal("0.21 BTC"), Ok(21_000_000));
        assert_eq!(parse_goal(".5"), Ok(50_000_000));
        assert_eq!(parse_goal("0.00000001"), Ok(1));
        assert_eq!(parse_goal("21000000"), Ok(GOAL_MAX_SATS));
        assert_eq!(parse_goal("007"), Ok(700_000_000));
    }

    #[test]
    fn bad_targets_are_refused_with_a_reason() {
        assert_eq!(parse_goal(""), Err(GoalError::Empty));
        assert_eq!(parse_goal("  ₿ "), Err(GoalError::Empty));
        assert_eq!(parse_goal("abc"), Err(GoalError::NotANumber));
        assert_eq!(parse_goal("1.000,5"), Err(GoalError::NotANumber));
        assert_eq!(parse_goal("1.2.3"), Err(GoalError::NotANumber));
        assert_eq!(parse_goal("-1"), Err(GoalError::NotANumber));
        assert_eq!(parse_goal("1e3"), Err(GoalError::NotANumber));
        assert_eq!(parse_goal("."), Err(GoalError::NotANumber));
        assert_eq!(parse_goal("0.000000001"), Err(GoalError::TooPrecise));
        assert_eq!(parse_goal("0"), Err(GoalError::Zero));
        assert_eq!(parse_goal("0.00000000"), Err(GoalError::Zero));
        assert_eq!(parse_goal("21000000.00000001"), Err(GoalError::TooLarge));
        assert_eq!(parse_goal("99999999999999999999"), Err(GoalError::TooLarge));
    }

    #[test]
    fn targets_round_trip_through_the_field_text() {
        for sats in [
            1,
            21_000_000,
            50_000_000,
            100_000_000,
            123_456_789,
            GOAL_MAX_SATS,
        ] {
            assert_eq!(parse_goal(&goal_text(sats)), Ok(sats), "{sats}");
        }
        assert_eq!(goal_text(150_000_000), "1.5");
        assert_eq!(goal_text(1), "0.00000001");
    }

    #[test]
    fn the_goal_persists_with_the_layout_and_is_validated() {
        use super::super::grid::model::Layout;
        let mut l = Layout::default();
        assert!(l.set_goal_sats(Some(100_000_000)));
        assert!(!l.set_goal_sats(Some(0)), "zero refused");
        assert!(
            !l.set_goal_sats(Some(GOAL_MAX_SATS + 1)),
            "past 21 M refused"
        );
        assert_eq!(l.goal_sats(), Some(100_000_000));
        let v = l.to_json();
        assert_eq!(v["version"], 2, "the schema stays version 2");
        assert_eq!(v["stackGoal"]["sats"], 100_000_000);
        assert_eq!(Layout::from_json(&v), l);
        // A stored goal out of range, negative, fractional or of the wrong type is dropped.
        for bad in [
            serde_json::json!(0),
            serde_json::json!(-5),
            serde_json::json!(1.5),
            serde_json::json!("100"),
            serde_json::json!(GOAL_MAX_SATS + 1),
        ] {
            let mut doc = v.clone();
            doc["stackGoal"]["sats"] = bad.clone();
            assert_eq!(Layout::from_json(&doc).goal_sats(), None, "{bad}");
        }
        // Reset to default keeps the goal; clearing it removes the key.
        l.remove(super::super::grid::model::WidgetId::Stats);
        l.reset();
        assert!(l.is_default_arrangement());
        assert_eq!(l.goal_sats(), Some(100_000_000));
        assert!(l.set_goal_sats(None));
        assert!(l.to_json().get("stackGoal").is_none());
    }

    #[test]
    fn pace_is_the_balance_change_over_twelve_months() {
        let d = |s: &str| format::Day::parse(s).unwrap().epoch_days();
        let today = format::Day::parse("2026-11-16").unwrap();
        let pts = vec![
            (d("2025-01-01"), 1.0),
            (d("2025-12-01"), 1.5),
            (d("2026-06-01"), 1.2),
            (d("2026-10-01"), 2.0),
        ];
        assert!((pace(&pts, today) - 1.0).abs() < 1e-12);
        assert_eq!(pace(&[], today), 0.0);
        assert_eq!(months_to_go(0.5, 1.2), Some(5));
        assert_eq!(months_to_go(0.0, 0.0), Some(0));
        assert_eq!(months_to_go(1.0, 0.0), None);
        assert_eq!(months_to_go(1.0, -2.0), None);
        assert_eq!(months_to_go(1e9, 1e-9), None, "beyond a millennium");
    }

    #[test]
    fn suggestions_are_round_and_above_the_stack() {
        assert_eq!(suggest(0.0), 1_000_000);
        assert_eq!(suggest(0.3), 50_000_000);
        assert_eq!(suggest(1.0), 210_000_000);
        assert_eq!(suggest(98.58), 10_000_000_000);
        assert_eq!(suggest(5_000.0), 600_000_000_000);
    }
}
