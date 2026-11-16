//! NEW STRINGS — not in the web i18n bundles.
//!
//! "Link several" (owner request: link one send to several receipts, or several sends to one
//! receipt, by hand) is not in the Tauri dialog, so `review.json` has no keys for it. Its
//! English and Austrian German copy lives here until the bundles gain keys; everything the
//! one-to-one flow already says goes through `t("review:transfersPage.link.*")`.

use crate::i18n::{Lang, lang};

fn pick(en: &str, de: &str) -> String {
    match lang() {
        Lang::De => de.to_owned(),
        _ => en.to_owned(),
    }
}

pub fn toggle() -> String {
    pick("Link several at once", "Mehrere auf einmal verknüpfen")
}

pub fn toggle_hint() -> String {
    pick(
        "One send split into several receipts, or several sends collected into one receipt.",
        "Ein Ausgang, aufgeteilt auf mehrere Eingänge, oder mehrere Ausgänge, gesammelt in einem Eingang.",
    )
}

pub fn start_from() -> String {
    pick("Start from", "Ausgehend von")
}

pub fn one_send() -> String {
    pick("One send", "Einem Ausgang")
}

pub fn one_send_hint() -> String {
    pick(
        "Split into several receipts",
        "Aufgeteilt auf mehrere Eingänge",
    )
}

pub fn one_receipt() -> String {
    pick("One receipt", "Einem Eingang")
}

pub fn one_receipt_hint() -> String {
    pick(
        "Collected from several sends",
        "Gesammelt aus mehreren Ausgängen",
    )
}

pub fn title() -> String {
    pick(
        "Link several transactions",
        "Mehrere Transaktionen verknüpfen",
    )
}

pub fn subtitle() -> String {
    pick(
        "Choose the one transaction, then tick every transaction on the other side. Nothing is linked until you confirm.",
        "Wähle die eine Transaktion und hake dann alle Transaktionen auf der anderen Seite an. Verknüpft wird erst, wenn du bestätigst.",
    )
}

pub fn subtitle_check() -> String {
    pick(
        "Check the links and what changes. Nothing is linked until you confirm.",
        "Prüfe die Verknüpfungen und was sich ändert. Verknüpft wird erst, wenn du bestätigst.",
    )
}

pub fn slot_tick(receipts: bool) -> String {
    if receipts {
        pick(
            "Tick the receipts in the list below",
            "Hake die Eingänge in der Liste unten an",
        )
    } else {
        pick(
            "Tick the sends in the list below",
            "Hake die Ausgänge in der Liste unten an",
        )
    }
}

pub fn ticked(n: usize) -> String {
    match lang() {
        Lang::De => format!("{n} angehakt"),
        _ => format!("{n} ticked"),
    }
}

pub fn several_label(receipts: bool) -> String {
    if receipts {
        pick("Receipts to link", "Zu verknüpfende Eingänge")
    } else {
        pick("Sends to link", "Zu verknüpfende Ausgänge")
    }
}

/// A row that cannot be ticked: why.
pub fn only_one_to_one() -> String {
    pick(
        "Swap or other asset: links one to one only",
        "Swap oder anderes Asset: nur eins zu eins verknüpfbar",
    )
}

pub fn swaps_note() -> String {
    pick(
        "Several at once works within one asset on one network. Swaps and peg-ins link one to one.",
        "Mehrere auf einmal geht innerhalb eines Assets in einem Netzwerk. Swaps und Peg-ins werden eins zu eins verknüpft.",
    )
}

pub fn diff_none() -> String {
    pick(
        "Sent and received match exactly.",
        "Gesendet und erhalten stimmen genau überein.",
    )
}

pub fn diff_fee(fee: &str) -> String {
    match lang() {
        Lang::De => {
            format!("Alles Gesendete kam an. Die Netzwerkgebühr ({fee}) wird als Gebühr gebucht.")
        }
        _ => format!("Everything sent arrived. The network fee ({fee}) is booked as a fee."),
    }
}

