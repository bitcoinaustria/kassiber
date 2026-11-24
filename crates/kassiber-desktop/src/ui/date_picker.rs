//! Shared date + year picker popover and strict date parsing/formatting.
//!
//! Combines the calendar month grid (`reports::ledger::exit_tax`) with the 4-column year grid
//! (`review::tools`) and a 3×4 month grid so picking historical dates such as a wallet birthday
//! takes a few direct clicks instead of stepping month by month.

use rune::{Color, El, Key, KeyEvent, Shadow, TextStyle, Tx, col, el, row, text};
use rune_kit::Placement;

use crate::app::App;
use crate::i18n::{Lang, lang, t, t_in, tr_in};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::{button, ic};

/// Which grid the popover is currently showing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum View {
    #[default]
    Days,
    Months,
    Years,
}

/// Open state for one host's active date picker popover.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickerState {
    /// The trigger key that opened this picker.
    pub key: String,
    /// Displayed calendar year.
    pub year: i32,
    /// Displayed calendar month (0 = January .. 11 = December).
    pub month: u32,
    /// First year of the 12-year grid shown in [`View::Years`].
    pub year_page_start: i32,
    pub view: View,
}

/// Where a screen keeps its single open [`PickerState`].
#[derive(Clone, Copy)]
pub struct Access {
    pub get: fn(&App) -> Option<&PickerState>,
    pub get_mut: fn(&mut App) -> &mut Option<PickerState>,
}

// ---- Calendar math & parsing -----------------------------------------------------------------

/// Days since 1970-01-01 → (year, month 1–12, day) (Hinnant's `civil_from_days`).
pub fn civil(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((yoe + era * 400 + i64::from(m <= 2)) as i32, m, d)
}

/// (year, month 1–12, day) → days since 1970-01-01.
pub fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = i64::from(y) - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (i64::from(m) + 9) % 12;
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Number of calendar days in `(year, month0)` where `month0` is 0..=11.
pub fn days_in_month(y: i32, m0: u32) -> u32 {
    let (ny, nm) = if m0 == 11 { (y + 1, 1) } else { (y, m0 + 2) };
    (days_from_civil(ny, nm, 1) - days_from_civil(y, m0 + 1, 1)) as u32
}

/// Monday-first weekday of the month's first day (0 = Monday .. 6 = Sunday).
pub fn first_weekday(y: i32, m0: u32) -> u32 {
    // 1970-01-01 was a Thursday (Monday-first index 3).
    ((days_from_civil(y, m0 + 1, 1) + 3).rem_euclid(7)) as u32
}

/// Today in UTC as `(year, month0 0..=11, day 1..=31)`.
pub fn today_utc() -> (i32, u32, u32) {
    let days = (crate::format::now_secs() / 86_400.0).floor() as i64;
    let (y, m, d) = civil(days);
    (y, m - 1, d)
}

/// Formats `(year, month0 0..=11, day)` as canonical `YYYY-MM-DD`.
pub fn format_iso(y: i32, m0: u32, d: u32) -> String {
    format!("{y:04}-{:02}-{d:02}", m0 + 1)
}

fn valid_ymd(y: i32, m1: u32, d: u32) -> Option<(i32, u32, u32)> {
    if !(1..=9999).contains(&y) || !(1..=12).contains(&m1) {
        return None;
    }
    let m0 = m1 - 1;
    let max_d = days_in_month(y, m0);
    if (1..=max_d).contains(&d) {
        Some((y, m0, d))
    } else {
        None
    }
}

/// Strictly parses `YYYY-MM-DD` into `(year, month0 0..=11, day 1..=31)`.
pub fn parse_iso(v: &str) -> Option<(i32, u32, u32)> {
    let v = v.trim();
    let b = v.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    if !b[0..4].iter().all(u8::is_ascii_digit)
        || !b[5..7].iter().all(u8::is_ascii_digit)
        || !b[8..10].iter().all(u8::is_ascii_digit)
    {
        return None;
    }
    let y = v[0..4].parse::<i32>().ok()?;
    let m = v[5..7].parse::<u32>().ok()?;
    let d = v[8..10].parse::<u32>().ok()?;
    valid_ymd(y, m, d)
}

