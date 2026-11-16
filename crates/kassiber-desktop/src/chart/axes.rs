//! Axis domains, ticks and scales: model.ts's log / auto-fit helpers, Recharts' nice linear
//! ticks for `[0, auto]` domains, category ticks and the power-law time axis.

use super::data::{DAY_MS, GENESIS_MS, Pt};
use crate::format::Day;

/// `positiveLogDomain`.
pub fn positive_log_domain(values: &[Option<f64>]) -> Option<(f64, f64)> {
    let mut min = f64::INFINITY;
    let mut max = 0.0f64;
    for v in values.iter().flatten() {
        if v.is_finite() && *v > 0.0 {
            min = min.min(*v);
            max = max.max(*v);
        }
    }
    if max <= 0.0 {
        return None;
    }
    if min == max {
        Some((min * 0.9, max * 1.1))
    } else {
        Some((min * 0.96, max * 1.04))
    }
}

/// `autoFitDomain`.
pub fn auto_fit_domain(values: &[Option<f64>]) -> Option<(f64, f64)> {
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for v in values.iter().flatten() {
        if v.is_finite() {
            min = min.min(*v);
            max = max.max(*v);
        }
    }
    if max < min {
        return None;
    }
    if min == max {
        let pad = if min.abs() * 0.05 != 0.0 {
            min.abs() * 0.05
        } else {
            1.0
        };
        return Some(((min - pad).max(0.0), max + pad));
    }
    let pad = (max - min) * 0.06;
    Some(((min - pad).max(0.0), max + pad))
}

#[derive(Clone, Copy)]
enum Edge {
    Low,
    High,
    Mid,
}

fn snap(raw: f64, grid: f64, edge: Edge) -> f64 {
    let q = ((raw / grid) * 1e9).round() / 1e9;
    let s = match edge {
        Edge::Low => q.ceil(),
        Edge::High => q.floor(),
        Edge::Mid => q.round(),
    };
    s * grid
}

fn dedup_sorted(mut v: Vec<f64>) -> Vec<f64> {
    v.sort_by(f64::total_cmp);
    v.dedup_by(|a, b| (*a - *b).abs() <= f64::EPSILON * a.abs().max(1.0));
    v
}

/// `logAxisTicks`.
pub fn log_ticks((lo, hi): (f64, f64), count: usize) -> Vec<f64> {
    if lo <= 0.0 || hi <= lo || count < 2 {
        return vec![];
    }
    let ratio = hi / lo;
    let sig = if ratio >= 1.5 {
        2
    } else if ratio >= 1.05 {
        3
    } else {
        4
    };
    let (llo, lhi) = (lo.log10(), hi.log10());
    let mut out = Vec::new();
    for i in 0..count {
        let raw = 10f64.powf(llo + (lhi - llo) * i as f64 / (count - 1) as f64);
        let grid = 10f64.powf(raw.log10().floor() - (sig - 1) as f64);
        let edge = if i == 0 {
            Edge::Low
        } else if i == count - 1 {
            Edge::High
        } else {
            Edge::Mid
        };
        let r = snap(raw, grid, edge);
        if r >= lo && r <= hi {
            out.push(r);
        }
    }
    dedup_sorted(out)
}

/// `linearAxisTicks`.
pub fn linear_ticks((lo, hi): (f64, f64), count: usize) -> Vec<f64> {
    if !lo.is_finite() || hi <= lo || count < 2 {
        return vec![];
    }
    let step = (hi - lo) / (count - 1) as f64;
    let grid = 10f64.powf(step.log10().floor()) / 2.0;
    let mut out = Vec::new();
    for i in 0..count {
        let raw = lo + step * i as f64;
        let edge = if i == 0 {
            Edge::Low
        } else if i == count - 1 {
            Edge::High
        } else {
            Edge::Mid
        };
        let r = snap(raw, grid, edge);
        if r >= lo && r <= hi {
            out.push(r);
        }
    }
    dedup_sorted(out)
}

