//! Strand widths and positions after mempool's bowtie graph, shared by the 2.5D glass view and
//! the flat fallback (TransactionGraphGeometry.ts, exactly).

use super::model::Row;

const MIN_WEIGHT: f64 = 2.0;
const MIN_SPACING: f64 = 4.0;

/// `legValue`: `None` when unknown (confidential, missing, another asset).
pub fn leg_value(r: &Row) -> Option<f64> {
    let n = &r.node;
    if matches!(n.state(), "confidential" | "other_asset") {
        return None;
    }
    n.value_sats.map(|v| v.max(0.0))
}

fn leg_count(r: &Row) -> f64 {
    if r.node.overflow {
        r.node.overflow_count.unwrap_or(1.0).max(1.0)
    } else {
        1.0
    }
}

fn known_total(rows: &[Row]) -> f64 {
    rows.iter().map(|r| leg_value(r).unwrap_or(0.0)).sum()
}

fn unknown_legs(rows: &[Row]) -> f64 {
    rows.iter()
        .map(|r| {
            if leg_value(r).is_none() {
                leg_count(r)
            } else {
                0.0
            }
        })
        .sum()
}

/// `bowtieTotal`.
pub fn total(inputs: &[Row], destinations: &[Row], liquid: bool) -> f64 {
    let out = known_total(destinations);
    if !liquid {
        return out;
    }
    let inp = known_total(inputs);
    let (ui, uo) = (unknown_legs(inputs), unknown_legs(destinations));
    if ui > 0.0 && uo > 0.0 {
        let in_legs: f64 = inputs.iter().map(leg_count).sum();
        let out_legs: f64 = destinations.iter().map(leg_count).sum();
        let kin = if in_legs - ui == 0.0 {
            1.0
        } else {
            in_legs - ui
        };
        let kout = if out_legs - uo == 0.0 {
            1.0
        } else {
            out_legs - uo
        };
        return (inp + inp / kin * ui).max(out + out / kout * uo);
    }
    inp.max(out)
}