/// Parses `YYYY-MM-DD`, `MM/DD/YYYY`, or `DD.MM.YYYY` with strict calendar validation
/// (including leap years) and returns canonical `YYYY-MM-DD`.
pub fn normalize_date(s: &str) -> Option<String> {
    let s = s.trim();
    if let Some((y, m0, d)) = parse_iso(s) {
        return Some(format_iso(y, m0, d));
    }
    let sep = if s.contains('/') && !s.contains('.') {
        '/'
    } else if s.contains('.') && !s.contains('/') {
        '.'
    } else {
        return None;
    };
    let parts: Vec<&str> = s.split(sep).collect();
    if parts.len() != 3
        || parts
            .iter()
            .any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    if parts[2].len() != 4 {
        return None;
    }
    let (m_str, d_str, y_str) = if sep == '/' {
        (parts[0], parts[1], parts[2])
    } else {
        (parts[1], parts[0], parts[2])
    };
    let y = y_str.parse::<i32>().ok()?;
    let m = m_str.parse::<u32>().ok()?;
    let d = d_str.parse::<u32>().ok()?;
    let (y, m0, d) = valid_ymd(y, m, d)?;
    Some(format_iso(y, m0, d))
}

/// Whether `s` is a valid calendar date in ISO (`YYYY-MM-DD`), US (`MM/DD/YYYY`), or
/// Austrian/German (`DD.MM.YYYY`) form.
pub fn is_valid_date(s: &str) -> bool {
    normalize_date(s).is_some()
}

const MONTH_FULL_KEYS: [&str; 12] = [
    "common:datePicker.monthsFull.jan",
    "common:datePicker.monthsFull.feb",
    "common:datePicker.monthsFull.mar",
    "common:datePicker.monthsFull.apr",
    "common:datePicker.monthsFull.may",
    "common:datePicker.monthsFull.jun",
    "common:datePicker.monthsFull.jul",
    "common:datePicker.monthsFull.aug",
    "common:datePicker.monthsFull.sep",
    "common:datePicker.monthsFull.oct",
    "common:datePicker.monthsFull.nov",
    "common:datePicker.monthsFull.dec",
];

const MONTH_SHORT_KEYS: [&str; 12] = [
    "common:datePicker.monthsShort.jan",
    "common:datePicker.monthsShort.feb",
    "common:datePicker.monthsShort.mar",
    "common:datePicker.monthsShort.apr",
    "common:datePicker.monthsShort.may",
    "common:datePicker.monthsShort.jun",
    "common:datePicker.monthsShort.jul",
    "common:datePicker.monthsShort.aug",
    "common:datePicker.monthsShort.sep",
    "common:datePicker.monthsShort.oct",
    "common:datePicker.monthsShort.nov",
    "common:datePicker.monthsShort.dec",
];

const WEEKDAY_KEYS: [&str; 7] = [
    "common:datePicker.weekdays.mon",
    "common:datePicker.weekdays.tue",
    "common:datePicker.weekdays.wed",
    "common:datePicker.weekdays.thu",
    "common:datePicker.weekdays.fri",
    "common:datePicker.weekdays.sat",
    "common:datePicker.weekdays.sun",
];

fn month_full(m0: u32, l: Lang) -> String {
    t_in(MONTH_FULL_KEYS[m0 as usize], l)
}

fn month_short(m0: u32, l: Lang) -> String {
    t_in(MONTH_SHORT_KEYS[m0 as usize], l)
}

fn weekday(i: usize, l: Lang) -> String {
    t_in(WEEKDAY_KEYS[i], l)
}

/// Formats a valid date for display: `Mar 05, 2024` (EN) or `05. März 2024` (DE-AT).
pub fn format_display(v: &str, l: Lang) -> Option<String> {
    let iso = normalize_date(v)?;
    let (y, m0, d) = parse_iso(&iso)?;
    let month = month_short(m0, l);
    let day = format!("{d:02}");
    let year = format!("{y:04}");
    Some(tr_in(
        "common:datePicker.displayFormat",
        l,
        &[("month", &month), ("day", &day), ("year", &year)],
    ))
}

/// 12-year page start containing `y`, anchored so `today_y` is the final cell of its page
/// (e.g. `2015..=2026` when `today_y == 2026`, `2003..=2014` one click back).
pub fn year_page_start(y: i32, today_y: i32) -> i32 {
    let offset = (today_y - y).div_euclid(12);
    (today_y - 11 - offset * 12).max(1)
}

fn tnum(s: TextStyle) -> TextStyle {
    s.features("'tnum' 1")
}

