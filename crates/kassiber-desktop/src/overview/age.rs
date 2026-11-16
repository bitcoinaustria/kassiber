//! Coins by holding age (Rune only, an owner-requested widget): the stack in four bands by how
//! long the book's balance has stayed at least that high: under a year, one to two, two to
//! four, four years and more.
//!
//! The data is the overview snapshot's daily balance (`ui.overview.snapshot` →
//! `portfolioSeries[].balanceBtc`), already on screen for the chart; no other read. No daemon
//! read returns the open lots with their acquisition dates, and lot selection belongs to the
//! tax engine, so the bands are not lots: the amount held for at least a year is the lowest the
//! balance has been since a year ago (coins that never left), whatever the book's method. The
//! tax side shows only what the book's rules and the daemon state, never a figure made here:
//! Austria's tax-free old holdings (`taxFreeBalance`, the daemon's Alt/Neu split), and for a
//! book with a holding period (`generic` with its day count, `us`, `de`) the rule itself, with
//! the per-lot answer left to the gains report. Countries without one show the plain bands.

use rune::{El, col, el, row, text};

use super::grid::frame::{self, Slot};
use super::holdings::now_secs;
use super::holdings_model as hm;
use super::stats::mask;
use crate::app::App;
use crate::format::{self, Day};
use crate::i18n::{Lang, lang, t};
use crate::model::Snapshot;
use crate::theme::Tk;
use crate::ui::sensitive;

/// The bands' lower bounds in years.
pub const BANDS: [u32; 3] = [1, 2, 4];

/// The stack by holding age: `bands[0]` under a year, `[1]` one to two, `[2]` two to four,
/// `[3]` four years and more. They add up to `total`.
#[derive(Clone, Debug, PartialEq)]
pub struct Ages {
    pub total: f64,
    pub bands: [f64; 4],
}

impl Ages {
    /// The share held a year or longer (0–1).
    pub fn over_one_year(&self) -> f64 {
        if self.total > 0.0 {
            (self.total - self.bands[0]) / self.total
        } else {
            0.0
        }
    }
}

/// The same calendar day `years` earlier (29 February becomes the 28th).
pub fn years_before(day: Day, years: u32) -> Day {
    let y = day.y - years as i32;
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let d = if day.m == 2 && day.d == 29 && !leap {
        28
    } else {
        day.d
    };
    Day { y, m: day.m, d }
}

/// The balance at the end of `day`: the last point on or before it (0 before the first).
fn balance_on(points: &[(i64, f64)], day: i64) -> f64 {
    points
        .iter()
        .take_while(|(d, _)| *d <= day)
        .last()
        .map(|(_, b)| *b)
        .unwrap_or(0.0)
}

/// The amount held since `cut`: the lowest the balance has been from that day to `today`.
fn held_since(points: &[(i64, f64)], cut: i64, today: i64) -> f64 {
    let at_cut = balance_on(points, cut);
    points
        .iter()
        .filter(|(d, _)| *d > cut && *d <= today)
        .map(|(_, b)| *b)
        .fold(at_cut, f64::min)
        .max(0.0)
}

/// Bands the daily balance `points` (epoch day, BTC; sorted by day) as of `today`. `None`
/// when nothing is held.
pub fn bands(points: &[(i64, f64)], today: Day) -> Option<Ages> {
    let t = today.epoch_days();
    let total = balance_on(points, t);
    if total <= 0.0 {
        return None;
    }
    let held =
        |years: u32| held_since(points, years_before(today, years).epoch_days(), t).min(total);
    let (h1, h2, h4) = (held(BANDS[0]), held(BANDS[1]), held(BANDS[2]));
    let pos = |x: f64| x.max(0.0);
    Some(Ages {
        total,
        bands: [pos(total - h1), pos(h1 - h2), pos(h2 - h4), pos(h4)],
    })
}

/// The snapshot's daily balance as (epoch day, BTC), oldest first.
pub fn points(s: &Snapshot) -> Vec<(i64, f64)> {
    let mut out: Vec<(i64, f64)> = s
        .portfolio_series
        .iter()
        .filter_map(|p| Some((Day::parse(&p.date)?.epoch_days(), p.balance_btc)))
        .collect();
    out.sort_by_key(|p| p.0);
    out
}

