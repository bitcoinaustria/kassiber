//! NEW STRINGS — not in the web i18n bundles.
//!
//! The export dialog's redesign (owner request: a short summary preview, a wallet list behind
//! an Edit button) needs a few words the Tauri dialog never shows, so
//! `ui-tauri/src/i18n/locales/*/transactions.json` has no keys for them. Their English and German
//! copy lives here until the bundles gain keys; everything else goes through `t("ns:key")`.

use crate::i18n::{Lang, lang};

fn pick(en: &str, de: &str) -> String {
    match lang() {
        Lang::De => de.to_owned(),
        _ => en.to_owned(),
    }
}

/// `992 rows · 23 columns · Excel workbook`'s middle part.
pub fn columns(n: usize) -> String {
    let f = crate::format::count(n);
    match (lang(), n) {
        (Lang::De, 1) => format!("{f} Spalte"),
        (Lang::De, _) => format!("{f} Spalten"),
        (_, 1) => format!("{f} column"),
        _ => format!("{f} columns"),
    }
}

pub fn show_all_columns() -> String {
    pick("Show all columns", "Alle Spalten anzeigen")
}

pub fn show_summary() -> String {
    pick("Show summary", "Zusammenfassung anzeigen")
}

pub fn value() -> String {
    pick("Value", "Wert")
}

pub fn wallet() -> String {
    pick("Wallet", "Wallet")
}

pub fn direction() -> String {
    pick("Direction", "Richtung")
}

pub fn reference() -> String {
    pick("Reference", "Referenz")
}

pub fn summary_table() -> String {
    pick("Export preview", "Exportvorschau")
}

pub fn edit_wallets() -> String {
    pick("Edit wallets", "Wallets bearbeiten")
}

pub fn timezone_of(zone: &str) -> String {
    pick(&format!("Timezone: {zone}"), &format!("Zeitzone: {zone}"))
}

pub fn invalid_range() -> String {
    pick(
        "Enter two dates as YYYY-MM-DD, the first not after the second.",
        "Gib zwei Daten als JJJJ-MM-TT ein, das erste nicht nach dem zweiten.",
    )
}

pub fn first_rows(n: usize) -> String {
    let f = crate::format::count(n);
    pick(&format!("First {f} rows"), &format!("Erste {f} Zeilen"))
}