// ---- Actions ---------------------------------------------------------------------------------

fn toggle_open(app: &mut App, access: Access, key: &'static str, current: &str) {
    let already = (access.get)(app).is_some_and(|p| p.key == key);
    if already {
        *(access.get_mut)(app) = None;
        return;
    }
    let today = today_utc();
    let (y, m0, _) = normalize_date(current)
        .and_then(|iso| parse_iso(&iso))
        .unwrap_or(today);
    *(access.get_mut)(app) = Some(PickerState {
        key: key.to_owned(),
        year: y,
        month: m0,
        year_page_start: year_page_start(y, today.0),
        view: View::Days,
    });
}

fn step(app: &mut App, access: Access, delta: i32) {
    let Some(p) = (access.get_mut)(app).as_mut() else {
        return;
    };
    let today_y = today_utc().0;
    match p.view {
        View::Days => {
            let total = p.year * 12 + p.month as i32 + delta;
            let ny = total.div_euclid(12).clamp(1, 9999);
            let nm = total.rem_euclid(12) as u32;
            p.year = ny;
            p.month = nm;
            p.year_page_start = year_page_start(ny, today_y);
        }
        View::Months => {
            p.year = (p.year + delta).clamp(1, 9999);
            p.year_page_start = year_page_start(p.year, today_y);
        }
        View::Years => {
            p.year_page_start = (p.year_page_start + delta * 12).clamp(1, 9988);
        }
    }
}

// ---- View ------------------------------------------------------------------------------------

/// Renders a full-width date picker trigger (matching Kassiber's `h-9` form inputs) and mounts
/// its glass calendar + month/year popover into `layer` while open.
#[allow(clippy::too_many_arguments)]
pub fn render(
    app: &App,
    tk: &Tk,
    layer: &mut Vec<El>,
    access: Access,
    key: &'static str,
    label: &str,
    value: &str,
    enabled: bool,
    clearable: bool,
    on_select: fn(&mut App, String),
) -> El {
    let trimmed = value.trim();
    let has_value = !trimmed.is_empty();
    let l = lang();
    let shown = if has_value {
        format_display(trimmed, l).unwrap_or_else(|| trimmed.to_owned())
    } else {
        t("common:datePicker.selectDate")
    };
    let open = enabled && (access.get)(app).is_some_and(|p| p.key == key);
    let fill = if tk.dark {
        tk.input.alpha(tk.input.a * 0.3)
    } else {
        Color::TRANSPARENT
    };

    let trigger_name = if has_value {
        format!("{label}: {shown}")
    } else {
        label.to_owned()
    };
    let mut trigger = row()
        .key(key.to_owned())
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .px(tk.r(0.75))
        .gap(tk.r(0.5))
        .center_y()
        .role("button")
        .label(trigger_name)
        .expanded(open)
        .focusable(enabled)
        .disabled(!enabled)
        .child(ic(&I::CALENDAR_DAYS, tk.r(0.95), tk.muted_fg).a11y_hidden())
        .child(
            text(shown)
                .font(tnum(tk.tsm(400.0)))
                .color(if has_value { tk.fg } else { tk.muted_fg })
                .truncate()
                .grow(1.0)
                .min_w(0.0),
        )
        .child(ic(&I::CHEVRON_DOWN, tk.r(0.875), tk.muted_fg).a11y_hidden());
    if enabled {
        let current = trimmed.to_owned();
        let current_key = current.clone();
        trigger = trigger
            .hover(tk.muted.alpha(0.35))
            .on_press(move |a: &mut App| toggle_open(a, access, key, &current))
            .on_key(move |a: &mut App, ev: &KeyEvent, _tx: &mut Tx| {
                if ev.mods.cmd || ev.mods.ctrl || ev.mods.alt || ev.mods.logo || ev.mods.shift {
                    return false;
                }
                if ev.key == Key::Enter {
                    toggle_open(a, access, key, &current_key);
                    return true;
                }
                false
            });
    }

    let mut frame = row()
        .key(format!("{key}-box"))
        .w_full()
        .min_w(0.0)
        .h(tk.r(2.25))
        .center_y()
        .radius(tk.r(0.375))
        .fill(fill)
        .border(1.0, if open { tk.ring } else { tk.input })
        .border_layout()
        .clip_rounded(tk.r(0.375))
        .box_shadow(Shadow {
            y: 1.0,
            blur: 2.0,
            color: Color::BLACK.alpha(0.05),
            ..Shadow::default()
        })
        .opacity(if enabled { 1.0 } else { 0.5 })
        .child(trigger);

    if clearable && has_value && enabled {
        frame = frame.child(
            row()
                .key(format!("{key}-clear"))
                .size(tk.r(1.625), tk.r(1.625))
                .mr(tk.r(0.25))
                .center()
                .radius(tk.r(0.25))
                .hover(tk.accent)
                .role("button")
                .label(t("common:datePicker.clearDate"))
                .focusable(true)
                .on_press(move |a: &mut App| {
                    *(access.get_mut)(a) = None;
                    on_select(a, String::new());
                })
                .child(ic(&I::X, tk.r(0.875), tk.muted_fg).a11y_hidden()),
        );
    }

    if open && let Some(state) = (access.get)(app) {
        let close = move |a: &mut App| *(access.get_mut)(a) = None;
        layer.push(rune_kit::scrim(format!("{key}-scrim"), close));
        let pop = popover(tk, access, key, label, state, trimmed, clearable, on_select)
            .key(format!("{key}-popover"))
            .modal(true, close)
            .focus_return(key.to_owned());
        layer.push(crate::shell::anchored(
            tk,
            pop,
            key,
            Placement::BottomStart,
            4.0,
        ));
    }

    frame
}

