//! The home screen's own copy. The web Overview has no widget grid, so its bundles carry no keys
//! for these; English and German live here until they do.

use super::model::Size;
use crate::i18n::{Lang, lang};

fn l(en: &str, de: &str) -> String {
    match lang() {
        Lang::De => de.to_owned(),
        Lang::En => en.to_owned(),
    }
}

pub fn stats_title() -> String {
    l("Key figures", "Kennzahlen")
}
pub fn grid_aria() -> String {
    l("Overview widgets", "Widgets der Übersicht")
}
pub fn edit() -> String {
    l("Edit", "Bearbeiten")
}
pub fn edit_home() -> String {
    l("Edit Home Screen", "Home-Bildschirm bearbeiten")
}
pub fn editing() -> String {
    l(
        "Editing widgets. Drag a widget to move it.",
        "Widgets werden bearbeitet. Widget ziehen, um es zu verschieben.",
    )
}
pub fn edit_done() -> String {
    l(
        "Finished editing widgets.",
        "Bearbeiten der Widgets beendet.",
    )
}
pub fn add_widget() -> String {
    l("Add widget", "Widget hinzufügen")
}
pub fn add_widget_more() -> String {
    l("Add widget…", "Widget hinzufügen …")
}
pub fn change_size() -> String {
    l("Change size", "Größe ändern")
}
pub fn reset() -> String {
    l("Reset to default", "Standard wiederherstellen")
}
pub fn reset_done() -> String {
    l(
        "Widgets reset to the default.",
        "Widgets auf den Standard zurückgesetzt.",
    )
}
pub fn size_label() -> String {
    l("Size", "Größe")
}
pub fn size(s: Size) -> String {
    match s {
        Size::Small => l("Small", "Klein"),
        Size::Medium => l("Medium", "Mittel"),
        Size::Large => l("Large", "Groß"),
        Size::XLarge => l("Extra large", "Sehr groß"),
    }
}
pub fn remove() -> String {
    l("Remove widget", "Widget entfernen")
}
pub fn remove_named(name: &str) -> String {
    match lang() {
        Lang::De => format!("{name} entfernen"),
        Lang::En => format!("Remove {name}"),
    }
}
pub fn removed(name: &str) -> String {
    match lang() {
        Lang::De => format!("{name} entfernt."),
        Lang::En => format!("{name} removed."),
    }
}
pub fn added(name: &str, size: Size) -> String {
    match lang() {
        Lang::De => format!("{name} hinzugefügt, {}.", self::size(size)),
        Lang::En => format!("{name} added, {}.", self::size(size)),
    }
}
pub fn moved(name: &str, pos: usize, of: usize) -> String {
    match lang() {
        Lang::De => format!("{name} verschoben, Position {pos} von {of}."),
        Lang::En => format!("{name} moved, position {pos} of {of}."),
    }
}
pub fn sized(name: &str, size: Size) -> String {
    match lang() {
        Lang::De => format!("{name}: Größe {}.", self::size(size)),
        Lang::En => format!("{name}: size {}.", self::size(size)),
    }
}
pub fn one_size(name: &str) -> String {
    match lang() {
        Lang::De => format!("{name} hat nur diese Größe."),
        Lang::En => format!("{name} has only this size."),
    }
}
pub fn edge(name: &str) -> String {
    match lang() {
        Lang::De => format!("{name} lässt sich nicht weiter verschieben."),
        Lang::En => format!("{name} cannot move further."),
    }
}
/// A tile's name in edit mode: what it is, its size and its place.
pub fn tile(name: &str, size: Size, pos: usize, of: usize) -> String {
    match lang() {
        Lang::De => format!("{name}, {}, Position {pos} von {of}", self::size(size)),
        Lang::En => format!("{name}, {}, position {pos} of {of}", self::size(size)),
    }
}
pub fn keys() -> String {
    l(
        "Arrow keys move it, S changes its size, Delete removes it, Escape finishes.",
        "Pfeiltasten verschieben, S ändert die Größe, Entf entfernt, Esc beendet.",
    )
}
pub fn size_of(name: &str, size: Size) -> String {
    match lang() {
        Lang::De => format!("Größe von {name} ändern: {}", self::size(size)),
        Lang::En => format!("Change size of {name}: {}", self::size(size)),
    }
}
pub fn gallery_title() -> String {
    l("Widgets", "Widgets")
}
pub fn gallery_hint() -> String {
    l("Pick a widget and a size.", "Widget und Größe wählen.")
}
pub fn on_home() -> String {
    l("Added", "Hinzugefügt")
}
pub fn preview_of(name: &str, size: Size) -> String {
    match lang() {
        Lang::De => format!("Vorschau: {name}, {}", self::size(size)),
        Lang::En => format!("Preview: {name}, {}", self::size(size)),
    }
}
pub fn empty_home() -> String {
    l(
        "No widgets on the Overview.",
        "Keine Widgets in der Übersicht.",
    )
}
pub fn flows_empty() -> String {
    l(
        "No money moved in the last twelve months.",
        "In den letzten zwölf Monaten wurde kein Geld bewegt.",
    )
}
pub fn open_named(name: &str, action: &str) -> String {
    format!("{name}: {action}")
}
pub fn description(id: super::model::WidgetId) -> String {
    use super::model::WidgetId as W;
    match id {
        W::Stats => l(
            "Bitcoin balance, price and portfolio value at a glance.",
            "Bitcoin-Bestand, Kurs und Portfoliowert auf einen Blick.",
        ),
        W::Chart => l(
            "Balance and activity over time, with zoom and range.",
            "Bestand und Aktivität im Zeitverlauf, mit Zoom und Zeitraum.",
        ),
        W::Recent => l("The newest transactions.", "Die neuesten Transaktionen."),
        W::Holdings => l(
            "Where the bitcoin sits, by wallet and source.",
            "Wo die Bitcoin liegen, nach Wallet und Quelle.",
        ),
        W::TaxYear => l(
            "Realized and unrealized gains this year.",
            "Realisierte und unrealisierte Gewinne in diesem Jahr.",
        ),
        W::Flows => l(
            "Received and spent, month by month.",
            "Empfangen und ausgegeben, Monat für Monat.",
        ),
        W::Assets => l(
            "What the book holds, by asset and layer.",
            "Was das Buch hält, nach Asset und Ebene.",
        ),
        W::Attention => crate::overview::attention::description(),
        W::Age => crate::overview::age::description(),
        W::Goal => crate::overview::goal::description(),
    }
}
pub fn clear_goal() -> String {
    l("Remove goal", "Ziel entfernen")
}