/// Recharts' `getFormatStep`.
fn format_step(rough: f64, correction: f64) -> f64 {
    if rough <= 0.0 {
        return 0.0;
    }
    let digits = rough.abs().log10().floor() + 1.0;
    let dv = 10f64.powf(digits);
    let ratio = rough / dv;
    let scale = if digits != 1.0 { 0.05 } else { 0.1 };
    (((ratio / scale) - 1e-12).ceil() + correction) * scale * dv
}

/// Recharts' `getNiceTickValues([min, max], 5)`: the ticks of a `[0, auto]` domain, whose
/// last tick becomes the domain's top.
pub fn nice_ticks(min: f64, max: f64, count: usize) -> Vec<f64> {
    if max <= min || max.is_nan() || min.is_nan() {
        let top = if max > 0.0 { max } else { 1.0 };
        return (0..count)
            .map(|i| min + (top - min) * i as f64 / (count - 1) as f64)
            .collect();
    }
    let mut correction = 0.0;
    loop {
        let step = format_step((max - min) / (count - 1) as f64, correction);
        let middle = if min <= 0.0 && max >= 0.0 {
            0.0
        } else {
            let m = (min + max) / 2.0;
            m - m.rem_euclid(step)
        };
        let mut below = ((middle - min) / step - 1e-12).ceil().max(0.0) as usize;
        let mut up = ((max - middle) / step - 1e-12).ceil().max(0.0) as usize;
        let scale = below + up + 1;
        if scale > count && correction < 40.0 {
            correction += 1.0;
            continue;
        }
        if scale < count {
            if max > 0.0 {
                up += count - scale;
            } else {
                below += count - scale;
            }
        }
        let lo = middle - below as f64 * step;
        return (0..=below + up)
            .map(|i| clean(lo + i as f64 * step))
            .collect();
    }
}

fn clean(v: f64) -> f64 {
    (v * 1e9).round() / 1e9
}

/// A y axis: domain, ticks and whether it is logarithmic.
#[derive(Clone, Debug)]
pub struct YAxis {
    pub lo: f64,
    pub hi: f64,
    pub log: bool,
    pub ticks: Vec<f64>,
    /// The fitted (log or auto-fit) domain the formatter keys precision on.
    pub fit: Option<(f64, f64)>,
}

impl YAxis {
    pub fn build(values: &[Option<f64>], log: bool, fit: bool, expanded: bool) -> YAxis {
        let n = if expanded { 6 } else { 5 };
        if log && let Some(d) = positive_log_domain(values) {
            let t = log_ticks(d, n);
            return YAxis {
                lo: d.0,
                hi: d.1,
                log: true,
                ticks: if t.is_empty() { vec![d.0, d.1] } else { t },
                fit: Some(d),
            };
        }
        if fit && let Some(d) = auto_fit_domain(values) {
            let t = linear_ticks(d, n);
            return YAxis {
                lo: d.0,
                hi: d.1,
                log: false,
                ticks: if t.is_empty() {
                    nice_ticks(d.0, d.1, 5)
                } else {
                    t
                },
                fit: Some(d),
            };
        }
        let max = values
            .iter()
            .flatten()
            .copied()
            .filter(|v| v.is_finite())
            .fold(0.0f64, f64::max);
        let ticks = nice_ticks(0.0, max, 5);
        let hi = ticks.last().copied().filter(|v| *v > 0.0).unwrap_or(1.0);
        YAxis {
            lo: 0.0,
            hi,
            log: false,
            ticks,
            fit: None,
        }
    }