#[allow(clippy::too_many_arguments)]
fn popover(
    tk: &Tk,
    access: Access,
    key: &'static str,
    label: &str,
    state: &PickerState,
    current: &str,
    clearable: bool,
    on_select: fn(&mut App, String),
) -> El {
    let l = lang();
    let selected = normalize_date(current).and_then(|iso| parse_iso(&iso));
    let today = today_utc();
    let cell_w = tk.r(2.0);
    let gap = tk.r(0.25);
    let grid_w = cell_w * 7.0 + gap * 6.0;

    let (prev_lbl, next_lbl) = match state.view {
        View::Days => (
            t("common:datePicker.previousMonth"),
            t("common:datePicker.nextMonth"),
        ),
        View::Months => (
            t("common:datePicker.previousYear"),
            t("common:datePicker.nextYear"),
        ),
        View::Years => (
            t("common:datePicker.previousYears"),
            t("common:datePicker.nextYears"),
        ),
    };
    let nav = |suffix: &str, glyph: &'static rune::Icon, aria: String, delta: i32| {
        button::icon_button(
            tk,
            format!("{key}-{suffix}"),
            button::Variant::Ghost,
            tk.r(2.0),
            glyph,
            tk.r(1.0),
            aria,
        )
        .on_press(move |a: &mut App| step(a, access, delta))
    };

    let pill_btn = |suffix: &str, shown: String, aria: String, next_view: View| {
        row()
            .key(format!("{key}-{suffix}"))
            .h(tk.r(1.75))
            .px(tk.r(0.45))
            .gap(tk.r(0.2))
            .center_y()
            .radius(tk.r(0.375))
            .hover(tk.accent)
            .role("button")
            .label(aria)
            .focusable(true)
            .on_press(move |a: &mut App| {
                if let Some(p) = (access.get_mut)(a).as_mut() {
                    if next_view == View::Years {
                        p.year_page_start = year_page_start(p.year, today_utc().0);
                    }
                    p.view = next_view;
                }
            })
            .child(text(shown).font(tnum(tk.tsm(500.0))).color(tk.fg))
            .child(ic(&I::CHEVRON_DOWN, tk.r(0.75), tk.muted_fg).a11y_hidden())
    };

    let center = match state.view {
        View::Days => row()
            .gap(tk.r(0.125))
            .center_y()
            .child(pill_btn(
                "pick-month",
                month_full(state.month, l),
                t("common:datePicker.chooseMonth"),
                View::Months,
            ))
            .child(pill_btn(
                "pick-year",
                format!("{:04}", state.year),
                t("common:datePicker.chooseYear"),
                View::Years,
            )),
        View::Months => pill_btn(
            "pick-year",
            format!("{:04}", state.year),
            t("common:datePicker.chooseYear"),
            View::Years,
        ),
        View::Years => {
            let start = state.year_page_start;
            let end = (start + 11).min(9999);
            row().h(tk.r(1.75)).center_y().child(
                text(format!("{start:04} – {end:04}"))
                    .font(tnum(tk.tsm(500.0)))
                    .color(tk.fg),
            )
        }
    };

    let header = row()
        .w(grid_w)
        .pb(tk.r(0.5))
        .center_y()
        .justify_between()
        .child(nav("cal-prev", &I::CHEVRON_LEFT, prev_lbl, -1))
        .child(center)
        .child(nav("cal-next", &I::CHEVRON_RIGHT, next_lbl, 1));

    let body = match state.view {
        View::Days => days_grid(
            tk,
            access,
            key,
            state.year,
            state.month,
            selected,
            today,
            cell_w,
            gap,
            grid_w,
            l,
            on_select,
        ),
        View::Months => months_grid(
            tk,
            access,
            key,
            state.year,
            state.month,
            selected,
            grid_w,
            l,
        ),
        View::Years => years_grid(
            tk,
            access,
            key,
            state.year,
            state.year_page_start,
            selected,
            today.0,
            grid_w,
        ),
    };

    let mut footer = row()
        .w(grid_w)
        .pt(tk.r(0.5))
        .mt(tk.r(0.5))
        .justify_between()
        .center_y()
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border.alpha(tk.border.a * 0.7))
        .border_layout()
        .child(
            row()
                .key(format!("{key}-today"))
                .h(tk.r(1.75))
                .px(tk.r(0.5))
                .center()
                .radius(tk.r(0.375))
                .hover(tk.accent)
                .role("button")
                .label(t("common:datePicker.today"))
                .focusable(true)
                .on_press(move |a: &mut App| {
                    let (ty, tm, td) = today_utc();
                    *(access.get_mut)(a) = None;
                    on_select(a, format_iso(ty, tm, td));
                })
                .child(
                    text(t("common:datePicker.today"))
                        .font(tk.txs(500.0))
                        .color(tk.fg),
                ),
        );
    if clearable && !current.is_empty() {
        footer = footer.child(
            row()
                .key(format!("{key}-pop-clear"))
                .h(tk.r(1.75))
                .px(tk.r(0.5))
                .center()
                .radius(tk.r(0.375))
                .hover(tk.accent)
                .role("button")
                .label(t("common:datePicker.clearDate"))
                .focusable(true)
                .on_press(move |a: &mut App| {
                    *(access.get_mut)(a) = None;
                    on_select(a, String::new());
                })
                .child(
                    text(t("common:datePicker.clearDate"))
                        .font(tk.txs(500.0))
                        .color(tk.muted_fg),
                ),
        );
    }

    crate::shell::glass(
        tk,
        col()
            .padding(tk.r(0.75))
            .child(header)
            .child(body)
            .child(footer),
    )
    .radius(tk.r(0.5))
    .role("dialog")
    .label(label.to_owned())
}