/// Today in UTC (the scenes' pinned clock where set).
pub fn today(app: &App) -> Day {
    Day::from_epoch_days((now_secs(app) / 86_400.0).floor() as i64)
}

/// What the book's tax rules say about holding age (`crates/tax-countries`' `long_term`).
#[derive(Clone, Debug, PartialEq)]
pub enum Rule {
    /// No holding period (`never`, e.g. `ch`, `gb`, `ie`, `es`, `jp`), or a country the
    /// registry does not know.
    None,
    /// Long-term after this many days (`profile_days`: the generic book's own count).
    Days(u32),
    /// Long-term when held more than one year (`more_than_one_year`: `us`, `de`, `au`).
    OverOneYear,
    /// Austria: no holding period for new holdings; old holdings are tax-free (the daemon's
    /// `taxFreeBalance`).
    Austria,
}

/// The active book's rule, from `ui.profiles.snapshot` (already read by the shell).
pub fn rule(app: &App) -> Rule {
    let Some(p) = app.read("ui.profiles.snapshot") else {
        return Rule::None;
    };
    let active = p["activeProfileId"].as_str().unwrap_or("");
    let profile = p["workspaces"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|w| w["profiles"].as_array())
        .flatten()
        .find(|x| x["id"].as_str() == Some(active));
    let Some(profile) = profile else {
        return Rule::None;
    };
    rule_for(
        profile["taxCountry"].as_str().unwrap_or("generic"),
        profile["taxLongTermDays"].as_u64(),
    )
}

/// The registry's holding-period kind for the book's country (`reports::books::model`'s copy of
/// `crates/tax-countries` `COUNTRIES`). A `profile_days` book without a stored count takes the
/// registry's 365; a stored count, 0 included, is the book's own.
pub fn rule_for(country: &str, long_term_days: Option<u64>) -> Rule {
    use crate::reports::books::model::{self as registry, LongTerm};
    let code = country.trim().to_lowercase();
    let Some(entry) = registry::country(&code) else {
        return Rule::None;
    };
    match entry.long_term {
        // Austria's registry kind is `never`; its old holdings are the daemon's tax-free balance.
        LongTerm::Never if entry.code == "at" => Rule::Austria,
        LongTerm::Never => Rule::None,
        LongTerm::MoreThanOneYear => Rule::OverOneYear,
        LongTerm::Days => Rule::Days(long_term_days.unwrap_or(365).min(u64::from(u32::MAX)) as u32),
    }
}

fn l(en: &str, de: &str) -> String {
    match lang() {
        Lang::De => de.to_owned(),
        Lang::En => en.to_owned(),
    }
}

pub fn title() -> String {
    l("Coins by holding age", "Bestand nach Haltedauer")
}

pub fn description() -> String {
    l(
        "How long the stack has been held, in four age bands.",
        "Wie lange der Bestand gehalten wird, in vier Altersstufen.",
    )
}

pub fn band_label(i: usize) -> String {
    match i {
        0 => l("Under 1 year", "Unter 1 Jahr"),
        1 => l("1–2 years", "1–2 Jahre"),
        2 => l("2–4 years", "2–4 Jahre"),
        _ => l("4 years and more", "4 Jahre und mehr"),
    }
}

fn older_than_year() -> String {
    l("held 1 year or longer", "1 Jahr oder länger gehalten")
}

fn none_held() -> String {
    l("No bitcoin held yet.", "Noch keine Bitcoin im Bestand.")
}

/// The rule line under the bands (Medium).
fn rule_line(rule: &Rule) -> Option<String> {
    Some(match rule {
        // A book set to 0 days states no holding period here (the line never claims a count
        // the book does not hold).
        Rule::None | Rule::Austria | Rule::Days(0) => return None,
        Rule::Days(d) => match lang() {
            Lang::De => {
                format!("Langfristig nach {d} Tagen laut Buchregel · je Los im Gewinnbericht")
            }
            Lang::En => format!(
                "Long-term after {d} days under the book's rule · per lot in the gains report"
            ),
        },
        Rule::OverOneYear => l(
            "Long-term after more than 1 year under the book's rule · per lot in the gains report",
            "Langfristig nach mehr als 1 Jahr laut Buchregel · je Los im Gewinnbericht",
        ),
    })
}

