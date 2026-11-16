//! Setting-level search targets (settingsSearch.ts) and the results the ⌘K palette lists for
//! Settings (`settingsResults` in search/appSearch.ts): one per section and one per setting, so
//! "clipboard" lands on its row. A result opens `/settings/<slug>#<anchor>`; the modal then
//! scrolls the row into view and washes it for a moment (`poll`).

use crate::app::App;
use crate::i18n::{Lang, t};

/// (section index, anchor, label key in `settings:`).
pub const ENTRIES: &[(usize, &str, &str)] = &[
    (super::APPEARANCE, "theme", "appearance.theme.title"),
    (
        super::APPEARANCE,
        "denomination",
        "appearance.denomination.title",
    ),
    (
        super::APPEARANCE,
        "interfaceScale",
        "appearance.scale.title",
    ),
    (super::APPEARANCE, "language", "appearance.language.title"),
    (
        super::APPEARANCE,
        "assistantDock",
        "appearance.assistantDock.title",
    ),
    (super::MARKET, "liveRates", "marketData.autoRefreshLabel"),
    (super::MARKET, "priceSource", "marketData.priceSourceLabel"),
    (super::MARKET, "rebuildPricing", "marketData.rebuildHeading"),
    (
        super::MARKET,
        "coarsePricingReview",
        "marketData.coarse.reviewToggleLabel",
    ),
    (
        super::MARKET,
        "bitcoinRails",
        "marketData.bitcoinRails.toggleLabel",
    ),
    (super::MARKET, "krakenHistory", "marketData.krakenHeading"),
    (super::MARKET, "bundledRates", "marketData.bundledHeading"),
    (super::BITCOIN, "bookNetwork", "bookNetwork.title"),
    (
        super::BITCOIN,
        "bitcoinBackends",
        "networkLayer.bitcoin.heading",
    ),
    (
        super::LIGHTNING,
        "lightningNodes",
        "networkLayer.lightning.heading",
    ),
    (
        super::LIQUID,
        "liquidBackends",
        "networkLayer.liquid.heading",
    ),
    (super::PRIVACY, "offlineMode", "privacy.offlineLabel"),
    (super::PRIVACY, "privacyMode", "privacy.modeLabel"),
    (super::PRIVACY, "hideOnUnlock", "privacy.unlockLabel"),
    (super::PRIVACY, "clearClipboard", "privacy.clipboardLabel"),
    (
        super::PRIVACY,
        "hideOnScreenShare",
        "privacy.screenShareLabel",
    ),
    (
        super::PRIVACY,
        "desktopNotifications",
        "privacy.notificationsLabel",
    ),
    (super::PRIVACY, "updateChecks", "privacy.updatesLabel"),
    (
        super::PRIVACY,
        "outgoingConnections",
        "privacy.leavesHeading",
    ),
    (
        super::PRIVACY,
        "explorerFallbacks",
        "privacy.explorerFallbacksLabel",
    ),
    (
        super::SECURITY,
        "databaseEncryption",
        "security.encryptionHeading",
    ),
    (super::SECURITY, "passphrase", "security.passphraseLabel"),
    (super::SECURITY, "autoLock", "security.autoLockLabel"),
    (
        super::SECURITY,
        "lockOnLaunch",
        "security.requireOnLaunchLabel",
    ),
    (super::SECURITY, "lockOnClose", "security.lockOnCloseLabel"),
    (
        super::SECURITY,
        "biometricUnlock",
        "security.biometricHeading",
    ),
    (super::AI, "assistantFeatures", "ai.featuresLabel"),
    (super::AI, "aiModels", "ai.modelsHeading"),
    (super::AI, "permissionMode", "ai.permissionMode.label"),
    (super::AI, "externalAgents", "ai.agentsLabel"),
    (super::AI, "chatHistory", "chatHistory.storeLabel"),
    (super::DATA, "importData", "data.importHeading"),
    (super::DATA, "backup", "data.backupHeading"),
    (super::DATA, "dataRoot", "data.dataRootLabel"),
    (super::DATA, "databasePath", "data.databaseLabel"),
    (super::DATA, "databaseSize", "data.databaseSizeLabel"),
    (super::DATA, "osBackup", "data.osBackupLabel"),
    (super::DATA, "resetWelcome", "data.resetWelcomeTitle"),
    (super::DATA, "resetBook", "data.resetBookTitle"),
    (super::DATA, "deleteBooks", "data.deleteBooksTitle"),
    (super::TERMINAL, "terminalCommand", "terminal.statusLabel"),
    (super::DEVELOPER, "earlyFeatures", "developer.enableLabel"),
    (
        super::DEVELOPER,
        "alphaWarning",
        "developer.alphaBannerLabel",
    ),
    (super::DEVELOPER, "logBuffer", "developer.bufferHeading"),
];

/// The static anchor id for a requested one.
pub fn anchor_id(a: &str) -> Option<&'static str> {
    ENTRIES.iter().find(|e| e.1 == a).map(|e| e.1)
}