#[allow(clippy::too_many_arguments)]
fn days_grid(
    tk: &Tk,
    access: Access,
    key: &'static str,
    vy: i32,
    vm: u32,
    selected: Option<(i32, u32, u32)>,
    today: (i32, u32, u32),
    cell_w: f32,
    gap: f32,
    grid_w: f32,
    l: Lang,
    on_select: fn(&mut App, String),
) -> El {
    let wd_row = row().w(grid_w).gap(gap).children((0..7).map(|i| {
        row()
            .w(cell_w)
            .py(tk.r(0.25))
            .center()
            .child(text(weekday(i, l)).font(tk.txs(500.0)).color(tk.muted_fg))
    }));
    let first = first_weekday(vy, vm);
    let n = days_in_month(vy, vm);
    let initial_day = match selected {
        Some((sy, sm, sd)) if sy == vy && sm == vm => sd,
        _ if today.0 == vy && today.1 == vm => today.2,
        _ => 1,
    };
    let mut weeks = col()
        .gap(gap)
        .role("grid")
        .label(format!("{} {vy:04}", month_full(vm, l)));
    let mut week = row().gap(gap);
    let mut slot = 0;
    for _ in 0..first {
        week = week.child(el().size(cell_w, cell_w));
        slot += 1;
    }
    for d in 1..=n {
        let is_sel = selected == Some((vy, vm, d));
        let is_today = (vy, vm, d) == today;
        let iso = format_iso(vy, vm, d);
        let iso_key = iso.clone();
        let cur_days = days_from_civil(vy, vm + 1, d);
        let mut b = row()
            .key(format!("{key}-day-{d}"))
            .size(cell_w, cell_w)
            .center()
            .radius(tk.r(0.375))
            .role("button")
            .label(iso.clone())
            .checked(is_sel)
            .focusable(true)
            .focus_selected(is_sel)
            .on_press(move |a: &mut App| {
                *(access.get_mut)(a) = None;
                on_select(a, iso.clone());
            })
            .on_key(move |a: &mut App, ev: &KeyEvent, tx: &mut Tx| {
                if ev.mods.cmd || ev.mods.ctrl || ev.mods.alt || ev.mods.logo || ev.mods.shift {
                    return false;
                }
                if ev.key == Key::Enter {
                    *(access.get_mut)(a) = None;
                    on_select(a, iso_key.clone());
                    tx.focus(key.to_owned(), true);
                    return true;
                }
                let next_ymd = match ev.key {
                    Key::Left => Some(civil(cur_days - 1)),
                    Key::Right => Some(civil(cur_days + 1)),
                    Key::Up => Some(civil(cur_days - 7)),
                    Key::Down => Some(civil(cur_days + 7)),
                    Key::Home => {
                        let wd = (cur_days + 3).rem_euclid(7);
                        Some(civil(cur_days - wd))
                    }
                    Key::End => {
                        let wd = (cur_days + 3).rem_euclid(7);
                        Some(civil(cur_days + (6 - wd)))
                    }
                    Key::PageUp | Key::PageDown => {
                        let delta = if ev.key == Key::PageUp { -1 } else { 1 };
                        let total = vy * 12 + vm as i32 + delta;
                        let ny = total.div_euclid(12).clamp(1, 9999);
                        let nm0 = total.rem_euclid(12) as u32;
                        let nd = d.min(days_in_month(ny, nm0));
                        Some((ny, nm0 + 1, nd))
                    }
                    _ => None,
                };
                let Some((ny, nm1, nd)) = next_ymd else {
                    return false;
                };
                if !(1..=9999).contains(&ny) {
                    return true;
                }
                let nm0 = nm1 - 1;
                if let Some(p) = (access.get_mut)(a).as_mut() {
                    p.year = ny;
                    p.month = nm0;
                    p.year_page_start = year_page_start(ny, today_utc().0);
                }
                tx.focus(format!("{key}-day-{nd}"), true);
                true
            });
        if d == initial_day {
            b = b.autofocus();
        }
        b = if is_sel {
            b.fill(tk.primary)
        } else {
            let b = b.hover(tk.accent);
            if is_today {
                b.border(1.0, tk.ring).border_layout()
            } else {
                b
            }
        };
        week = week.child(
            b.child(
                text(d.to_string())
                    .font(tnum(tk.tsm(400.0)))
                    .color(if is_sel { tk.primary_fg } else { tk.fg }),
            ),
        );
        slot += 1;
        if slot % 7 == 0 {
            weeks = weeks.child(week);
            week = row().gap(gap);
        }
    }
    if slot % 7 != 0 {
        weeks = weeks.child(week);
    }
    col().child(wd_row).child(el().h(gap)).child(weeks)
}

