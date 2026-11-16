//! Privacy (PrivacySettingsPanel.tsx, ExplorerFallbackSetting.tsx): offline mode, on-screen
//! privacy mode, hide-on-unlock, clipboard clearing, notifications, update checks, and the
//! outgoing-connections overview grouped by what each endpoint sees.

use rune::{El, col, text};
use serde_json::{Value, json};

use super::controls::{
    Btn, amber, btn, desc, emerald, group, opt, para, select, srow, srow_a, switch,
};
use super::model::{self, Backend, Net, Posture};
use super::send;
use crate::app::{App, Privacy};
use crate::i18n::{t, tc, tr};
use crate::theme::Tk;

/// `backendExposureFilter`.
fn exposure(b: &Backend) -> &'static str {
    match model::backend_trust(b).posture {
        Posture::OnDevice | Posture::SelfHosted => "first",
        Posture::Shielded => "shielded",
        Posture::Remote => "remote",
    }
}

const FILTERS: [(&str, &str, &str); 3] = [
    (
        "first",
        "settings:privacy.exposure.firstParty",
        "settings:privacy.exposure.firstPartyHint",
    ),
    (
        "shielded",
        "settings:privacy.exposure.shielded",
        "settings:privacy.exposure.shieldedHint",
    ),
    (
        "remote",
        "settings:privacy.exposure.thirdParty",
        "settings:privacy.exposure.thirdPartyHint",
    ),
];

fn tone(tk: &Tk, f: &str) -> rune::Color {
    match f {
        "first" => emerald(tk),
        "remote" => amber(tk),
        _ => tk.muted_fg,
    }
}

fn offline(app: &App) -> (bool, bool, bool) {
    let v = app.read("ui.network.offline");
    (
        v.is_some_and(|v| v["offline"].as_bool() == Some(true)),
        v.is_some_and(|v| v["environment_blocked"].as_bool() == Some(true)),
        v.is_some(),
    )
}

