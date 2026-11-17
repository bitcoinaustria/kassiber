//! Headless states of Welcome and setup (`--scene onboarding-…`). Every scene shows Welcome on
//! the launch's daemon without reading the book; the import scenes open a folder as the picker
//! would (`--import-path <dir>`, default: the launch book's own folder).

use rune::TextEdit;
use serde_json::json;

use super::model::{AiMode, BackendKind, BackendMode, DbMode};
use super::projects;
use crate::app::{App, ThemeMode};
use crate::i18n::Lang;

/// Welcome, freshly mounted, without the regtest probe (a scene shows the page itself).
fn welcome(a: &mut App) {
    a.onboarding.identity = None;
    super::enter_welcome_with(a, false);
}

fn setup_at(a: &mut App, step: usize) {
    welcome(a);
    super::begin_setup(a);
    a.onboarding.step = step;
    super::prime(a);
}

fn light(a: &mut App) {
    a.prefs.theme = ThemeMode::Light;
}

fn de(a: &mut App) {
    a.prefs.lang = Lang::De;
}

fn custom_electrum(a: &mut App) {
    setup_at(a, 1);
    let f = &mut a.onboarding.form;
    f.backend_mode = Some(BackendMode::Custom);
    f.backend_kind = BackendKind::Electrum;
    f.backend_name = TextEdit::new("");
    f.backend_host = TextEdit::new("");
}

fn security_filled(a: &mut App) {
    setup_at(a, 3);
    let f = &mut a.onboarding.form;
    f.passphrase = TextEdit::new("aaaaaaaaaaaa");
    f.confirm = TextEdit::new("aaaaaaaaaaab");
    a.onboarding.touched.insert("database-passphrase-confirm");
}

fn review(a: &mut App) {
    setup_at(a, 4);
    let f = &mut a.onboarding.form;
    f.backend_mode = Some(BackendMode::Default);
    f.passphrase = TextEdit::new("correct horse battery staple");
    f.confirm = TextEdit::new("correct horse battery staple");
    f.recovery_ack = true;
}

/// Review after the plaintext path (what the parity recipe can type: no passphrase).
fn review_plain(a: &mut App) {
    setup_at(a, 4);
    let f = &mut a.onboarding.form;
    f.backend_mode = Some(BackendMode::Default);
    f.db_mode = DbMode::Plaintext;
    f.plaintext_ack = true;
}

/// Opens a folder as the picker would and loads its books.
fn import(a: &mut App) {
    welcome(a);
    let path = crate::app::flag(rune::extra_args(), "--import-path")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            a.onboarding
                .seed
                .as_ref()
                .and_then(|s| s.parent().map(std::path::Path::to_path_buf))
        });
    if let Some(p) = path {
        super::picked(a, &p);
        super::pump_until(
            a,
            |a| !a.onboarding.loading_profiles,
            std::time::Duration::from_secs(20),
        );
    }
}

/// SYNTHETIC: an encrypted folder waiting for its passphrase (no daemon restart; the paths are
/// placeholders).
fn import_encrypted(a: &mut App) {
    welcome(a);
    a.onboarding.import_selection = Some(projects::Selection {
        state_root: "/Users/me/Kassiber".into(),
        data_root: "/Users/me/Kassiber/data".into(),
        database: "/Users/me/Kassiber/data/kassiber.sqlite3".into(),
        encrypted: true,
    });
    a.onboarding.import_pass.focused = true;
}