#[allow(clippy::too_many_arguments)]
fn months_grid(
    tk: &Tk,
    access: Access,
    key: &'static str,
    vy: i32,
    vm: u32,
    selected: Option<(i32, u32, u32)>,
    grid_w: f32,
    l: Lang,
) -> El {
    let gap = tk.r(0.25);
    let mut grid = col()
        .w(grid_w)
        .gap(gap)
        .role("grid")
        .label(t("common:datePicker.chooseMonth"));
    for r_idx in 0..4 {
        let mut r = row().w_full().gap(gap);
        for c_idx in 0..3 {
            let m0 = (r_idx * 3 + c_idx) as u32;
            let is_sel = selected.is_some_and(|(sy, sm, _)| sy == vy && sm == m0);
            let is_cur = vm == m0;
            let mut b = row()
                .key(format!("{key}-month-{m0}"))
                .grow(1.0)
                .w(0.0)
                .h(tk.r(2.25))
                .center()
                .radius(tk.r(0.375))
                .role("button")
                .label(month_full(m0, l))
                .checked(is_sel)
                .focusable(true)
                .focus_selected(is_sel || is_cur)
                .on_press(move |a: &mut App| {
                    if let Some(p) = (access.get_mut)(a).as_mut() {
                        p.month = m0;
                        p.view = View::Days;
                    }
                })
                .on_key(move |a: &mut App, ev: &KeyEvent, tx: &mut Tx| {
                    if ev.mods.cmd || ev.mods.ctrl || ev.mods.alt || ev.mods.logo || ev.mods.shift {
                        return false;
                    }
                    if ev.key == Key::Enter {
                        let target_day = match selected {
                            Some((sy, sm, sd)) if sy == vy && sm == m0 => sd,
                            _ => 1,
                        };
                        if let Some(p) = (access.get_mut)(a).as_mut() {
                            p.month = m0;
                            p.view = View::Days;
                        }
                        tx.focus(format!("{key}-day-{target_day}"), true);
                        return true;
                    }
                    let delta = match ev.key {
                        Key::Left => Some(-1),
                        Key::Right => Some(1),
                        Key::Up => Some(-3),
                        Key::Down => Some(3),
                        Key::PageUp => Some(-12),
                        Key::PageDown => Some(12),
                        Key::Home => Some(-(m0 as i32)),
                        Key::End => Some(11 - m0 as i32),
                        _ => None,
                    };
                    let Some(delta) = delta else {
                        return false;
                    };
                    let total = vy * 12 + m0 as i32 + delta;
                    let ny = total.div_euclid(12).clamp(1, 9999);
                    let nm0 = total.rem_euclid(12) as u32;
                    if let Some(p) = (access.get_mut)(a).as_mut() {
                        p.year = ny;
                        p.month = nm0;
                        p.year_page_start = year_page_start(ny, today_utc().0);
                    }
                    tx.focus(format!("{key}-month-{nm0}"), true);
                    true
                });
            if is_cur {
                b = b.autofocus();
            }
            b = if is_sel {
                b.fill(tk.primary)
            } else {
                let b = b.hover(tk.accent);
                if is_cur {
                    b.border(1.0, tk.ring).border_layout()
                } else {
                    b
                }
            };
            r = r.child(
                b.child(
                    text(month_short(m0, l))
                        .font(tk.tsm(if is_sel || is_cur { 500.0 } else { 400.0 }))
                        .color(if is_sel { tk.primary_fg } else { tk.fg }),
                ),
            );
        }
        grid = grid.child(r);
    }
    grid
}