pub fn view(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let p = &app.settings.prefs;
    let (is_off, env_blocked, known) = offline(app);
    let pending = app.settings.is_busy("settings.offline_set");
    let off_sw = switch(
        tk,
        "set-offline",
        is_off,
        &t("settings:privacy.offlineLabel"),
        known && !pending,
        |a, on| {
            a.settings.offline_failed = false;
            send(
                a,
                "ui.network.offline.set",
                json!({"enabled": on}),
                "settings.offline_set",
            );
        },
    );
    let mut offline_g = group()
        .title(t("settings:privacy.offlineHeading"))
        .row(srow_a(
            app,
            tk,
            t("settings:privacy.offlineLabel"),
            desc(
                tk,
                t(if is_off {
                    "settings:privacy.offlineOn"
                } else {
                    "settings:privacy.offlineOff"
                }),
            ),
            vec![off_sw],
            Some("offlineMode"),
        ));
    if env_blocked && !is_off {
        offline_g = offline_g.footer(t("settings:privacy.offlineEnvironment"));
    }
    if app.settings.offline_failed {
        offline_g = offline_g
            .row(para(tk, t("settings:privacy.offlineFailed"), tk.destructive).role("alert"));
    }

    // The transaction graph's automatic lookup from the wallet's own backend (a Rune addition).
    let lookup = {
        use crate::txdetail::graph::auto;
        let writable = auto::preference_path(app).is_some();
        let on = app.graph.auto_enabled;
        let label = auto::setting_text("label");
        let note = if !writable {
            "fixed"
        } else if on {
            "on"
        } else {
            "off"
        };
        group()
            .title(auto::setting_text("heading").to_owned())
            .row(srow(
                app,
                tk,
                label,
                desc(tk, auto::setting_text(note)),
                vec![switch(
                    tk,
                    "set-wallet-backend-lookup",
                    on,
                    label,
                    writable,
                    auto::set_enabled,
                )],
            ))
    };

    let mode = match app.prefs.privacy {
        Privacy::Normal => "normal",
        Privacy::HideAmounts => "hideAmounts",
        Privacy::HideAll => "hideAll",
    };
    let mode_sel = select(
        app,
        tk,
        layer,
        "set-privacy-mode",
        &t("settings:privacy.modeLabel"),
        mode,
        vec![
            opt("normal", t("chrome:sensitive.modes.normal")),
            opt("hideAmounts", t("chrome:sensitive.modes.hideAmounts")),
            opt("hideAll", t("chrome:sensitive.modes.hideAll")),
        ],
        true,
        None,
        |a, v| {
            a.prefs.privacy = match v {
                "hideAmounts" => Privacy::HideAmounts,
                "hideAll" => Privacy::HideAll,
                _ => Privacy::Normal,
            }
        },
    );
    let mode_desc = match mode {
        "hideAmounts" => "settings:privacy.modeHideAmounts",
        "hideAll" => "settings:privacy.modeHideAll",
        _ => "settings:privacy.modeNormal",
    };
    let screen = group()
        .title(t("settings:privacy.onScreenHeading"))
        .row(srow_a(
            app,
            tk,
            t("settings:privacy.modeLabel"),
            desc(tk, t(mode_desc)),
            vec![mode_sel],
            Some("privacyMode"),
        ))
        .row(srow_a(
            app,
            tk,
            t("settings:privacy.unlockLabel"),
            desc(
                tk,
                t(if app.prefs.hide_on_unlock {
                    "settings:privacy.unlockOn"
                } else {
                    "settings:privacy.unlockOff"
                }),
            ),
            vec![switch(
                tk,
                "set-hide-unlock",
                app.prefs.hide_on_unlock,
                &t("settings:privacy.unlockLabel"),
                true,
                |a, v| a.prefs.hide_on_unlock = v,
            )],
            Some("hideOnUnlock"),
        ))
        .row(srow_a(
            app,
            tk,
            t("settings:privacy.clipboardLabel"),
            desc(tk, crate::osleaks::ui::clipboard_note(p.clear_clipboard)),
            vec![switch(
                tk,
                "set-clipboard",
                p.clear_clipboard,
                &t("settings:privacy.clipboardLabel"),
                true,
                |a, v| a.settings.prefs.clear_clipboard = v,
            )],
            Some("clearClipboard"),
        ))
        .rows(crate::osleaks::ui::privacy_rows(app, tk));
    let notif = group()
        .title(t("settings:privacy.notificationsHeading"))
        .footer(t("settings:privacy.notificationsDisclosure"))
        .row(srow_a(
            app,
            tk,
            t("settings:privacy.notificationsLabel"),
            desc(
                tk,
                t(if p.desktop_notifications {
                    "settings:privacy.notificationsOn"
                } else {
                    "settings:privacy.notificationsOff"
                }),
            ),
            vec![switch(
                tk,
                "set-notifications",
                p.desktop_notifications,
                &t("settings:privacy.notificationsLabel"),
                true,
                |a, v| a.settings.prefs.desktop_notifications = v,
            )],
            Some("desktopNotifications"),
        ));
    // Written to the update preference file the CLI reads (`setAppUpdateChecksEnabled`).
    let updates = group()
        .title(t("settings:privacy.updatesHeading"))
        .footer(t("settings:privacy.updatesTrust"))
        .row(srow_a(
            app,
            tk,
            t("settings:privacy.updatesLabel"),
            desc(
                tk,
                t(if p.update_checks {
                    "settings:privacy.updatesOn"
                } else {
                    "settings:privacy.updatesOff"
                }),
            ),
            vec![switch(
                tk,
                "set-updates",
                p.update_checks,
                &t("settings:privacy.updatesLabel"),
                true,
                super::effects::set_update_checks,
            )],
            Some("updateChecks"),
        ));

    // Outgoing connections: only enabled backends send traffic off the machine.
    let backends = model::backends(
        app.read("ui.backends.settings.list"),
        app.read("ui.maintenance.settings"),
    );
    let enabled: Vec<&Backend> = backends.iter().filter(|b| b.on).collect();
    let count = |f: &str| enabled.iter().filter(|b| exposure(b) == f).count();
    let remote = count("remote");
    let filter = app.settings.privacy_filter;
    let summary = if remote > 0 {
        tc("settings:privacy.thirdPartyWarning", remote, &[])
    } else if app.prefs.ai_features {
        t("settings:privacy.noThirdPartyWithAi")
    } else {
        t("settings:privacy.noThirdParty")
    };
    let list = app.read("ui.backends.settings.list");
    let setup_allows =
        list.is_some() && model::public_fallbacks(&backends, model::bootstrap_mode(list));
    let allowed = p.explorer_fallbacks_allowed;
    let fallback = srow_a(
        app,
        tk,
        t("settings:privacy.explorerFallbacksLabel"),
        desc(
            tk,
            t(if !setup_allows {
                "settings:privacy.explorerFallbacksUnavailable"
            } else if allowed {
                "settings:privacy.explorerFallbacksOn"
            } else {
                "settings:privacy.explorerFallbacksOff"
            }),
        ),
        vec![switch(
            tk,
            "set-explorer-fallbacks",
            setup_allows && allowed,
            &t("settings:privacy.explorerFallbacksLabel"),
            setup_allows,
            |a, v| a.settings.prefs.explorer_fallbacks_allowed = v,
        )],
        Some("explorerFallbacks"),
    );
    let mut opts = vec![opt(
        "all",
        tr(
            "settings:privacy.filterAll",
            &[("count", &enabled.len().to_string())],
        ),
    )];
    for (id, label, _) in FILTERS {
        opts.push(opt(
            id,
            tr(
                "settings:privacy.filterOption",
                &[("label", &t(label)), ("count", &count(id).to_string())],
            ),
        ));
    }
    let filter_sel = select(
        app,
        tk,
        layer,
        "set-exposure-filter",
        &t("settings:privacy.filterLabel"),
        filter.unwrap_or("all"),
        opts,
        true,
        None,
        |a, v| {
            a.settings.privacy_filter = FILTERS.iter().find(|f| f.0 == v).map(|f| f.0);
        },
    );
    let hint = filter
        .and_then(|f| FILTERS.iter().find(|x| x.0 == f))
        .map(|f| t(f.2))
        .unwrap_or_else(|| t("settings:privacy.filterAllHint"));
    let outgoing = group()
        .anchor("outgoingConnections")
        .title(t("settings:privacy.leavesHeading"))
        .footer(t("settings:privacy.leavesDescription"))
        .row(para(
            tk,
            summary,
            if remote > 0 { amber(tk) } else { tk.muted_fg },
        ))
        .row(srow(
            app,
            tk,
            t("settings:privacy.assistantHeading"),
            desc(
                tk,
                t(if app.prefs.ai_features {
                    "settings:privacy.assistantEnabled"
                } else {
                    "settings:privacy.assistantDisabled"
                }),
            ),
            vec![
                btn(
                    tk,
                    "set-review-ai",
                    &t("settings:privacy.reviewAi"),
                    Btn::Outline,
                    true,
                )
                .on_press(|a: &mut App| {
                    super::open_index(a, super::AI);
                }),
            ],
        ))
        .row(fallback)
        .row(srow(
            app,
            tk,
            t("settings:privacy.filterLabel"),
            desc(tk, hint),
            vec![filter_sel],
        ));

    let mut page = col()
        .w_full()
        .gap(tk.r(1.5))
        .child(offline_g.build(app, tk))
        .child(lookup.build(app, tk))
        .child(screen.build(app, tk))
        .child(notif.build(app, tk))
        .child(updates.build(app, tk))
        .child(outgoing.build(app, tk));
    let groups: [(&str, &[Net], &str, &str); 3] = [
        (
            "addresses",
            &[Net::Btc, Net::Liquid],
            "settings:privacy.group.addressesTitle",
            "settings:privacy.group.addressesSubtitle",
        ),
        (
            "lightning",
            &[Net::Ln],
            "settings:privacy.group.lightningTitle",
            "settings:privacy.group.lightningSubtitle",
        ),
        (
            "market",
            &[Net::Fx],
            "settings:privacy.group.marketTitle",
            "settings:privacy.group.marketSubtitle",
        ),
    ];
    for (gid, nets, title, sub) in groups {
        let all: Vec<&&Backend> = enabled
            .iter()
            .filter(|b| nets.contains(&b.net_kind))
            .collect();
        if all.is_empty() {
            continue;
        }
        let rows: Vec<&&Backend> = all
            .into_iter()
            .filter(|b| filter.is_none_or(|f| exposure(b) == f))
            .collect();
        let mut g = group().title(t(title)).footer(t(sub));
        if rows.is_empty() {
            let label = filter
                .and_then(|f| FILTERS.iter().find(|x| x.0 == f))
                .map(|f| t(f.1))
                .unwrap_or_default();
            g = g.row(para(
                tk,
                tr("settings:privacy.noneMatchFilter", &[("label", &label)]),
                tk.muted_fg,
            ));
        }
        for b in rows {
            let trust = model::backend_trust(b);
            let ex = exposure(b);
            let backend = (*b).clone();
            let market = gid == "market";
            let edit = btn(
                tk,
                format!("set-privacy-edit-{}", b.id),
                &t("common:actions.edit"),
                Btn::Outline,
                true,
            )
            .label(tr("settings:privacy.editEndpoint", &[("name", &b.name)]))
            .on_press(move |a: &mut App| {
                if market {
                    super::open_index(a, super::MARKET);
                } else {
                    super::backend::open_edit(a, &backend);
                }
            });
            g = g.row(srow(
                app,
                tk,
                b.name.clone(),
                desc(
                    tk,
                    format!(
                        "{} · {}",
                        model::protocol_label(b),
                        model::host_label(&b.url)
                    ),
                ),
                vec![
                    text(trust.label.clone())
                        .font(tk.tsm(400.0))
                        .color(tone(tk, ex))
                        .help(trust.note.clone()),
                    edit,
                ],
            ));
        }
        page = page.child(g.build(app, tk));
    }
    let _ = Value::Null;
    page
}