pub static SCENES: &[crate::app::Scene] = &[
    ("onboarding-start", welcome),
    ("onboarding-start-light", |a| {
        light(a);
        welcome(a);
    }),
    ("onboarding-start-de", |a| {
        de(a);
        welcome(a);
    }),
    ("onboarding-start-narrow", welcome),
    // The dev-seeded launch's tile, held on screen (the web opens it at once).
    ("onboarding-start-regtest", |a| {
        welcome(a);
        a.onboarding.regtest = Some(
            json!({"current_workspace": "Regtest Demo", "current_profile": "Full Accounting", "database_encrypted": false}),
        );
    }),
    ("onboarding-lang", |a| {
        welcome(a);
        a.onboarding.lang_open = true;
    }),
    ("onboarding-essentials", |a| setup_at(a, 0)),
    ("onboarding-essentials-light", |a| {
        light(a);
        setup_at(a, 0);
    }),
    ("onboarding-essentials-de", |a| {
        de(a);
        setup_at(a, 0);
    }),
    ("onboarding-essentials-generic", |a| {
        setup_at(a, 0);
        let f = &mut a.onboarding.form;
        f.tax_country = "generic".into();
        f.gains = "FIFO".into();
        f.long_term_days = TextEdit::new("0");
    }),
    ("onboarding-essentials-preview", |a| {
        setup_at(a, 0);
        let f = &mut a.onboarding.form;
        f.tax_country = "de".into();
        f.gains = "FIFO".into();
    }),
    ("onboarding-essentials-country", |a| {
        setup_at(a, 0);
        a.onboarding.select = Some("onb-tax-country".into());
    }),
    ("onboarding-sync", |a| setup_at(a, 1)),
    ("onboarding-sync-default", |a| {
        setup_at(a, 1);
        a.onboarding.form.backend_mode = Some(BackendMode::Default);
    }),
    ("onboarding-sync-light", |a| {
        light(a);
        setup_at(a, 1);
        a.onboarding.form.backend_mode = Some(BackendMode::Default);
    }),
    ("onboarding-sync-de", |a| {
        de(a);
        setup_at(a, 1);
        a.onboarding.form.backend_mode = Some(BackendMode::Default);
    }),
    ("onboarding-sync-custom", custom_electrum),
    ("onboarding-sync-custom-onion", |a| {
        custom_electrum(a);
        let f = &mut a.onboarding.form;
        f.backend_kind = BackendKind::Esplora;
        f.backend_url = TextEdit::new("abcdefghijklmnop.onion/api");
    }),
    ("onboarding-sync-offline", |a| {
        setup_at(a, 1);
        a.onboarding.form.backend_mode = Some(BackendMode::Skip);
    }),
    ("onboarding-ai", |a| {
        setup_at(a, 2);
        a.onboarding.form.backend_mode = Some(BackendMode::Default);
    }),
    ("onboarding-ai-light", |a| {
        light(a);
        setup_at(a, 2);
        a.onboarding.form.backend_mode = Some(BackendMode::Default);
    }),
    ("onboarding-ai-de", |a| {
        de(a);
        setup_at(a, 2);
        a.onboarding.form.backend_mode = Some(BackendMode::Default);
    }),
    ("onboarding-ai-remote", |a| {
        setup_at(a, 2);
        let f = &mut a.onboarding.form;
        f.backend_mode = Some(BackendMode::Default);
        f.ai_mode = AiMode::Remote;
        f.ai_kind = super::model::AiKind::Remote;
        f.ai_name = TextEdit::new("");
        f.ai_url = TextEdit::new("");
    }),
    ("onboarding-ai-offline", |a| {
        setup_at(a, 2);
        a.onboarding.form.backend_mode = Some(BackendMode::Skip);
    }),
    ("onboarding-security", |a| {
        setup_at(a, 3);
        a.onboarding.form.passphrase.focused = true;
    }),
    ("onboarding-security-light", |a| {
        light(a);
        setup_at(a, 3);
    }),
    ("onboarding-security-de", |a| {
        de(a);
        setup_at(a, 3);
    }),
    ("onboarding-security-filled", security_filled),
    ("onboarding-security-plaintext", |a| {
        setup_at(a, 3);
        a.onboarding.form.db_mode = DbMode::Plaintext;
    }),
    ("onboarding-security-credentials", |a| {
        setup_at(a, 3);
        a.onboarding.migrate_open = true;
    }),
    // "Or create with recommended defaults": Security, with the built-in servers chosen.
    ("onboarding-quick", |a| {
        welcome(a);
        super::begin_quick_start(a);
        super::prime(a);
    }),
    ("onboarding-review", review),
    ("onboarding-review-light", |a| {
        light(a);
        review(a);
    }),
    ("onboarding-review-de", |a| {
        de(a);
        review(a);
    }),
    ("onboarding-review-plain", review_plain),
    ("onboarding-review-plain-light", |a| {
        light(a);
        review_plain(a);
    }),
    ("onboarding-review-plain-de", |a| {
        de(a);
        review_plain(a);
    }),
    ("onboarding-review-offline", |a| {
        review(a);
        let f = &mut a.onboarding.form;
        f.backend_mode = Some(BackendMode::Skip);
        f.db_mode = DbMode::Plaintext;
        f.plaintext_ack = true;
    }),
    ("onboarding-finish-error", |a| {
        review(a);
        a.onboarding.finish_error = Some(crate::i18n::t("onboarding:shell.finishError"));
    }),
    ("onboarding-import", import),
    ("onboarding-import-light", |a| {
        light(a);
        import(a);
    }),
    ("onboarding-import-de", |a| {
        de(a);
        import(a);
    }),
    ("onboarding-import-encrypted", import_encrypted),
    ("onboarding-import-error", |a| {
        welcome(a);
        a.onboarding.import_error = Some("Choose a Kassiber project folder containing data/kassiber.sqlite3, or choose the data folder itself.".into());
    }),
];