pub fn diff_rest(fee: Option<&str>, rest: &str) -> String {
    match (lang(), fee) {
        (Lang::De, Some(f)) => format!(
            "{rest} bleibt unverknüpft und wird zur Prüfung zurückgehalten, bis du sie erklärst. Die Netzwerkgebühr ({f}) wird als Gebühr gebucht."
        ),
        (Lang::De, None) => format!(
            "{rest} bleibt unverknüpft und wird zur Prüfung zurückgehalten, bis du sie erklärst."
        ),
        (_, Some(f)) => format!(
            "{rest} is left unlinked and held for review until you explain it. The network fee ({f}) is booked as a fee."
        ),
        (_, None) => format!("{rest} is left unlinked and held for review until you explain it."),
    }
}

pub fn diff_more(amount: &str) -> String {
    match lang() {
        Lang::De => format!(
            "Es kam {amount} mehr an, als gesendet wurde. Das kann die Verknüpfung nicht erklären: Prüfe die angehakten Transaktionen."
        ),
        _ => format!(
            "{amount} more arrived than was sent. A link can't explain that: check the ticked transactions."
        ),
    }
}

pub fn review(n: usize) -> String {
    match (lang(), n) {
        (Lang::De, 1) => "1 Verknüpfung prüfen".to_owned(),
        (Lang::De, _) => format!("{n} Verknüpfungen prüfen"),
        (_, 1) => "Review 1 link".to_owned(),
        _ => format!("Review {n} links"),
    }
}

pub fn confirm(n: usize) -> String {
    match (lang(), n) {
        (Lang::De, 1) => "1 Verknüpfung anlegen".to_owned(),
        (Lang::De, _) => format!("{n} Verknüpfungen anlegen"),
        (_, 1) => "Make 1 link".to_owned(),
        _ => format!("Make {n} links"),
    }
}

pub fn limit(n: usize) -> String {
    match lang() {
        Lang::De => format!("Höchstens {n} auf einmal."),
        _ => format!("At most {n} at once."),
    }
}

pub fn links_title() -> String {
    pick("Links", "Verknüpfungen")
}

pub fn plan_title() -> String {
    pick("The book's plan", "Plan des Buchs")
}

pub fn plan_bound(version: &str) -> String {
    match lang() {
        Lang::De => format!(
            "Gebunden an dieses Buch, Eingabestand {version}. Ändert sich das Buch, wird neu geprüft."
        ),
        _ => format!(
            "Bound to this book at input version {version}. If the book changes, it is checked again."
        ),
    }
}

pub fn plan_counts() -> String {
    pick(
        "Your own bitcoin moving between your wallets: no tax, and the cost basis carries over. The links are booked as one movement, so the fee counts once.",
        "Deine eigenen Bitcoin wandern zwischen deinen Wallets: keine Steuer, die Anschaffungskosten gehen mit. Die Verknüpfungen werden als eine Bewegung gebucht, die Gebühr zählt einmal.",
    )
}

pub fn several_warn_before() -> String {
    pick(
        "Some receipts are dated before their send.",
        "Manche Eingänge sind vor ihrem Ausgang datiert.",
    )
}

pub fn several_warn_same_wallet() -> String {
    pick(
        "Some ticked transactions are in the same wallet as the other side.",
        "Manche angehakten Transaktionen liegen in derselben Wallet wie die andere Seite.",
    )
}

/// The core could not give every ticked transaction a part of the other side.
pub fn refused_fit(into_receipt: bool) -> String {
    if into_receipt {
        pick(
            "These sends don't fit into this receipt: once the first ones fill it, nothing is left for the rest. Untick the sends that didn't go into it.",
            "Diese Ausgänge passen nicht in diesen Eingang: Sobald die ersten ihn füllen, bleibt für die übrigen nichts. Hake die Ausgänge ab, die nicht in ihn gingen.",
        )
    } else {
        pick(
            "These receipts don't fit into this send: together they are more than was sent. Untick the receipts that didn't come from it.",
            "Diese Eingänge passen nicht zu diesem Ausgang: Zusammen sind sie mehr als gesendet wurde. Hake die Eingänge ab, die nicht von ihm kamen.",
        )
    }
}
