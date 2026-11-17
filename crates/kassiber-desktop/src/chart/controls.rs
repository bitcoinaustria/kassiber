//! Activity-dot minimum helpers. The web's "More settings…" side sheet (ChartControlsSheet.tsx)
//! is not ported: its settings live in the gear menu.

/// `clampActivityMarkerMinimum`: ≥ 0, on the 1e-8 grid.
pub fn clamp_min(v: f64) -> f64 {
    if !v.is_finite() {
        return 0.0;
    }
    (v.max(0.0) / 1e-8).round() * 1e-8
}

/// `serializeActivityMarkerMinimum`: up to 8 decimals, trailing zeros trimmed.
pub fn editable(v: f64) -> String {
    let s = format!("{:.8}", clamp_min(v));
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() {
        "0".into()
    } else {
        s.to_owned()
    }
}