fn pct(share: f64) -> String {
    let p = share * 100.0;
    if p > 0.0 && p < 1.0 {
        format!("<{}", super::goal::percent(1.0, 0))
    } else {
        super::goal::percent(p.round(), 0)
    }
}

/// Band tones: the youngest faintest, the oldest the bitcoin orange.
fn band_ink(tk: &Tk, i: usize) -> rune::Color {
    let orange = rune::color::hex("#f7931a").unwrap_or(tk.fg);
    match i {
        0 => tk.fg.alpha(0.28),
        1 => orange.alpha(0.45),
        2 => orange.alpha(0.7),
        _ => orange,
    }
}

/// The four bands as one stacked bar (`neutral`: one plain track, the split hidden).
fn stacked(tk: &Tk, a: &Ages, h: f32, neutral: bool) -> El {
    let bar = row().w_full().h(h).gap(2.0).a11y_hidden();
    if neutral || a.total <= 0.0 {
        return bar.child(el().grow(1.0).h(h).radius(h / 2.0).fill(tk.muted));
    }
    bar.children(
        a.bands
            .iter()
            .enumerate()
            .filter(|(_, v)| **v > 0.0)
            .map(|(i, v)| {
                el().grow((*v / a.total) as f32 * 100.0)
                    .w(0.0)
                    .min_w(3.0)
                    .h(h)
                    .radius(h / 2.0)
                    .fill(band_ink(tk, i))
            }),
    )
}

/// The content height at `size`: the headline over the stacked bar (Small), or the four bands
/// and the book's rule line (Medium).
pub fn natural(app: &App, tk: &Tk, size: super::grid::model::Size) -> f32 {
    if size == super::grid::model::Size::Small {
        return tk.r(2.25) * 1.15
            + tk.r(0.125)
            + frame::label(tk).size * 1.5
            + tk.r(0.5)
            + tk.txs(500.0).size * 1.5
            + tk.r(0.5)
            + tk.r(0.5);
    }
    let band = tk.t2xs(500.0).size * 1.5 + tk.r(0.1875) + tk.r(0.375);
    let rule = if rule_line(&rule(app)).is_some() {
        tk.r(0.375) + tk.t2xs(400.0).size * 1.5
    } else {
        0.0
    };
    4.0 * band + 3.0 * tk.r(0.375) + rule
}