    /// A percent-change axis: the values' own range with 6 % headroom, zero always in view.
    /// Unlike the absolute axes it runs below zero.
    pub fn percent(values: &[Option<f64>], ticks: usize) -> YAxis {
        let (mut lo, mut hi) = (0.0f64, 0.0f64);
        for v in values.iter().flatten().filter(|v| v.is_finite()) {
            lo = lo.min(*v);
            hi = hi.max(*v);
        }
        if hi - lo < 1e-9 {
            (lo, hi) = (lo - 1.0, hi + 1.0);
        }
        let pad = (hi - lo) * 0.06;
        let d = (lo - pad, hi + pad);
        let t = linear_ticks(d, ticks);
        YAxis {
            lo: d.0,
            hi: d.1,
            log: false,
            ticks: if t.is_empty() { vec![d.0, d.1] } else { t },
            fit: Some(d),
        }
    }

    /// The inverse of `y`: the value drawn at `y`.
    pub fn value_at(&self, y: f64, top: f64, bottom: f64) -> f64 {
        let t = (bottom - y) / (bottom - top);
        if self.log {
            10f64.powf(self.lo.log10() + t * (self.hi.log10() - self.lo.log10()))
        } else {
            self.lo + t * (self.hi - self.lo)
        }
    }

    /// Value → y in `[top, bottom]`.
    pub fn y(&self, v: f64, top: f64, bottom: f64) -> f64 {
        let t = if self.log {
            if v <= 0.0 {
                return f64::NAN;
            }
            (v.log10() - self.lo.log10()) / (self.hi.log10() - self.lo.log10())
        } else {
            (v - self.lo) / (self.hi - self.lo)
        };
        bottom - t * (bottom - top)
    }
}

/// A percent-change label: signed, with as many decimals as the axis span needs.
pub fn pct_label(v: f64, span: f64) -> String {
    let decimals = if span >= 20.0 {
        0
    } else if span >= 2.0 {
        1
    } else {
        2
    };
    let mut s = format!("{:.*}", decimals, v.abs());
    if crate::i18n::lang() == crate::i18n::Lang::De {
        s = s.replace('.', ",");
    }
    let zero = s.chars().all(|c| matches!(c, '0' | '.' | ','));
    let sign = if zero {
        ""
    } else if v > 0.0 {
        "+"
    } else {
        "\u{2212}"
    };
    format!("{sign}{s}%")
}

/// The x axis: a linear time axis over the plotted points' moments (so a stretch without
/// balance points keeps its calendar width and the ticks fall evenly), or a log axis of days
/// since the genesis block.
#[derive(Clone, Debug)]
pub enum XAxis {
    Time { lo: f64, hi: f64 },
    PowerLaw { lo: f64, hi: f64 },
}

impl XAxis {
    pub fn x_ms(&self, ms: f64, left: f64, right: f64) -> f64 {
        match self {
            XAxis::Time { lo, hi } => {
                if hi <= lo {
                    return (left + right) / 2.0;
                }
                left + (ms - lo) / (hi - lo) * (right - left)
            }
            XAxis::PowerLaw { .. } => self.x_days(super::data::power_days(ms), left, right),
        }
    }
    pub fn x_days(&self, days: f64, left: f64, right: f64) -> f64 {
        match self {
            XAxis::PowerLaw { lo, hi } => {
                left + (days.log10() - lo.log10()) / (hi.log10() - lo.log10()) * (right - left)
            }
            XAxis::Time { .. } => left,
        }
    }
    /// A point's x.
    pub fn x_pt(&self, p: &Pt, left: f64, right: f64) -> f64 {
        match self {
            XAxis::Time { .. } => self.x_ms(p.sort_ms, left, right),
            XAxis::PowerLaw { .. } => self.x_days(p.power_days, left, right),
        }
    }
}

/// A calendar tick interval.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Days(i64),
    Months(i32),
    Years(i32),
}

