//! Appearance (AppearanceSettingsPanel.tsx): theme, denomination, interface scale, language,
//! and the assistant dock while AI features are on. Every choice drives `app.prefs`.

use rune::{El, col};

use super::controls::{desc, group, opt, select, srow_a};
use crate::app::{App, Currency, ThemeMode};
use crate::i18n::{Lang, t, tr};
use crate::theme::Tk;

pub fn view(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let theme_v = match app.prefs.theme {
        ThemeMode::System => "system",
        ThemeMode::Light => "light",
        ThemeMode::Dark => "dark",
    };
    let theme = select(
        app,
        tk,
        layer,
        "set-theme",
        &t("settings:appearance.theme.title"),
        theme_v,
        vec![
            opt("system", t("settings:appearance.theme.system")),
            opt("light", t("settings:appearance.theme.light")),
            opt("dark", t("settings:appearance.theme.dark")),
        ],
        true,
        None,
        |a, v| {
            a.prefs.theme = match v {
                "system" => ThemeMode::System,
                "light" => ThemeMode::Light,
                _ => ThemeMode::Dark,
            }
        },
    );
    let denom = select(
        app,
        tk,
        layer,
        "set-denom",
        &t("settings:appearance.denomination.title"),
        if app.prefs.currency == Currency::Fiat {
            "eur"
        } else {
            "btc"
        },
        vec![
            opt("eur", t("settings:appearance.denomination.euro")),
            opt("btc", t("settings:appearance.denomination.bitcoin")),
        ],
        true,
        None,
        |a, v| {
            a.prefs.currency = if v == "eur" {
                Currency::Fiat
            } else {
                Currency::Btc
            }
        },
    );
    let pct = (app.prefs.scale * 100.0).round() as u32;
    let scales: Vec<_> = (0..=8)
        .map(|i| 80 + i * 5)
        .map(|p| {
            let label = if p as f32 == (crate::app::DEFAULT_SCALE * 100.0).round() {
                tr(
                    "settings:appearance.scale.defaultValue",
                    &[("percent", &p.to_string())],
                )
            } else {
                tr(
                    "settings:appearance.scale.value",
                    &[("percent", &p.to_string())],
                )
            };
            opt(p.to_string(), label)
        })
        .collect();
    let scale = select(
        app,
        tk,
        layer,
        "set-scale",
        &t("settings:appearance.scale.title"),
        &pct.to_string(),
        scales,
        true,
        None,
        |a, v| {
            if let Ok(p) = v.parse::<f32>() {
                a.prefs.scale = p / 100.0;
            }
        },
    );
    let lang = select(
        app,
        tk,
        layer,
        "set-lang",
        &t("settings:appearance.language.title"),
        app.prefs.lang.code(),
        Lang::ALL.iter().map(|l| opt(l.code(), l.label())).collect(),
        true,
        None,
        |a, v| {
            if let Some(l) = Lang::ALL.iter().find(|l| l.code() == v) {
                a.prefs.lang = *l;
            }
        },
    );
    let main = group()
        .row(srow_a(
            app,
            tk,
            t("settings:appearance.theme.title"),
            desc(tk, t("settings:appearance.theme.description")),
            vec![theme],
            Some("theme"),
        ))
        .row(srow_a(
            app,
            tk,
            t("settings:appearance.denomination.title"),
            desc(tk, t("settings:appearance.denomination.description")),
            vec![denom],
            Some("denomination"),
        ))
        .row(srow_a(
            app,
            tk,
            t("settings:appearance.scale.title"),
            desc(tk, t("settings:appearance.scale.description")),
            vec![scale],
            Some("interfaceScale"),
        ))
        .row(srow_a(
            app,
            tk,
            t("settings:appearance.language.title"),
            desc(tk, t("settings:appearance.language.description")),
            vec![lang],
            Some("language"),
        ))
        .build(app, tk);
    let mut page = col().w_full().gap(tk.r(1.5)).child(main);
    if app.prefs.ai_features {
        let closed = select(
            app,
            tk,
            layer,
            "set-closed-bar",
            &t("settings:appearance.assistantDock.closedBar"),
            app.settings.prefs.closed_bar,
            vec![
                opt(
                    "composer",
                    t("settings:appearance.assistantDock.closedBarComposer"),
                ),
                opt(
                    "launcher",
                    t("settings:appearance.assistantDock.closedBarLauncher"),
                ),
                opt(
                    "hidden",
                    t("settings:appearance.assistantDock.closedBarHidden"),
                ),
            ],
            true,
            None,
            |a, v| {
                a.settings.prefs.closed_bar = match v {
                    "composer" => "composer",
                    "launcher" => "launcher",
                    _ => "hidden",
                }
            },
        );
        let mut dock = group()
            .anchor("assistantDock")
            .title(t("settings:appearance.assistantDock.title"))
            .row(srow_a(
                app,
                tk,
                t("settings:appearance.assistantDock.closedBar"),
                desc(
                    tk,
                    t("settings:appearance.assistantDock.closedBarDescription"),
                ),
                vec![closed],
                None,
            ));
        if app.settings.prefs.closed_bar != "hidden" {
            let pos = select(
                app,
                tk,
                layer,
                "set-dock-pos",
                &t("settings:appearance.assistantDock.position"),
                app.settings.prefs.dock_position,
                vec![
                    opt("left", t("settings:appearance.assistantDock.positionLeft")),
                    opt(
                        "center",
                        t("settings:appearance.assistantDock.positionCenter"),
                    ),
                    opt(
                        "right",
                        t("settings:appearance.assistantDock.positionRight"),
                    ),
                ],
                true,
                None,
                |a, v| {
                    a.settings.prefs.dock_position = match v {
                        "left" => "left",
                        "center" => "center",
                        _ => "right",
                    }
                },
            );
            dock = dock.row(srow_a(
                app,
                tk,
                t("settings:appearance.assistantDock.position"),
                None,
                vec![pos],
                None,
            ));
        }
        page = page.child(dock.build(app, tk));
    }
    page
}
