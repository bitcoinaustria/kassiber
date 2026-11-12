use super::*;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

static METRICS: LazyLock<serde_json::Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("source_funds_font_metrics.json")).expect("font metrics")
});
fn list<'a>(row: &'a Value, key: &str) -> &'a [Value] {
    match value(row, key) {
        Value::List(v) => v,
        _ => &[],
    }
}
fn fallback(row: &Value, key: &str) -> String {
    get(row, key)
        .filter(|v| truthy(v))
        .map(py_str)
        .unwrap_or_default()
}
fn number(v: &Value) -> f64 {
    py_str(v).parse().unwrap_or(0.0)
}
fn f(n: f64) -> String {
    kassiber_pyjson::float_repr(n)
}
fn compact(n: f64) -> String {
    format!("{n:.6}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
}
fn escaped(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn width(raw: &str, font: &str, size: f64) -> f64 {
    let metrics = &METRICS[font];
    let default = metrics["default"].as_f64().unwrap();
    let sum: f64 = raw
        .chars()
        .map(|c| {
            metrics["widths"][(c as u32).to_string()]
                .as_f64()
                .unwrap_or(default)
        })
        .sum();
    sum * 0.001 * size
}
fn fit(raw: &str, max: f64, font: &str, size: f64) -> String {
    if raw.is_empty() || width(raw, font, size) <= max {
        return raw.into();
    }
    let available = max - width("...", font, size);
    if available <= 0.0 {
        return "...".into();
    }
    let mut raw = raw.to_owned();
    while !raw.is_empty() && width(&raw, font, size) > available {
        raw.pop();
    }
    format!(
        "{}...",
        raw.trim_end_matches(|c: char| kassiber_pytext::is_space(c as u32))
    )
}
fn rounded(v: &Value, places: u32) -> Result<String> {
    if matches!(v, Value::Null) || v == &s("") {
        return Ok(String::new());
    }
    decimal(v)?
        .to_fixed_places(places, Rounding::HalfEven)
        .map_err(arithmetic)
}
fn color(raw: &str) -> String {
    let channel = |i| u8::from_str_radix(&raw[i..i + 2], 16).unwrap() as f64 / 255.0 * 100.0;
    format!(
        "rgb({}%,{}%,{}%)",
        channel(1) as u8,
        channel(3) as u8,
        channel(5) as u8
    )
}
struct Svg {
    out: String,
}
impl Svg {
    fn new(height: f64) -> Self {
        let h = f(height);
        let compact = compact(height);
        Self{out:format!("<svg width=\"500.0\" height=\"{h}\" preserveAspectRatio=\"xMinYMin meet\" viewBox=\"0 0 500 {compact}\" xmlns=\"http://www.w3.org/2000/svg\" version=\"1.0\" fill-rule=\"evenodd\" xmlns:xlink=\"http://www.w3.org/1999/xlink\">\n\t<title>...</title>\n\t<desc>...</desc>\n\t<clipPath id=\"clip\">\n\t\t<rect x=\"0\" y=\"0\" width=\"500.0\" height=\"{h}\"/>\n\t</clipPath>\n\t<g id=\"group\" transform=\"scale(1,-1) translate(0,-{compact})\" style=\"clip-path: url(#clip)\">\n\t\t<g transform=\"\">\n")}
    }
    fn node(&mut self, node: String) {
        self.out.push_str("\t\t\t");
        self.out.push_str(&node);
        self.out.push('\n');
    }
    fn finish(mut self) -> String {
        self.out.push_str("\t\t</g>\n\t</g>\n</svg>\n");
        self.out
    }
    fn text(
        &mut self,
        position: (f64, f64),
        raw: &str,
        font: &str,
        size: &str,
        ink: &str,
        middle: bool,
    ) {
        let (mut x, y) = position;
        let size_number: f64 = size.parse().unwrap();
        if middle {
            x -= width(raw, font, size_number) / 2.0;
        }
        let (family, weight) = if font == "mono" {
            (
                "'DejaVu Sans Mono', 'Courier New', Courier, monospace",
                "400",
            )
        } else {
            (
                "'DejaVu Sans', Helvetica, Arial, sans-serif",
                if font == "bold" { "700" } else { "400" },
            )
        };
        self.node(format!("<text x=\"{}\" y=\"{}\" style=\"font-family: {family}; font-weight: {weight}; font-size: {size}px;fill: {}; stroke:none;\" transform=\"translate(0,{}) scale(1,-1)\">{}</text>",f(x),f(y),color(ink),(2.0*y)as i64,escaped(raw)));
    }
    fn rect(
        &mut self,
        rect: (f64, f64, f64, f64),
        radius: &str,
        fill: &str,
        stroke: Option<&str>,
        thickness: &str,
        dashed: bool,
    ) {
        let (x, y, w, h) = rect;
        let stroke = stroke.map(color).unwrap_or_else(|| "none".into());
        let dash = if dashed {
            " stroke-dasharray: 2, 2;"
        } else {
            ""
        };
        self.node(format!("<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{radius}\" ry=\"{radius}\" style=\"stroke: {stroke}; stroke-linecap: butt; stroke-width: {thickness}; fill: {}; fill-rule: evenodd;{dash}\"/>",f(x),f(y),f((x+w)-x),f((y+h)-y),color(fill)));
    }
    fn line(
        &mut self,
        start: (f64, f64),
        end: (f64, f64),
        ink: &str,
        thickness: f64,
        dashed: bool,
    ) {
        let dash = if dashed {
            " stroke-dasharray: 2, 2;"
        } else {
            ""
        };
        self.node(format!("<path d=\"M {},{} L {},{} Z\" style=\"stroke: {}; stroke-linecap: butt; stroke-width: {};{dash}\"/>",compact(start.0),compact(start.1),compact(end.0),compact(end.1),color(ink),f(thickness)));
    }
    fn circle(
        &mut self,
        cx: f64,
        cy: f64,
        r: f64,
        fill: &str,
        stroke: Option<&str>,
        thickness: &str,
    ) {
        self.node(format!("<circle cx=\"{}\" cy=\"{}\" r=\"{}\" style=\"stroke: {}; stroke-linecap: butt; stroke-width: {thickness}; fill: {}; fill-rule: evenodd;\"/>",f(cx),f(cy),f(r),stroke.map(color).unwrap_or_else(||"none".into()),color(fill)));
    }
    fn wedge(&mut self, cx: f64, cy: f64, r: f64, start: f64, end: f64, ink: &str) {
        let extent = end - start;
        let radians = |angle: f64| angle * std::f64::consts::PI / 180.0;
        let mx = r * radians(start).cos() + cx;
        let my = r * radians(start).sin() + cy;
        let ax = r * radians(start + extent).cos() + cx;
        let ay = r * radians(start + extent).sin() + cy;
        self.node(format!("<path d=\"M {},{} L {},{} A {},{} 0 {} 0 {},{} L {},{} Z\" style=\"font-family: Times-Roman; font-size: 10px; stroke: {}; stroke-linecap: butt; stroke-width: 0.8; fill: {}; fill-rule: evenodd;\"/>",compact(cx),compact(cy),compact(ax),compact(ay),compact(r),compact(r),u8::from(extent>=180.0),compact(mx),compact(my),compact(cx),compact(cy),color("#ffffff"),color(ink)));
    }
}

fn style(category: &str) -> (&'static str, &'static str, bool) {
    match category {
        "target" => ("#ffffff", "#e3000f", false),
        "source_real" => ("#ecfdf5", "#16a34a", false),
        "source_attest" => ("#fffbeb", "#d97706", true),
        "privacy" => ("#fff7ed", "#ea580c", true),
        _ => ("#ffffff", "#d9d9d9", false),
    }
}
fn privacy(edge: &Value) -> bool {
    truthy(value(edge, "deferred_privacy_hop"))
        || ["coinjoin", "payjoin"].contains(&text(edge, "link_type").as_str())
}
fn swap(edge: &Value) -> bool {
    ["swap", "peg_in", "peg_out", "lightning_swap"].contains(&text(edge, "link_type").as_str())
}
fn omitted(id: String, label: String, kind: &str) -> Value {
    dict([
        ("id", s(id)),
        ("label", s(label)),
        ("kind", s(kind)),
        ("asset", s("")),
        ("amount", Value::Null),
        ("node_type", s("omitted")),
        ("deferred_privacy_hop", Value::Bool(false)),
    ])
}

pub fn flow_svg(flow: &Value, detailed: bool) -> Result<String> {
    let (max_levels, max_nodes) = if detailed { (16, 10) } else { (6, 4) };
    let mut levels = list(flow, "levels").to_vec();
    if levels.len() > max_levels {
        let hidden = levels.len() - (max_levels - 1);
        let head = ((max_levels - 1) / 2 + 1).max(1);
        let tail = max_levels - 1 - head;
        let mut next = levels[..head].to_vec();
        next.push(dict([
            ("role", s("omitted")),
            (
                "nodes",
                Value::List(vec![omitted(
                    "synthetic:omitted".into(),
                    format!("{hidden} path levels omitted"),
                    "continued path",
                )]),
            ),
        ]));
        if tail > 0 {
            next.extend_from_slice(&levels[levels.len() - tail..]);
        }
        levels = next;
    }
    for (index, level) in levels.iter_mut().enumerate() {
        let nodes = list(level, "nodes");
        if nodes.len() > max_nodes {
            let extra = nodes.len() - (max_nodes - 1);
            let mut next = nodes[..max_nodes - 1].to_vec();
            next.push(omitted(
                format!(
                    "synthetic:extra:{}",
                    get(level, "level")
                        .map(py_str)
                        .unwrap_or_else(|| index.to_string())
                ),
                format!("{extra} more items"),
                "additional reviewed nodes",
            ));
            replace(level, "nodes", Value::List(next));
        }
    }
    if levels.is_empty() {
        let mut svg = Svg::new(24.0);
        svg.text(
            (0.0, 8.0),
            "No simplified flow path available.",
            "regular",
            "8",
            "#666666",
            false,
        );
        return Ok(svg.finish());
    }
    let count = levels.len();
    let col_w = (500.0 - 16.0 * (count - 1) as f64) / count as f64;
    let box_w = col_w.max(60.0);
    let max_rows = levels
        .iter()
        .map(|r| list(r, "nodes").len())
        .max()
        .unwrap_or(1);
    let body_h = max_rows as f64 * 40.0 + max_rows.saturating_sub(1) as f64 * 12.0;
    let height = 4.0 + 16.0 + body_h + 12.0 + 4.0;
    let body_top = height - 4.0 - 12.0;
    let mut svg = Svg::new(height);
    let mut positions = BTreeMap::new();
    for (col, level) in levels.iter().enumerate() {
        let nodes = list(level, "nodes");
        let group_h = nodes.len() as f64 * 40.0 + nodes.len().saturating_sub(1) as f64 * 12.0;
        let base = body_top - (body_h - group_h) / 2.0;
        let x = col as f64 * (col_w + 16.0);
        for (row, node) in nodes.iter().enumerate() {
            let id = fallback(node, "id");
            positions.insert(
                if id.is_empty() {
                    format!("synthetic:{col}:{row}")
                } else {
                    id
                },
                (x, base - 40.0 - row as f64 * 52.0, box_w, 40.0),
            );
        }
    }
    let mut categories = BTreeSet::new();
    let mut has_swap = false;
    let mut labels = Vec::new();
    for edge in list(flow, "edges") {
        let (Some(&(fx, fy, fw, fh)), Some(&(tx, ty, tw, th))) = (
            positions.get(&fallback(edge, "from")),
            positions.get(&fallback(edge, "to")),
        ) else {
            continue;
        };
        if fx == tx {
            continue;
        }
        has_swap |= swap(edge);
        let ink = if privacy(edge) {
            "#ea580c"
        } else if swap(edge) {
            "#2563eb"
        } else {
            "#666666"
        };
        let share = number(value(edge, "share_of_target")) / 100.0;
        let thickness = (0.6 + share * 4.0).clamp(0.6, 4.0);
        let from_y = fy + fh / 2.0;
        let to_y = ty + th / 2.0;
        let (start, tip, barb) = if fx > tx {
            (fx, tx + tw, 5.0)
        } else {
            (fx + fw, tx, -5.0)
        };
        svg.line((start, from_y), (tip, to_y), ink, thickness, privacy(edge));
        svg.line((tip, to_y), (tip + barb, to_y + 3.0), ink, thickness, false);
        svg.line((tip, to_y), (tip + barb, to_y - 3.0), ink, thickness, false);
        let percent = rounded(value(edge, "share_of_target"), 1)?;
        if !percent.is_empty() {
            labels.push((
                (start + tip) / 2.0,
                (from_y + to_y) / 2.0,
                format!("{percent}%"),
                ink,
            ));
        }
    }
    for (col, level) in levels.iter().enumerate() {
        let nodes = list(level, "nodes");
        if nodes.is_empty() {
            continue;
        }
        let role = fallback(level, "role");
        let role = if role.is_empty() { "flow" } else { &role };
        if let Some(&(x, _, w, _)) = positions.get(&fallback(&nodes[0], "id")) {
            svg.text(
                (x + w / 2.0, height - 4.0 - 8.0),
                &kassiber_pytext::upper(&role.replace('_', " ")),
                "bold",
                "6.0",
                "#666666",
                true,
            );
        }
        for (row, node) in nodes.iter().enumerate() {
            let id = fallback(node, "id");
            let id = if id.is_empty() {
                format!("synthetic:{col}:{row}")
            } else {
                id
            };
            let Some(&(x, y, w, h)) = positions.get(&id) else {
                continue;
            };
            let deferred = truthy(value(node, "deferred_privacy_hop"));
            let category = if role == "target" {
                "target"
            } else if deferred {
                "privacy"
            } else if text(node, "node_type") == "source" {
                if ["missing_history", "opening_balance_attestation"]
                    .contains(&text(node, "kind").as_str())
                {
                    "source_attest"
                } else {
                    "source_real"
                }
            } else {
                "hop"
            };
            categories.insert(category);
            let (fill, stroke, dashed) = style(category);
            svg.rect(
                (x, y, w, h),
                "4",
                fill,
                Some(stroke),
                if ["target", "privacy", "source_attest"].contains(&category) {
                    "1.1"
                } else {
                    "0.6"
                },
                dashed,
            );
            let label = fit(&fallback(node, "label"), w - 8.0, "bold", 6.6);
            let kind = if deferred {
                "privacy hop deferred".into()
            } else {
                fallback(node, "kind").replace('_', " ")
            };
            let kind = fit(&kind, w - 8.0, "regular", 5.8);
            let amount = if value(node, "amount") != &Value::Null {
                fit(
                    format!(
                        "{} {}",
                        rounded(value(node, "amount"), 8)?,
                        get(node, "asset").map(py_str).unwrap_or_default()
                    )
                    .trim(),
                    w - 8.0,
                    "mono",
                    5.7,
                )
            } else {
                String::new()
            };
            svg.text(
                (x + 4.0, y + h - 11.0),
                &label,
                "bold",
                "6.6",
                "#222222",
                false,
            );
            svg.text(
                (x + 4.0, y + h - 21.0),
                &kind,
                "regular",
                "5.8",
                "#666666",
                false,
            );
            if !amount.is_empty() {
                svg.text((x + 4.0, y + 6.0), &amount, "mono", "5.7", "#222222", false);
            }
        }
    }
    for (x, y, text, ink) in labels {
        svg.text((x, y + 1.0), &text, "bold", "5.4", ink, true);
    }
    let mut x = 0.0;
    for (category, label) in [
        ("target", "Target"),
        ("source_real", "Root source"),
        ("source_attest", "Attestation / missing history"),
        ("hop", "Transaction hop"),
        ("swap", "Swap / peg"),
        ("privacy", "Privacy hop (deferred)"),
    ] {
        if !categories.contains(category) && !(category == "swap" && has_swap) {
            continue;
        }
        let (fill, stroke, _) = if category == "swap" {
            ("#dbeafe", "#2563eb", false)
        } else {
            style(category)
        };
        svg.rect((x, 6.0, 8.0, 8.0), "1.5", fill, Some(stroke), "0.6", false);
        svg.text((x + 11.0, 7.0), label, "regular", "6.0", "#666666", false);
        x += 15.0 + width(label, "regular", 6.0) + 12.0;
    }
    Ok(svg.finish())
}

fn ring_color(key: &str, index: usize) -> &'static str {
    match key {
        "fiat_purchase" | "wallet" | "platform_export" => "#2563eb",
        "prior_exchange_withdrawal" | "exchange" => "#0ea5e9",
        "income" | "import" => "#16a34a",
        "mining" => "#65a30d",
        "gift" => "#a855f7",
        "opening_balance_attestation" | "manual" | "manual_import" => "#d97706",
        "missing_history" => "#9ca3af",
        "unknown" => "#6b7280",
        "blockchain" | "chain_sync" => "#0891b2",
        _ => [
            "#2563eb", "#16a34a", "#d97706", "#a855f7", "#0891b2", "#dc2626",
        ][index % 6],
    }
}
fn ring(segments: &[(String, String, f64)], center_value: &str, center_label: &str) -> String {
    let total: f64 = segments.iter().map(|s| s.2.max(0.0)).sum();
    let legend_h = (segments.len() * 11 + 6) as f64;
    let height = 102.0f64.max(legend_h);
    let mut svg = Svg::new(height);
    let cx = 49.0;
    let cy = height / 2.0;
    let radius = 45.0;
    if total <= 0.0 {
        svg.circle(cx, cy, radius, "#f3f4f6", Some("#d9d9d9"), "0.6");
    } else {
        let mut start = 90.0;
        for (index, row) in segments.iter().enumerate() {
            let value = row.2.max(0.0);
            if value <= 0.0 {
                continue;
            }
            let sweep = value / total * 360.0;
            let end = start - sweep;
            svg.wedge(cx, cy, radius, end, start, ring_color(&row.0, index));
            start = end;
        }
    }
    svg.circle(cx, cy, radius * 0.58, "#ffffff", None, "1");
    if !center_value.is_empty() {
        svg.text(
            (cx, cy + 1.0),
            &fit(center_value, radius * 1.1, "bold", 9.0),
            "bold",
            "9",
            "#222222",
            true,
        );
    }
    if !center_label.is_empty() {
        svg.text(
            (cx, cy - 9.0),
            &fit(center_label, radius * 1.1, "regular", 6.0),
            "regular",
            "6",
            "#666666",
            true,
        );
    }
    let top = cy + legend_h / 2.0 - 8.0;
    for (index, row) in segments.iter().enumerate() {
        let y = top - index as f64 * 11.0;
        let ink = ring_color(&row.0, index);
        svg.rect((104.0, y, 7.0, 7.0), "1.5", ink, None, "1", false);
        let share = if total > 0.0 {
            row.2 / total * 100.0
        } else {
            0.0
        };
        let text = fit(
            &format!("{}  {share:.1}%", row.1.replace('_', " ")),
            378.0,
            "regular",
            6.4,
        );
        svg.text((115.0, y + 0.5), &text, "regular", "6.4", "#222222", false);
    }
    svg.finish()
}

pub fn attach(report: &mut Value) -> Result<()> {
    let mut diagrams = dict::<&str>([]);
    let flow = value(report, "simplified_flow");
    let detail = text(value(report, "report_options"), "diagram_detail") == "detailed";
    if !list(flow, "levels").is_empty() {
        replace(&mut diagrams, "flow_svg", s(flow_svg(flow, detail)?));
    }
    let rows: Vec<_> = list(report, "source_mix")
        .iter()
        .filter(|r| number(value(r, "amount")) > 0.0)
        .collect();
    let assets: BTreeSet<_> = rows.iter().map(|r| fallback(r, "asset")).collect();
    if assets.len() == 1 && !assets.contains("") {
        let mut total = Decimal::zero();
        for row in &rows {
            total = total
                .add(&decimal(value(row, "amount"))?, &DECIMAL)
                .map_err(arithmetic)?;
        }
        let segments = rows
            .iter()
            .map(|r| {
                (
                    text(r, "source_type"),
                    fallback(r, "source_type").replace('_', " "),
                    number(value(r, "amount")),
                )
            })
            .collect::<Vec<_>>();
        let center = total
            .to_fixed_places(8, Rounding::HalfEven)
            .map_err(arithmetic)?;
        replace(
            &mut diagrams,
            "source_mix_ring_svg",
            s(ring(
                &segments,
                &center,
                &format!("{} gross upstream demand", assets.first().unwrap()),
            )),
        );
    }
    let mut segments: Vec<_> = list(report, "data_provenance_summary")
        .iter()
        .filter(|r| number(value(r, "count")) > 0.0)
        .map(|r| {
            let key = fallback(r, "provenance");
            let key = if key.is_empty() {
                "manual_import".into()
            } else {
                key
            };
            let label = fallback(r, "label");
            (key, label, number(value(r, "count")))
        })
        .collect();
    if segments.is_empty() {
        let mut counts = BTreeMap::<String, f64>::new();
        for row in list(report, "data_sources") {
            let key = fallback(row, "kind");
            *counts
                .entry(if key.is_empty() { "other".into() } else { key })
                .or_default() += number(value(row, "transaction_count"));
        }
        segments = counts
            .into_iter()
            .filter(|(_, n)| *n > 0.0)
            .map(|(k, n)| (k.clone(), k.replace('_', " "), n))
            .collect();
    }
    if !segments.is_empty() {
        let total = segments.iter().map(|r| r.2).sum::<f64>() as i64;
        replace(
            &mut diagrams,
            "data_source_ring_svg",
            s(ring(&segments, &total.to_string(), "transactions")),
        );
    }
    replace(report, "diagrams", diagrams);
    Ok(())
}
