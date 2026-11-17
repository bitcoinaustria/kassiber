//! NEW STRINGS — not in the web i18n bundles.
//!
//! The transfer route card (route.rs) is an owner-requested redesign with no counterpart in the
//! Tauri app: the gap between two linked transactions, the time between them and the note that
//! a link is the owner's own assertion. Their English and German copy lives here until the
//! bundles gain keys; everything else on the card reuses existing keys.

use crate::i18n::{Lang, lang};

fn pick(en: String, de: String) -> String {
    match lang() {
        Lang::De => de,
        _ => en,
    }
}

/// The gap node between two linked transactions.
pub fn not_recorded() -> String {
    pick(
        "Not recorded in this book".into(),
        "Nicht in diesem Buch erfasst".into(),
    )
}

/// The gap node's hover help.
pub fn not_recorded_help() -> String {
    pick(
        "No transaction in this book connects the two, for example a wallet that isn't connected.".into(),
        "Keine Transaktion in diesem Buch verbindet die beiden, zum Beispiel eine nicht verbundene Wallet.".into(),
    )
}

/// Under an owner-made link: what the link is.
pub fn owner_caveat() -> String {
    pick(
        "Your link, not chain evidence.".into(),
        "Deine Verknüpfung, kein Chain-Nachweis.".into(),
    )
}

/// The segmented control's caption: which leg the graph below shows.
pub fn graph_of() -> String {
    pick("Graph".into(), "Graph".into())
}

/// "3 days later": the time from the send to the receipt.
pub fn later(secs: f64) -> String {
    let d = duration(secs);
    pick(format!("{d} later"), format!("{d} später"))
}

/// A coarse duration: minutes, hours, days, months or years.
pub fn duration(secs: f64) -> String {
    let secs = secs.max(0.0);
    let (n, en1, en, de1, de) = if secs < 3600.0 {
        ((secs / 60.0).round().max(1.0), "min", "min", "Min.", "Min.")
    } else if secs < 86_400.0 {
        (
            (secs / 3600.0).floor(),
            "hour",
            "hours",
            "Stunde",
            "Stunden",
        )
    } else if secs < 60.0 * 86_400.0 {
        ((secs / 86_400.0).floor(), "day", "days", "Tag", "Tage")
    } else if secs < 730.0 * 86_400.0 {
        (
            (secs / (30.44 * 86_400.0)).floor(),
            "month",
            "months",
            "Monat",
            "Monate",
        )
    } else {
        (
            (secs / (365.25 * 86_400.0)).floor(),
            "year",
            "years",
            "Jahr",
            "Jahre",
        )
    };
    let n = n as u64;
    pick(
        format!("{n} {}", if n == 1 { en1 } else { en }),
        format!("{n} {}", if n == 1 { de1 } else { de }),
    )
}

#[cfg(test)]
mod tests {
    use super::duration;

    #[test]
    fn durations_are_coarse() {
        assert_eq!(duration(30.0), "1 min");
        assert_eq!(duration(68.0 * 60.0), "1 hour");
        assert_eq!(duration(3.0 * 86_400.0 + 5.0), "3 days");
        assert_eq!(duration(208.0 * 86_400.0), "6 months");
        assert_eq!(duration(800.0 * 86_400.0), "2 years");
    }
}