#[allow(clippy::too_many_arguments)]
fn years_grid(
    tk: &Tk,
    access: Access,
    key: &'static str,
    vy: i32,
    start: i32,
    selected: Option<(i32, u32, u32)>,
    today_y: i32,
    grid_w: f32,
) -> El {
    let gap = tk.r(0.25);
    let mut grid = col()
        .w(grid_w)
        .gap(gap)
        .role("grid")
        .label(t("common:datePicker.chooseYear"));
    let initial_year = if (start..=(start + 11).min(9999)).contains(&vy) {
        vy
    } else {
        start
    };
    for r_idx in 0..3 {
        let mut r = row().w_full().gap(gap);
        for c_idx in 0..4 {
            let y = start + r_idx * 4 + c_idx;
            if y > 9999 {
                r = r.child(el().grow(1.0).w(0.0));
                continue;
            }
            let is_sel = selected.is_some_and(|(sy, _, _)| sy == y);
            let is_cur = y == vy || y == today_y;
            let mut b = row()
                .key(format!("{key}-year-{y}"))
                .grow(1.0)
                .w(0.0)
                .h(tk.r(2.25))
                .center()
                .radius(tk.r(0.375))
                .role("button")
                .label(format!("{y:04}"))
                .checked(is_sel)
                .focusable(true)
                .focus_selected(is_sel || y == vy)
                .on_press(move |a: &mut App| {
                    if let Some(p) = (access.get_mut)(a).as_mut() {
                        p.year = y;
                        p.view = View::Months;
                    }
                })
                .on_key(move |a: &mut App, ev: &KeyEvent, tx: &mut Tx| {
                    if ev.mods.cmd || ev.mods.ctrl || ev.mods.alt || ev.mods.logo || ev.mods.shift {
                        return false;
                    }
                    if ev.key == Key::Enter {
                        let month = (access.get)(a).map(|p| p.month).unwrap_or(0);
                        if let Some(p) = (access.get_mut)(a).as_mut() {
                            p.year = y;
                            p.view = View::Months;
                        }
                        tx.focus(format!("{key}-month-{month}"), true);
                        return true;
                    }
                    let ny = match ev.key {
                        Key::Left => Some((y - 1).clamp(1, 9999)),
                        Key::Right => Some((y + 1).clamp(1, 9999)),
                        Key::Up => Some((y - 4).clamp(1, 9999)),
                        Key::Down => Some((y + 4).clamp(1, 9999)),
                        Key::PageUp => Some((y - 12).clamp(1, 9999)),
                        Key::PageDown => Some((y + 12).clamp(1, 9999)),
                        Key::Home => Some(start),
                        Key::End => Some((start + 11).min(9999)),
                        _ => None,
                    };
                    let Some(ny) = ny else {
                        return false;
                    };
                    if let Some(p) = (access.get_mut)(a).as_mut() {
                        if ny < p.year_page_start || ny > p.year_page_start + 11 {
                            p.year_page_start = year_page_start(ny, today_utc().0);
                        }
                        p.year = ny;
                    }
                    tx.focus(format!("{key}-year-{ny}"), true);
                    true
                });
            if y == initial_year {
                b = b.autofocus();
            }
            b = if is_sel {
                b.fill(tk.primary)
            } else {
                let b = b.hover(tk.accent);
                if is_cur {
                    b.border(1.0, tk.ring).border_layout()
                } else {
                    b
                }
            };
            r = r.child(
                b.child(
                    text(format!("{y:04}"))
                        .font(tnum(tk.tsm(if is_sel || y == vy { 500.0 } else { 400.0 })))
                        .color(if is_sel { tk.primary_fg } else { tk.fg }),
                ),
            );
        }
        grid = grid.child(r);
    }
    grid
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_date_accepts_iso_us_and_german_and_rejects_impossible_dates() {
        assert_eq!(normalize_date("2024-03-05").as_deref(), Some("2024-03-05"));
        assert_eq!(normalize_date("03/05/2024").as_deref(), Some("2024-03-05"));
        assert_eq!(normalize_date("3/5/2024").as_deref(), Some("2024-03-05"));
        assert_eq!(normalize_date("05.03.2024").as_deref(), Some("2024-03-05"));
        assert_eq!(normalize_date("5.3.2024").as_deref(), Some("2024-03-05"));
        assert_eq!(normalize_date("2024-02-29").as_deref(), Some("2024-02-29"));
        assert_eq!(normalize_date("02/29/2024").as_deref(), Some("2024-02-29"));
        assert_eq!(normalize_date("29.02.2024").as_deref(), Some("2024-02-29"));
        // Non-leap year Feb 29, Feb 30, month 13, mixed separators, garbage.
        assert_eq!(normalize_date("2023-02-29"), None);
        assert_eq!(normalize_date("2024-02-30"), None);
        assert_eq!(normalize_date("2024-13-01"), None);
        assert_eq!(normalize_date("2024-00-10"), None);
        assert_eq!(normalize_date("2024/03/05"), None);
        assert_eq!(normalize_date("2024.03.05"), None);
        assert_eq!(normalize_date("05.03/2024"), None);
        assert_eq!(normalize_date("2024x03x05"), None);
        assert_eq!(normalize_date("32.01.2024"), None);
        assert_eq!(normalize_date("13/01/2024"), None);
        assert_eq!(normalize_date(""), None);
    }

    #[test]
    fn format_display_handles_english_and_austrian_german() {
        assert_eq!(
            format_display("2024-01-09", Lang::En).as_deref(),
            Some("Jan 09, 2024")
        );
        assert_eq!(
            format_display("01/09/2024", Lang::De).as_deref(),
            Some("09. Jän. 2024")
        );
        assert_eq!(
            format_display("05.03.2024", Lang::De).as_deref(),
            Some("05. März 2024")
        );
        assert_eq!(format_display("2023-02-29", Lang::En), None);
    }

    #[test]
    fn year_page_anchors_recent_years_and_reaches_bitcoin_genesis_in_one_step() {
        assert_eq!(year_page_start(2026, 2026), 2015);
        assert_eq!(year_page_start(2015, 2026), 2015);
        assert_eq!(year_page_start(2014, 2026), 2003);
        assert_eq!(year_page_start(2009, 2026), 2003);
        assert_eq!(year_page_start(2027, 2026), 2027);
    }
}
