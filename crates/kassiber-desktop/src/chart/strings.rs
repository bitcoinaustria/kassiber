//! NEW STRINGS — not in the web i18n bundles.
//!
//! The zoom, crosshair, measure and percent-scale features have no counterpart in the Tauri
//! app, so `ui-tauri/src/i18n/locales/*/overview.json` has no keys for them. Their English and
//! German copy lives here until the bundles gain keys; everything else in the chart goes
//! through `t("ns:key")`.

use crate::i18n::{Lang, lang};

fn pick(en: &'static str, de: &'static str) -> String {
    match lang() {
        Lang::De => de.to_owned(),
        _ => en.to_owned(),
    }
}

pub fn reset_zoom() -> String {
    pick("Reset zoom", "Zoom zurücksetzen")
}
pub fn tools_group() -> String {
    pick("Zoom and tools", "Zoom und Werkzeuge")
}
pub fn crosshair() -> String {
    pick("Crosshair", "Fadenkreuz")
}
pub fn measure() -> String {
    pick("Measure", "Messen")
}
pub fn measure_shortcut() -> String {
    pick("⇧ drag", "⇧ ziehen")
}
pub fn percent_chip() -> String {
    "%".to_owned()
}
pub fn percent_title() -> String {
    pick(
        "Percent change from the start of the visible range",
        "Prozentuale Veränderung seit Beginn des sichtbaren Bereichs",
    )
}
pub fn plot_label() -> String {
    pick("Chart area", "Diagrammfläche")
}
pub fn plot_keys() -> String {
    pick(
        "Drag to zoom into a time range, Shift-drag to measure, Option-drag or the arrow keys to pan. Plus and minus zoom, 0 resets the zoom, Escape cancels.",
        "Ziehen zoomt in einen Zeitraum, Umschalt-Ziehen misst, Wahltaste-Ziehen oder die Pfeiltasten verschieben. Plus und Minus zoomen, 0 setzt den Zoom zurück, Escape bricht ab.",
    )
}
pub fn days(n: i64) -> String {
    match (lang(), n) {
        (Lang::De, 1) => "1 Tag".to_owned(),
        (Lang::De, n) => format!("{n} Tage"),
        (_, 1) => "1 day".to_owned(),
        (_, n) => format!("{n} days"),
    }
}
pub fn measure_balance() -> String {
    pick("Balance", "Bestand")
}
pub fn measure_value() -> String {
    pick("Value", "Wert")
}
pub fn measure_hint() -> String {
    pick(
        "Drag across the chart to measure",
        "Zum Messen über das Diagramm ziehen",
    )
}
/// The brush handles' accessible names (Recharts names them "Min value: undefined, …").
pub fn brush_start() -> String {
    pick("Timeline start", "Beginn des Zeitfensters")
}
pub fn brush_end() -> String {
    pick("Timeline end", "Ende des Zeitfensters")
}
pub fn brush_window() -> String {
    pick("Timeline window", "Zeitfenster")
}