impl Step {
    const ALL: [Step; 14] = [
        Step::Days(1),
        Step::Days(2),
        Step::Days(7),
        Step::Days(14),
        Step::Months(1),
        Step::Months(2),
        Step::Months(3),
        Step::Months(6),
        Step::Years(1),
        Step::Years(2),
        Step::Years(5),
        Step::Years(10),
        Step::Years(20),
        Step::Years(50),
    ];
    fn approx_ms(self) -> f64 {
        match self {
            Step::Days(n) => n as f64 * DAY_MS,
            Step::Months(n) => n as f64 * 30.436_875 * DAY_MS,
            Step::Years(n) => n as f64 * 365.2425 * DAY_MS,
        }
    }
    /// The tick days in `[lo_day, hi_day]`: whole days, weeks on Mondays, months on the 1st
    /// (multiples of the step from January), years on 1 January (multiples of the step).
    fn days(self, lo_day: i64, hi_day: i64) -> Vec<i64> {
        let mut out = Vec::new();
        match self {
            Step::Days(n) => {
                // 1970-01-05 was a Monday.
                let anchor = if n % 7 == 0 { 4 } else { 0 };
                let mut d = lo_day + (anchor - lo_day).rem_euclid(n);
                while d <= hi_day {
                    out.push(d);
                    d += n;
                }
            }
            Step::Months(n) => {
                let first = Day::from_epoch_days(lo_day);
                let (mut y, mut m) = (first.y, first.m as i32);
                loop {
                    if (m - 1) % n == 0 {
                        let d = Day {
                            y,
                            m: m as u32,
                            d: 1,
                        }
                        .epoch_days();
                        if d > hi_day {
                            break;
                        }
                        if d >= lo_day {
                            out.push(d);
                        }
                    }
                    m += 1;
                    if m > 12 {
                        m = 1;
                        y += 1;
                    }
                    if y > first.y + 400 {
                        break;
                    }
                }
            }
            Step::Years(n) => {
                let first = Day::from_epoch_days(lo_day).y;
                let last = Day::from_epoch_days(hi_day).y;
                for y in first..=last {
                    if y.rem_euclid(n) != 0 {
                        continue;
                    }
                    let d = Day { y, m: 1, d: 1 }.epoch_days();
                    if d >= lo_day && d <= hi_day {
                        out.push(d);
                    }
                }
            }
        }
        out
    }
    fn label(self, d: Day) -> String {
        match self {
            Step::Days(_) => d.month_day_ui(),
            Step::Months(_) => d.month_year_ui(),
            Step::Years(_) => d.y.to_string(),
        }
    }
}

/// Evenly spaced calendar ticks over `[lo, hi]` (ms) on a `px`-wide axis: the finest interval
/// that keeps at most `max` ticks and room for every label, each tick at the end of its day,
/// where that day's balance point sits.
pub fn time_ticks(lo: f64, hi: f64, px: f32, max: usize) -> (Step, Vec<(f64, String)>) {
    if hi <= lo || !lo.is_finite() || !hi.is_finite() {
        return (Step::Days(1), vec![]);
    }
    let lo_day = (lo / DAY_MS).floor() as i64 - 1;
    let hi_day = (hi / DAY_MS).floor() as i64;
    let at = |d: i64| d as f64 * DAY_MS + DAY_MS - 1.0;
    let per_ms = px as f64 / (hi - lo);
    for step in Step::ALL {
        let label_px = label_w(
            &step.label(Day {
                y: 2026,
                m: 12,
                d: 28,
            }),
            10.0,
        ) as f64;
        if step.approx_ms() * per_ms < label_px + 14.0 {
            continue;
        }
        let days: Vec<i64> = step
            .days(lo_day, hi_day)
            .into_iter()
            .filter(|d| at(*d) >= lo - 0.5 && at(*d) <= hi + 0.5)
            .collect();
        if days.len() > max {
            continue;
        }
        return (
            step,
            days.into_iter()
                .map(|d| (at(d), step.label(Day::from_epoch_days(d))))
                .collect(),
        );
    }
    (Step::Years(50), vec![])
}