pub fn section_of(anchor: &str) -> Option<usize> {
    ENTRIES.iter().find(|e| e.1 == anchor).map(|e| e.0)
}

/// Daemon-backed rows render after their reads settle; the reveal waits for them.
pub fn anchor_ready(app: &App, anchor: &str) -> bool {
    match anchor {
        "bookNetwork" => app.read("ui.networks.binding").is_some(),
        "aiModels" | "externalAgents" | "chatHistory" => app.read("ai.providers.list").is_some(),
        "assistantDock" => app.prefs.ai_features,
        _ => true,
    }
}

/// A disclosure that must unfold to show the anchor: the pages' Advanced folds.
pub fn reveal_key(app: &App, anchor: &str) -> Option<String> {
    match anchor {
        "krakenHistory" | "bundledRates" => Some(super::market::ADVANCED.to_owned()),
        "dataRoot" | "databasePath" | "databaseSize" => Some(super::data::ADVANCED.to_owned()),
        "bookNetwork" if super::network::is_bound(app) => Some(super::network::ADVANCED.to_owned()),
        _ => None,
    }
}

/// One palette result.
pub struct Result {
    pub title: String,
    pub subtitle: String,
    /// What the query matches against (localized and English terms).
    pub keywords: String,
    /// `/settings/<slug>` or `/settings/<slug>#<anchor>`.
    pub href: String,
}

fn english(key: &str) -> String {
    let lang = crate::i18n::lang();
    crate::i18n::set_lang(Lang::En);
    let s = t(key);
    crate::i18n::set_lang(lang);
    s
}

/// `settingsResults`: sections, then one result per setting; developer-only ones only with
/// developer tools on.
pub fn results(app: &App) -> Vec<Result> {
    let prefix = t("search:settings.groupPrefix");
    let mut out = vec![];
    let visible = |i: usize| app.prefs.developer_tools || !super::dev_only(i);
    for (i, s) in super::SECTIONS
        .iter()
        .enumerate()
        .filter(|(i, _)| visible(*i))
    {
        let title = t(&format!("settings:nav.section.{}.label", s.key));
        let description = t(&format!("settings:nav.section.{}.description", s.key));
        out.push(Result {
            keywords: format!(
                "settings preferences {} {} {} {title} {description}",
                s.slug,
                english(&format!("settings:nav.section.{}.label", s.key)),
                english(&format!("settings:nav.group.{}", s.group))
            ),
            subtitle: format!("{prefix} · {description}"),
            title,
            href: format!("/settings/{}", super::SECTIONS[i].slug),
        });
    }
    for (sec, anchor, label) in ENTRIES.iter().filter(|e| visible(e.0)) {
        let s = &super::SECTIONS[*sec];
        let title = t(&format!("settings:{label}"));
        let section_title = t(&format!("settings:nav.section.{}.label", s.key));
        let kw = format!("settings:search.keywords.{anchor}");
        out.push(Result {
            keywords: format!(
                "{title} {section_title} {} {} {} {}",
                english(&format!("settings:nav.section.{}.label", s.key)),
                english(&format!("settings:{label}")),
                t(&kw).replace(',', " "),
                english(&kw).replace(',', " ")
            ),
            subtitle: format!("{prefix} · {section_title}"),
            title,
            href: format!("/settings/{}#{anchor}", s.slug),
        });
    }
    // Sub-pages a result opens directly (`#addBackend` etc., see `subpage::ANCHORS`).
    for (sec, anchor, label) in PAGES.iter().filter(|e| visible(e.0)) {
        let s = &super::SECTIONS[*sec];
        let title = t(label);
        let section_title = t(&format!("settings:nav.section.{}.label", s.key));
        out.push(Result {
            keywords: format!(
                "{title} {section_title} {} {} add new",
                english(label),
                english(&format!("settings:nav.section.{}.label", s.key))
            ),
            subtitle: format!("{prefix} · {section_title}"),
            title,
            href: format!("/settings/{}#{anchor}", s.slug),
        });
    }
    out
}

/// (section, page anchor, title key) for the palette's sub-page results.
pub const PAGES: &[(usize, &str, &str)] = &[
    (
        super::BITCOIN,
        "addBackend",
        "settings:networkLayer.bitcoin.add",
    ),
    (
        super::LIGHTNING,
        "addBackend",
        "settings:networkLayer.lightning.add",
    ),
    (
        super::LIQUID,
        "addBackend",
        "settings:networkLayer.liquid.add",
    ),
    (super::AI, "addProvider", "settings:aiProvider.addTitle"),
    (
        super::SECURITY,
        "changePassphrase",
        "settings:changePassphrase.title",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_anchor_belongs_to_one_section() {
        for (sec, a, _) in ENTRIES {
            assert_eq!(section_of(a), Some(*sec));
            assert_eq!(anchor_id(a), Some(*a));
        }
        assert_eq!(anchor_id("nope"), None);
        for (sec, a, _) in PAGES {
            assert!(super::super::subpage::ANCHORS.contains(&(*a, *sec)));
        }
    }
}
