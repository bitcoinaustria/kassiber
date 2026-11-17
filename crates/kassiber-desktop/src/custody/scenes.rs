//! Named states for `--scene` (headless snapshots and the parity harness).

use crate::app::{App, ThemeMode};
use crate::i18n::Lang;

fn go(a: &mut App, to: &str) {
    a.navigate(to);
    super::prime(a);
}

fn dev(a: &mut App) {
    a.prefs.developer_tools = true;
}

/// A scene a part defines in its own `SCENES`, run by name.
fn sub(list: &[crate::app::Scene], name: &str, a: &mut App) {
    if let Some((_, f)) = list.iter().find(|(n, _)| *n == name) {
        f(a);
    }
}

pub const SCENES: &[crate::app::Scene] = &[
    ("custody", |a| go(a, "/transfers")),
    ("custody-light", |a| {
        a.prefs.theme = ThemeMode::Light;
        go(a, "/transfers");
    }),
    ("custody-de", |a| {
        a.prefs.lang = Lang::De;
        crate::i18n::set_lang(Lang::De);
        go(a, "/transfers");
    }),
    ("custody-needs", |a| {
        go(a, "/transfers");
        a.custody.page.tab = Some(super::page::StatusTab::Needs);
    }),
    ("custody-menu", |a| {
        go(a, "/transfers");
        a.custody.page.popup = Some(super::page::Popup::Menu);
    }),
    ("custody-menu-dev", |a| {
        dev(a);
        go(a, "/transfers");
        a.custody.page.popup = Some(super::page::Popup::Menu);
    }),
    ("custody-filter", |a| {
        go(a, "/transfers");
        a.custody.page.popup = Some(super::page::Popup::Filter);
    }),
    // On the variant book (`tmp/variant.sh`): a pair unlinked, a suggestion set aside.
    ("custody-unlinked", |a| {
        go(a, "/transfers");
        a.custody.page.tab = Some(super::page::StatusTab::Unlinked);
    }),
    ("custody-setaside", |a| {
        go(a, "/transfers");
        a.custody.page.tab = Some(super::page::StatusTab::SetAside);
    }),
    // The demo book has nothing unlinked: three linked moves shown as recently unlinked, so the
    // card's rows (the table kit's stacked route) can be checked headless.
    ("custody-unlinked-synthetic", |a| {
        go(a, "/transfers");
        let rows: Vec<super::model::Unlinked> = a
            .custody
            .page
            .moves
            .iter()
            .take(3)
            .map(|m| super::model::Unlinked {
                key: format!("syn-{}", m.key.replace('\u{0}', "-")),
                out_transaction_id: m.out_transaction_id.clone(),
                in_transaction_id: m.in_transaction_id.clone(),
                from_label: m.from_label.clone(),
                to_label: m.to_label.clone(),
                from_rail: m.from_rail,
                to_rail: m.to_rail,
                occurred_at: m.occurred_at.clone(),
                asset: m.asset.clone(),
                amount_msat: m.amount_msat,
                unlinked_at: m.occurred_at.clone(),
                pair: None,
                dismissal: None,
            })
            .collect();
        a.custody.page.unlinked = rows;
        a.custody.page.tab = Some(super::page::StatusTab::Unlinked);
    }),
    ("custody-unlinked-menu", |a| {
        go(a, "/transfers");
        a.custody.page.tab = Some(super::page::StatusTab::Unlinked);
        if let Some(r) = a.custody.page.unlinked.first() {
            a.custody.page.popup = Some(super::page::Popup::Held(format!(
                "unl-{}",
                r.key.replace('\u{0}', "-")
            )));
        }
    }),
    ("custody-row-menu", |a| {
        go(a, "/transfers");
        super::page::open_first_row_menu(a);
    }),
    ("custody-strip", |a| {
        go(a, "/transfers");
        super::page::inject_note(a, true);
    }),
    ("custody-toast", |a| {
        go(a, "/transfers");
        super::page::inject_note(a, false);
    }),
    ("custody-synthetic-holds", |a| {
        go(a, "/transfers");
        super::page::inject_synthetic_holds(a);
    }),
    ("custody-synthetic-holds-dev", |a| {
        dev(a);
        go(a, "/transfers");
        super::page::inject_synthetic_holds(a);
    }),
    ("custody-filtered", |a| {
        go(a, "/transfers");
        a.custody.page.filter = Some(super::model::Filter::Reviewed);
        a.custody.page.year = Some(2024);
    }),
    ("custody-advanced", |a| {
        dev(a);
        go(a, "/transfers?section=advanced");
    }),
    ("custody-advanced-components", |a| {
        dev(a);
        go(a, "/transfers?section=advanced&tab=components");
    }),
    ("custody-advanced-coverage", |a| {
        dev(a);
        go(a, "/transfers?section=advanced&tab=coverage");
    }),
    ("custody-gaps-fixture-resolved", |a| {
        sub(super::gaps::SCENES, "custody-gaps-fixture-resolved", a)
    }),
    ("custody-gaps-fixture-reopen", |a| {
        sub(super::gaps::SCENES, "custody-gaps-fixture-reopen", a)
    }),
    ("custody-gaps-fixture-residual", |a| {
        sub(super::gaps::SCENES, "custody-gaps-fixture-residual", a)
    }),
    ("custody-gaps", |a| {
        sub(super::gaps::SCENES, "custody-gaps", a)
    }),
    ("custody-gaps-light", |a| {
        sub(super::gaps::SCENES, "custody-gaps-light", a)
    }),
    ("custody-gaps-de", |a| {
        sub(super::gaps::SCENES, "custody-gaps-de", a)
    }),
    ("custody-gaps-review", |a| {
        sub(super::gaps::SCENES, "custody-gaps-review", a)
    }),
    ("custody-gaps-plan", |a| {
        sub(super::gaps::SCENES, "custody-gaps-plan", a)
    }),
    ("custody-gaps-plan-confirmed", |a| {
        sub(super::gaps::SCENES, "custody-gaps-plan-confirmed", a)
    }),
    ("custody-gaps-dismiss", |a| {
        sub(super::gaps::SCENES, "custody-gaps-dismiss", a)
    }),
    ("custody-gaps-review-de", |a| {
        sub(super::gaps::SCENES, "custody-gaps-review-de", a)
    }),
    ("custody-gaps-coverage", |a| {
        sub(super::gaps::SCENES, "custody-gaps-coverage", a)
    }),
    ("custody-gaps-coverage-light", |a| {
        sub(super::gaps::SCENES, "custody-gaps-coverage-light", a)
    }),
    ("custody-link-new", |a| {
        sub(super::link::SCENES, "custody-link-new", a)
    }),
    ("custody-link-send", |a| {
        sub(super::link::SCENES, "custody-link-send", a)
    }),
    ("custody-link-check", |a| {
        sub(super::link::SCENES, "custody-link-check", a)
    }),
    ("custody-link-question", |a| {
        sub(super::link::SCENES, "custody-link-question", a)
    }),
    ("custody-link-light", |a| {
        sub(super::link::SCENES, "custody-link-light", a)
    }),
    ("custody-link-de", |a| {
        sub(super::link::SCENES, "custody-link-de", a)
    }),
    ("custody-link-several", |a| {
        sub(super::link::SCENES, "custody-link-several", a)
    }),
    ("custody-link-several-pick", |a| {
        sub(super::link::SCENES, "custody-link-several-pick", a)
    }),
    ("custody-link-several-sends", |a| {
        sub(super::link::SCENES, "custody-link-several-sends", a)
    }),
    ("custody-link-several-check", |a| {
        sub(super::link::SCENES, "custody-link-several-check", a)
    }),
    ("custody-link-several-light", |a| {
        sub(super::link::SCENES, "custody-link-several-light", a)
    }),
    ("custody-link-several-de", |a| {
        sub(super::link::SCENES, "custody-link-several-de", a)
    }),
    ("custody-link-several-check-de", |a| {
        sub(super::link::SCENES, "custody-link-several-check-de", a)
    }),
    ("custody-unlink-details", |a| {
        sub(super::unlink::SCENES, "custody-unlink-details", a)
    }),
    ("custody-unlink-details-dialog", |a| {
        sub(super::unlink::SCENES, "custody-unlink-details-dialog", a)
    }),
    ("custody-unlink-pair", |a| {
        sub(super::unlink::SCENES, "custody-unlink-pair", a)
    }),
    ("custody-unlink-pair-dialog", |a| {
        sub(super::unlink::SCENES, "custody-unlink-pair-dialog", a)
    }),
    ("custody-unlink-fixed", |a| {
        sub(super::unlink::SCENES, "custody-unlink-fixed", a)
    }),
    ("custody-unlink-light", |a| {
        sub(super::unlink::SCENES, "custody-unlink-light", a)
    }),
    ("custody-unlink-de", |a| {
        sub(super::unlink::SCENES, "custody-unlink-de", a)
    }),
    ("custody-comps", |a| {
        sub(super::components::SCENES, "custody-comps", a)
    }),
    ("custody-comps-light", |a| {
        sub(super::components::SCENES, "custody-comps-light", a)
    }),
    ("custody-comps-de", |a| {
        sub(super::components::SCENES, "custody-comps-de", a)
    }),
    ("custody-comps-expanded", |a| {
        sub(super::components::SCENES, "custody-comps-expanded", a)
    }),
    ("custody-comps-new", |a| {
        sub(super::components::SCENES, "custody-comps-new", a)
    }),
    ("custody-comps-new-invalid", |a| {
        sub(super::components::SCENES, "custody-comps-new-invalid", a)
    }),
    ("custody-comps-revise", |a| {
        sub(super::components::SCENES, "custody-comps-revise", a)
    }),
    ("custody-comps-revise-de", |a| {
        sub(super::components::SCENES, "custody-comps-revise-de", a)
    }),
    ("custody-comps-select", |a| {
        sub(super::components::SCENES, "custody-comps-select", a)
    }),
    ("custody-comps-preview", |a| {
        sub(super::components::SCENES, "custody-comps-preview", a)
    }),
    ("custody-comps-confirm", |a| {
        sub(super::components::SCENES, "custody-comps-confirm", a)
    }),
    ("custody-comps-confirm-de", |a| {
        sub(super::components::SCENES, "custody-comps-confirm-de", a)
    }),
    ("custody-sugg", |a| {
        sub(super::suggestions::SCENES, "custody-sugg", a)
    }),
    ("custody-sugg-light", |a| {
        sub(super::suggestions::SCENES, "custody-sugg-light", a)
    }),
    ("custody-sugg-de", |a| {
        sub(super::suggestions::SCENES, "custody-sugg-de", a)
    }),
    ("custody-sugg-expanded", |a| {
        sub(super::suggestions::SCENES, "custody-sugg-expanded", a)
    }),
    ("custody-sugg-selected", |a| {
        sub(super::suggestions::SCENES, "custody-sugg-selected", a)
    }),
    ("custody-sugg-menu", |a| {
        sub(super::suggestions::SCENES, "custody-sugg-menu", a)
    }),
    ("custody-sugg-about", |a| {
        sub(super::suggestions::SCENES, "custody-sugg-about", a)
    }),
    ("custody-sugg-bulk", |a| {
        sub(super::suggestions::SCENES, "custody-sugg-bulk", a)
    }),
    ("custody-sugg-undo", |a| {
        sub(super::suggestions::SCENES, "custody-sugg-undo", a)
    }),
    ("custody-sugg-empty-undo", |a| {
        sub(super::suggestions::SCENES, "custody-sugg-empty-undo", a)
    }),
    ("custody-sugg-view", |a| {
        sub(super::suggestions::SCENES, "custody-sugg-view", a)
    }),
    ("custody-sugg-view-empty", |a| {
        sub(super::suggestions::SCENES, "custody-sugg-view-empty", a)
    }),
    ("custody-sugg-narrow", |a| {
        sub(super::suggestions::SCENES, "custody-sugg-narrow", a)
    }),
    ("custody-rules", |a| {
        sub(super::rules::SCENES, "custody-rules", a)
    }),
    ("custody-rules-light", |a| {
        sub(super::rules::SCENES, "custody-rules-light", a)
    }),
    ("custody-rules-de", |a| {
        sub(super::rules::SCENES, "custody-rules-de", a)
    }),
    ("custody-rules-new", |a| {
        sub(super::rules::SCENES, "custody-rules-new", a)
    }),
    ("custody-rules-new-kind", |a| {
        sub(super::rules::SCENES, "custody-rules-new-kind", a)
    }),
    ("custody-rules-confirm", |a| {
        sub(super::rules::SCENES, "custody-rules-confirm", a)
    }),
];