/// `powerLawXDomain`.
pub fn power_law_domain(points: &[Pt]) -> Option<(f64, f64)> {
    let days: Vec<f64> = points
        .iter()
        .map(|p| p.power_days)
        .filter(|d| d.is_finite() && *d > 0.0)
        .collect();
    if days.len() < 2 {
        return None;
    }
    let lo = days.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = days.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    (hi > lo).then_some((lo, hi))
}

fn day_of_power(days: f64) -> Day {
    Day::from_epoch_days(((GENESIS_MS + days * DAY_MS) / DAY_MS).floor() as i64)
}

/// `powerLawXTicks` with labels (`formatPowerLawTick`).
pub fn power_law_ticks((lo, hi): (f64, f64), max: usize) -> Vec<(f64, String)> {
    let gap = (hi.log10() - lo.log10()) / max.max(1) as f64;
    let mut out: Vec<f64> = Vec::new();
    for year in day_of_power(lo).y..=day_of_power(hi).y {
        let d = (Day {
            y: year,
            m: 1,
            d: 1,
        }
        .epoch_days() as f64
            * DAY_MS
            - GENESIS_MS)
            / DAY_MS;
        if d < lo || d > hi {
            continue;
        }
        if let Some(prev) = out.last()
            && d.log10() - prev.log10() < gap
        {
            continue;
        }
        out.push(d);
    }
    if out.len() < 2 {
        out = vec![lo, hi];
    }
    out.into_iter()
        .map(|d| {
            let day = day_of_power(d);
            let label = if day.m == 1 && day.d <= 2 {
                day.y.to_string()
            } else {
                day.month_year_ui()
            };
            (d, label)
        })
        .collect()
}

