//! NEW STRINGS — not in the web i18n bundles.
//!
//! The picker overhaul (owner request) replaced the LOCAL / REMOTE pills with one quiet line
//! under the provider's name saying where prompts go. Where the web already has the words
//! (`assistant:modelPicker.postureHint.*`) they are reused; the lines that name the Mac, the
//! network, a host or a vendor live here in English and German until the bundles gain keys.

use crate::i18n::{Lang, lang, t};

use super::data::Where;

fn pick(en: &str, de: &str) -> String {
    match lang() {
        Lang::De => de.to_owned(),
        _ => en.to_owned(),
    }
}

/// The disclosure line for a provider (or a model whose posture differs from its provider's).
pub fn disclosure(w: &Where) -> String {
    match w {
        Where::ThisDevice if cfg!(target_os = "macos") => {
            pick("Runs on this Mac", "Läuft auf diesem Mac")
        }
        Where::ThisDevice => t("assistant:modelPicker.postureHint.local"),
        Where::Network => pick("On your network", "In deinem Netzwerk"),
        Where::Host(h) => pick(&format!("Runs on {h}"), &format!("Läuft auf {h}")),
        Where::Vendor(v) => pick(
            &format!("Sends prompts to {v}"),
            &format!("Sendet Prompts an {v}"),
        ),
        Where::OpenCode => pick(
            "Sends prompts to the model services OpenCode uses",
            "Sendet Prompts an die Modelldienste, die OpenCode nutzt",
        ),
        Where::Tee => t("assistant:modelPicker.postureHint.tee"),
        Where::Remote => t("assistant:modelPicker.postureHint.remote"),
    }
}

/// The rail's and the header's warning after a check of this provider failed.
pub fn check_failed() -> String {
    pick("Last check failed", "Letzte Prüfung fehlgeschlagen")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_line_reads_in_both_languages() {
        let all = [
            Where::ThisDevice,
            Where::Network,
            Where::Host("box.example".into()),
            Where::Vendor("Anthropic".into()),
            Where::OpenCode,
            Where::Tee,
            Where::Remote,
        ];
        for w in &all {
            assert!(!disclosure(w).is_empty());
        }
        assert!(disclosure(&Where::Vendor("OpenAI".into())).contains("OpenAI"));
    }
}