/// `bowtieWeights`.
pub fn weights(rows: &[Row], total: f64, cw: f64) -> Vec<f64> {
    if total == 0.0 {
        return rows
            .iter()
            .map(|_| cw / (rows.len().max(1) as f64))
            .collect();
    }
    let unknown = rows.iter().filter(|r| leg_value(r).is_none()).count();
    let share = if unknown > 0 {
        (total - known_total(rows)) / unknown as f64
    } else {
        0.0
    };
    rows.iter()
        .map(|r| (cw * leg_value(r).unwrap_or(share) / total).max(0.0))
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Line {
    pub outer_y: f64,
    pub inner_y: f64,
    pub thickness: f64,
    pub weight: f64,
    pub offset: f64,
    pub curve_base: f64,
    pub zero: bool,
    pub estimated: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Opts {
    pub height: f64,
    pub combined_weight: f64,
    pub curve_width: f64,
    pub outer_top: f64,
    pub outer_span: f64,
    pub zero_thickness: f64,
}

fn thickness(v: Option<f64>, w: f64, cw: f64, zero: f64) -> f64 {
    if v == Some(0.0) {
        zero
    } else {
        (cw + 0.5).min((MIN_WEIGHT - 1.0).max(w) + 1.0)
    }
}

/// `bowtieLines`.
pub fn lines(rows: &[Row], total: f64, o: Opts) -> Vec<Line> {
    if rows.is_empty() {
        return vec![];
    }
    let ws = weights(rows, total, o.combined_weight);
    let mut lines: Vec<Line> = rows
        .iter()
        .zip(&ws)
        .map(|(r, &w)| {
            let v = leg_value(r);
            Line {
                outer_y: o.height / 2.0,
                inner_y: o.height / 2.0,
                thickness: thickness(v, w, o.combined_weight, o.zero_thickness),
                weight: w,
                offset: 0.0,
                curve_base: 0.0,
                zero: v == Some(0.0),
                estimated: v.is_none(),
            }
        })
        .collect();
    let visible: f64 = lines.iter().map(|l| l.thickness).sum();
    let n = lines.len();
    let spacing = if n <= 1 {
        0.0
    } else {
        MIN_SPACING.max((o.outer_span - visible) / (n as f64 - 1.0))
    };
    let inner_top = o.height / 2.0 - o.combined_weight / 2.0;
    let inner_bottom = inner_top + o.combined_weight + 0.5;
    let (mut last_outer, mut last_inner) = (o.outer_top, inner_top);
    let (mut offset, mut min_off, mut max_off, mut last_w, mut pad) =
        (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for l in &mut lines {
        l.outer_y = if n == 1 {
            o.height / 2.0
        } else {
            last_outer + l.thickness / 2.0
        };
        if l.zero {
            last_outer += l.thickness + spacing;
            continue;
        }
        l.inner_y = (inner_bottom - l.thickness / 2.0)
            .min((inner_top + l.thickness / 2.0).max(last_inner + l.weight / 2.0));
        last_outer += l.thickness + spacing;
        last_inner += l.weight;
        let t = (last_w + l.weight) / 2.0;
        let dx = (0.75 * o.curve_width).max(1.0);
        let dy = 1.5 * (l.inner_y - l.outer_y);
        let a = dy.atan2(dx);
        if a.sin() != 0.0 {
            offset += (t * (1.0 - a.cos()) / a.sin()).min(t).max(-t);
        }
        l.offset = offset;
        min_off = min_off.min(offset);
        max_off = max_off.max(offset);
        pad = pad.max(l.thickness / 2.0);
        last_w = l.weight;
    }
    for l in &mut lines {
        l.offset -= min_off;
        l.curve_base = pad + (max_off - min_off);
    }
    lines
}

/// `bowtieMinimumSpan`.
pub fn minimum_span(rows: &[Row], total: f64, cw: f64, zero: f64) -> f64 {
    let ws = weights(rows, total, cw);
    let th: f64 = rows
        .iter()
        .zip(&ws)
        .map(|(r, &w)| thickness(leg_value(r), w, cw, zero))
        .sum();
    th + MIN_SPACING * (rows.len().saturating_sub(1) as f64)
}

/// A cubic Bézier coordinate.
pub fn bezier(p0: f64, p1: f64, p2: f64, p3: f64, t: f64) -> f64 {
    let u = 1.0 - t;
    u * u * u * p0 + 3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t * p3
}

#[cfg(test)]
mod tests {
    use super::super::data::Node;
    use super::super::model::Side;
    use super::*;

    fn row(v: Option<f64>) -> Row {
        Row {
            node: Node {
                value_sats: v,
                value_state: Some(if v.is_some() { "known" } else { "missing" }.into()),
                ..Default::default()
            },
            side: Side::Input,
        }
    }

    #[test]
    fn weights_split_the_band_and_unknown_legs_share_the_rest() {
        let rows = vec![row(Some(30.0)), row(None), row(None)];
        let w = weights(&rows, 100.0, 100.0);
        assert_eq!(w, vec![30.0, 35.0, 35.0]);
        assert_eq!(weights(&rows, 0.0, 90.0), vec![30.0, 30.0, 30.0]);
    }

    #[test]
    fn lines_match_the_typescript_reference() {
        // Values computed with TransactionGraphGeometry.ts for the same inputs.
        let rows = vec![row(Some(75.0)), row(Some(25.0))];
        let o = Opts {
            height: 280.0,
            combined_weight: 82.0,
            curve_width: 400.0,
            outer_top: 40.0,
            outer_span: 200.0,
            zero_thickness: 3.0,
        };
        let l = lines(&rows, 100.0, o);
        assert!((l[0].thickness - 62.5).abs() < 1e-9);
        assert!((l[1].thickness - 21.5).abs() < 1e-9);
        assert!((l[0].outer_y - 71.25).abs() < 1e-9);
        // innerTop + th/2 = 99 + 31.25 wins over lastInner + w/2 = 99 + 30.75.
        assert!((l[0].inner_y - 130.25).abs() < 1e-9);
        assert!(l.iter().all(|x| x.offset >= 0.0));
        let single = lines(&rows[..1], 75.0, o);
        assert_eq!(single[0].outer_y, 140.0);
        assert!((minimum_span(&rows, 100.0, 82.0, 3.0) - (62.5 + 21.5 + 4.0)).abs() < 1e-9);
    }

    #[test]
    fn liquid_total_extrapolates_unknown_legs() {
        let inputs = vec![row(Some(10.0)), row(None)];
        let outputs = vec![row(Some(4.0)), row(None), row(None)];
        assert_eq!(
            total(&inputs, &outputs, true),
            (10.0f64 + 10.0).max(4.0 + 4.0 * 2.0)
        );
        assert_eq!(total(&inputs, &outputs, false), 4.0);
    }
}