/// Rough label width at 10 px (Recharts' minTickGap uses measured widths; we estimate).
pub fn label_w(s: &str, size: f32) -> f32 {
    s.chars().count() as f32 * size * 0.56
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recharts_nice_ticks() {
        assert_eq!(
            nice_ticks(0.0, 98.58, 5),
            vec![0.0, 25.0, 50.0, 75.0, 100.0]
        );
        assert_eq!(
            nice_ticks(0.0, 90_000.0, 5),
            vec![0.0, 25_000.0, 50_000.0, 75_000.0, 100_000.0]
        );
        assert_eq!(nice_ticks(0.0, 3.2, 5), vec![0.0, 0.8, 1.6, 2.4, 3.2]);
    }

    #[test]
    fn percent_axis_spans_zero_and_inverts() {
        let a = YAxis::percent(&[Some(0.0), Some(-12.0), Some(40.0), None], 5);
        assert!(a.lo < -12.0 && a.hi > 40.0 && !a.log);
        assert!(a.ticks.iter().all(|t| *t >= a.lo && *t <= a.hi) && a.ticks.len() >= 3);
        // Flat data still gets a usable domain around zero.
        let flat = YAxis::percent(&[Some(0.0), Some(0.0)], 5);
        assert!(flat.lo < 0.0 && flat.hi > 0.0);
        for axis in [
            a,
            YAxis::build(&[Some(2.0), Some(900.0)], true, false, false),
            YAxis::build(&[Some(2.0), Some(9.0)], false, false, false),
        ] {
            for v in [3.0, 7.5] {
                let y = axis.y(v, 12.0, 300.0);
                assert!((axis.value_at(y, 12.0, 300.0) - v).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn percent_labels() {
        assert_eq!(pct_label(12.4, 80.0), "+12%");
        assert_eq!(pct_label(-3.26, 10.0), "\u{2212}3.3%");
        assert_eq!(pct_label(0.0, 10.0), "0.0%");
        assert_eq!(pct_label(0.004, 1.0), "0.00%");
    }

    fn ms(y: i32, m: u32, d: u32) -> f64 {
        Day { y, m, d }.epoch_days() as f64 * DAY_MS + DAY_MS - 1.0
    }

    /// Every gap between neighbouring ticks is the same calendar step.
    fn even(step: Step, ticks: &[(f64, String)]) -> bool {
        ticks.windows(2).all(|w| {
            let (a, b) = (
                Day::from_epoch_days((w[0].0 / DAY_MS).floor() as i64),
                Day::from_epoch_days((w[1].0 / DAY_MS).floor() as i64),
            );
            match step {
                Step::Days(n) => b.epoch_days() - a.epoch_days() == n,
                Step::Months(n) => {
                    a.d == 1 && b.d == 1 && (b.y * 12 + b.m as i32) - (a.y * 12 + a.m as i32) == n
                }
                Step::Years(n) => a.m == 1 && a.d == 1 && b.y - a.y == n,
            }
        })
    }

    #[test]
    fn time_ticks_are_even_calendar_steps() {
        // The demo book's 1Y window, whose points stop in March and resume in November: the
        // category axis read "Nov 25, Dec 25, Jan 26, Feb 26, Mar 26, Nov 26".
        let (step, t) = time_ticks(ms(2025, 11, 13), ms(2026, 11, 13), 620.0, 8);
        assert_eq!(step, Step::Months(2));
        assert!(even(step, &t));
        let labels: Vec<&str> = t.iter().map(|(_, l)| l.as_str()).collect();
        assert_eq!(
            labels,
            ["Jan 26", "Mar 26", "May 26", "Jul 26", "Sep 26", "Nov 26"]
        );
        // Wider plots take finer steps, up to the tick cap.
        let (step, t) = time_ticks(ms(2025, 11, 13), ms(2026, 11, 13), 1300.0, 10);
        assert!(
            matches!(step, Step::Months(1) | Step::Months(2)) && t.len() <= 10 && even(step, &t)
        );
        // 30 days: weekly, on Mondays.
        let (step, t) = time_ticks(ms(2026, 10, 14), ms(2026, 11, 13), 620.0, 8);
        assert_eq!(step, Step::Days(7));
        assert!(even(step, &t));
        assert!(
            t.iter()
                .all(|(m, _)| (((*m / DAY_MS).floor() as i64) - 4).rem_euclid(7) == 0)
        );
        // Seven years: whole years.
        let (step, t) = time_ticks(ms(2019, 1, 14), ms(2026, 11, 13), 620.0, 8);
        assert_eq!(step, Step::Years(1));
        assert_eq!(t.first().unwrap().1, "2020");
        assert!(even(step, &t) && t.len() == 7);
        // Every tick lies inside the window, at every zoom.
        for days in [8.0, 20.0, 45.0, 100.0, 200.0, 400.0, 900.0, 2000.0, 9000.0] {
            let hi = ms(2026, 11, 13);
            let lo = hi - days * DAY_MS;
            for px in [300.0, 620.0, 1300.0] {
                let (step, t) = time_ticks(lo, hi, px, 8);
                assert!(!t.is_empty() && t.len() <= 8, "{days} days at {px}: {t:?}");
                assert!(t.iter().all(|(m, _)| *m >= lo && *m <= hi));
                assert!(even(step, &t), "{days} days at {px}: {step:?} {t:?}");
            }
        }
    }

    #[test]
    fn time_axis_maps_linearly() {
        let a = XAxis::Time { lo: 0.0, hi: 100.0 };
        assert_eq!(a.x_ms(0.0, 10.0, 110.0), 10.0);
        assert_eq!(a.x_ms(50.0, 10.0, 110.0), 60.0);
        assert_eq!(XAxis::Time { lo: 5.0, hi: 5.0 }.x_ms(5.0, 0.0, 100.0), 50.0);
    }

    #[test]
    fn log_and_linear_ticks() {
        let t = log_ticks((58_000.0, 84_000.0), 5);
        assert!(t.len() >= 4 && t[0] >= 58_000.0 && *t.last().unwrap() <= 84_000.0);
        let l = linear_ticks((40.1, 41.3), 5);
        assert!(l.first().unwrap() >= &40.1 && l.last().unwrap() <= &41.3);
    }
}