/// The widget's content for its slot.
pub fn body(app: &App, tk: &Tk, s: &Snapshot, x: &hm::Extras, slot: &Slot) -> El {
    let hide = app.prefs.hide_sensitive();
    let hide_shares = app.prefs.hide_shares();
    let Some(a) = bands(&points(s), today(app)) else {
        return frame::empty(tk, slot.key("age-empty"), none_held());
    };
    let rule = rule(app);
    let share = a.over_one_year();
    let headline = col()
        .key(slot.key("age-headline"))
        .min_w(0.0)
        .gap(tk.r(0.125))
        .child(
            sensitive(
                mask(pct(share), hide_shares),
                frame::figure(tk, if slot.wide() { 1.75 } else { 2.25 }),
                tk.fg,
                hide_shares,
            )
            .truncate(),
        )
        .child(
            text(older_than_year())
                .font(frame::label(tk))
                .color(tk.muted_fg)
                .truncate(),
        );
    if !slot.wide() {
        let held_btc = a.total - a.bands[0];
        return col()
            .w_full()
            .h_full()
            .min_h(0.0)
            .gap(tk.r(0.5))
            .justify_between()
            .child(headline)
            .child(
                col()
                    .w_full()
                    .gap(tk.r(0.5))
                    .child(
                        sensitive(
                            mask(format::btc(held_btc, 3, false), hide),
                            tk.txs(500.0).features("'tnum' 1"),
                            tk.muted_fg,
                            hide,
                        )
                        .truncate(),
                    )
                    .child(stacked(tk, &a, tk.r(0.5), hide_shares)),
            );
    }
    // Medium: the headline beside the bands, a row each.
    let mut list = col()
        .key(slot.key("age-bands"))
        .grow(1.0)
        .w(0.0)
        .min_w(0.0)
        .gap(tk.r(0.375))
        .role("list")
        .label(title());
    for (i, v) in a.bands.iter().enumerate() {
        let share = if a.total > 0.0 { v / a.total } else { 0.0 };
        let track_h = tk.r(0.375);
        let mut track = row()
            .w_full()
            .h(track_h)
            .radius(track_h / 2.0)
            .fill(tk.muted)
            .a11y_hidden();
        if !hide_shares && *v > 0.0 {
            track = track.child(
                el().w_frac(share.clamp(0.0, 1.0) as f32)
                    .min_w(3.0)
                    .h(track_h)
                    .radius(track_h / 2.0)
                    .fill(band_ink(tk, i)),
            );
        }
        let amount = mask(format::btc(*v, 3, false), hide);
        let share_text = mask(pct(share), hide_shares);
        let name = band_label(i);
        let spoken = if hide || hide_shares {
            name.clone()
        } else {
            format!("{name}: {amount}, {share_text}")
        };
        list = list.child(
            col()
                .key(slot.key(format!("age-band-{i}")))
                .w_full()
                .gap(tk.r(0.1875))
                .role("listitem")
                .label(spoken)
                .child(
                    row()
                        .w_full()
                        .min_w(0.0)
                        .gap(tk.r(0.5))
                        .center_y()
                        .child(
                            text(name)
                                .font(tk.t2xs(500.0))
                                .color(tk.muted_fg)
                                .truncate()
                                .grow(1.0)
                                .min_w(0.0),
                        )
                        .child(sensitive(
                            amount,
                            tk.t2xs(500.0).features("'tnum' 1"),
                            tk.fg,
                            hide,
                        ))
                        .child(row().w(tk.r(2.0)).justify_end().child(sensitive(
                            share_text,
                            tk.t2xs(500.0).features("'tnum' 1"),
                            tk.muted_fg,
                            hide_shares,
                        ))),
                )
                .child(track),
        );
    }
    let mut left = col()
        .w(slot.w * 0.34)
        .min_w(0.0)
        .h_full()
        .gap(tk.r(0.5))
        .justify_between()
        .child(headline);
    // Austria: the daemon's tax-free old holdings, only while the split is current.
    if rule == Rule::Austria
        && let Some(tf) = x.tax_free.as_ref().filter(|tf| tf.status() == "current")
    {
        left = left.child(
            col()
                .key(slot.key("age-taxfree"))
                .min_w(0.0)
                .child(
                    text(t("overview:widgets.tax.taxFree"))
                        .font(tk.t2xs(500.0))
                        .color(tk.muted_fg)
                        .truncate(),
                )
                .child(
                    sensitive(
                        mask(
                            format::btc(tf.tax_free_sats / 100_000_000.0, 3, false),
                            hide,
                        ),
                        tk.txs(600.0).features("'tnum' 1"),
                        tk.fg,
                        hide,
                    )
                    .truncate(),
                ),
        );
    }
    let top = row()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .gap(tk.r(1.0))
        .items_start()
        .child(left)
        .child(list);
    let mut out = col()
        .w_full()
        .h_full()
        .min_h(0.0)
        .clip_rounded(0.0)
        .gap(tk.r(0.375))
        .child(top);
    if let Some(r) = rule_line(&rule) {
        out = out.child(
            text(r)
                .font(tk.t2xs(400.0))
                .color(tk.muted_fg)
                .truncate()
                .key(slot.key("age-rule"))
                .help(rule_line(&rule).unwrap_or_default()),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> i64 {
        Day::parse(s).unwrap().epoch_days()
    }

    #[test]
    fn bands_follow_the_lowest_balance_since_each_cut() {
        let today = Day::parse("2026-11-16").unwrap();
        // 5 bought in 2020, 3 more in 2023, 1 spent in 2025-06, 2 bought in 2026-03.
        let pts = vec![
            (d("2020-01-10"), 5.0),
            (d("2023-02-01"), 8.0),
            (d("2025-06-01"), 7.0),
            (d("2026-03-01"), 9.0),
        ];
        let a = bands(&pts, today).unwrap();
        assert_eq!(a.total, 9.0);
        // Since 2025-11-16 the balance was 7 then 9: 7 held a year; since 2024-11-16 it was 8, 7, 9:
        // 7; since 2022-11-16: 5, 8, 7, 9 → 5; since 2020-11-16: 5.
        assert_eq!(a.bands, [2.0, 0.0, 2.0, 5.0]);
        assert!((a.bands.iter().sum::<f64>() - a.total).abs() < 1e-12);
        assert!((a.over_one_year() - 7.0 / 9.0).abs() < 1e-12);
    }

    #[test]
    fn a_young_book_holds_nothing_old_and_an_empty_one_has_no_bands() {
        let today = Day::parse("2026-11-16").unwrap();
        let a = bands(&[(d("2026-05-01"), 0.5)], today).unwrap();
        assert_eq!(a.bands, [0.5, 0.0, 0.0, 0.0]);
        assert_eq!(a.over_one_year(), 0.0);
        assert_eq!(bands(&[], today), None);
        assert_eq!(
            bands(&[(d("2020-01-01"), 1.0), (d("2024-01-01"), 0.0)], today),
            None,
            "all spent"
        );
        // Points after today are not counted.
        let a = bands(&[(d("2019-01-01"), 1.0), (d("2027-01-01"), 3.0)], today).unwrap();
        assert_eq!(a.bands, [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn a_dip_below_an_old_level_ages_only_what_stayed() {
        let today = Day::parse("2026-11-16").unwrap();
        // 10 held since 2018, down to 1 for a day in 2026, back to 10.
        let pts = vec![
            (d("2018-01-01"), 10.0),
            (d("2026-02-01"), 1.0),
            (d("2026-02-02"), 10.0),
        ];
        let a = bands(&pts, today).unwrap();
        assert_eq!(a.bands, [9.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn leap_day_cuts_land_on_the_28th() {
        assert_eq!(
            years_before(Day::parse("2028-02-29").unwrap(), 1),
            Day {
                y: 2027,
                m: 2,
                d: 28
            }
        );
        assert_eq!(
            years_before(Day::parse("2028-02-29").unwrap(), 4),
            Day {
                y: 2024,
                m: 2,
                d: 29
            }
        );
    }

    #[test]
    fn rules_come_from_the_registry_kinds() {
        assert_eq!(rule_for("generic", Some(365)), Rule::Days(365));
        assert_eq!(rule_for("GENERIC", Some(730)), Rule::Days(730));
        assert_eq!(rule_for("generic", None), Rule::Days(365));
        assert_eq!(rule_for("us", None), Rule::OverOneYear);
        assert_eq!(rule_for("de", Some(365)), Rule::OverOneYear);
        assert_eq!(rule_for("at", None), Rule::Austria);
        assert_eq!(
            rule_for("au", None),
            Rule::OverOneYear,
            "Australia: more than one year"
        );
        assert_eq!(rule_for(" AU ", Some(0)), Rule::OverOneYear);
        for c in ["ch", "gb", "ie", "es", "jp", "nz", "in", "za", "xx"] {
            assert_eq!(rule_for(c, Some(365)), Rule::None, "{c}");
        }
    }

    #[test]
    fn every_registry_country_gets_its_own_kind() {
        use crate::reports::books::model::{COUNTRIES, LongTerm};
        for c in COUNTRIES {
            let r = rule_for(c.code, Some(400));
            let expected = match c.long_term {
                LongTerm::Never if c.code == "at" => Rule::Austria,
                LongTerm::Never => Rule::None,
                LongTerm::MoreThanOneYear => Rule::OverOneYear,
                LongTerm::Days => Rule::Days(400),
            };
            assert_eq!(r, expected, "{}", c.code);
        }
    }

    #[test]
    fn a_zero_day_book_claims_no_count() {
        assert_eq!(rule_for("generic", Some(0)), Rule::Days(0));
        crate::i18n::set_lang(crate::i18n::Lang::En);
        assert_eq!(rule_line(&Rule::Days(0)), None);
        assert!(rule_line(&Rule::Days(730)).unwrap().contains("730 days"));
        assert!(
            rule_line(&rule_for("au", None))
                .unwrap()
                .contains("more than 1 year")
        );
    }
}
