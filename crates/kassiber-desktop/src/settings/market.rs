//! Market data (MarketDataSettingsPanel.tsx): live minute-rate opt-in, price source, cache
//! rebuild (egress, behind its dialog), pricing policy switches, rate providers and the Kraken
//! offline imports.

use rune::{El, FileDialogOptions, TextEdit, Tx, col, row, text};
use serde_json::{Value, json};

use super::controls::{
    Btn, PressIf, amber, btn, desc, emerald, error_line, group, input, opt, select, srow, srow_a,
    switch,
};
use super::model;
use super::send;
use crate::app::{App, Reply, ToastTone};
use crate::i18n::{t, tc, tr};
use crate::theme::Tk;

#[derive(Default)]
pub struct Market {
    pub kraken_path: TextEdit,
    pub import_result: Option<Value>,
    pub import_error: Option<String>,
    /// ("full" | "incremental", bundled) while an import runs.
    pub pending: Option<(&'static str, bool)>,
    pub rebuild_result: Option<Value>,
    pub rebuild_error: Option<String>,
    pub rebuild_open: bool,
}

fn settings(app: &App) -> Option<&Value> {
    app.read("ui.maintenance.settings")
        .map(|m| &m["settings"])
        .filter(|s| !s.is_null())
}

fn provider(app: &App) -> String {
    settings(app)
        .and_then(|s| s["market_rate_provider"].as_str())
        .unwrap_or("coinbase-exchange")
        .to_owned()
}

pub fn live_enabled(app: &App) -> bool {
    settings(app)
        .and_then(|s| s["source_classes"]["market_rates"].as_bool())
        .unwrap_or(false)
}

fn busy(app: &App) -> bool {
    let m = app.read("ui.maintenance.settings");
    (app.settings.is_busy("settings.maintenance") && m.is_none())
        || app.settings.is_busy("settings.configure")
        || m.is_none_or(|m| m["profile"].is_null())
}

pub fn active_pair(app: &App) -> String {
    settings(app)
        .and_then(|s| s["active_rate_pair"].as_str())
        .unwrap_or("BTC-fiat")
        .to_owned()
}

/// What a configure request was for, so its reply picks the right notice.
#[derive(Clone, Copy)]
enum Change {
    Live(bool),
    Provider,
    Coarse(bool),
    Rails(bool),
}

fn configure(app: &mut App, args: Value, change: Change) {
    app.settings.market_change = Some(change_tag(change));
    send(app, "ui.maintenance.configure", args, "settings.configure");
}

fn change_tag(c: Change) -> (&'static str, bool) {
    match c {
        Change::Live(on) => ("live", on),
        Change::Provider => ("provider", true),
        Change::Coarse(on) => ("coarse", on),
        Change::Rails(on) => ("rails", on),
    }
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    match tag {
        "settings.configure" => {
            let change = app.settings.market_change.take();
            match (&r.error, change) {
                (None, Some((what, on))) => {
                    let (title, body) = match (what, on) {
                        ("live", true) => (
                            t("settings:marketData.autoEnabledTitle"),
                            t("settings:marketData.autoEnabledBody"),
                        ),
                        ("live", false) => (
                            t("settings:marketData.autoDisabledTitle"),
                            t("settings:marketData.autoDisabledBody"),
                        ),
                        ("provider", _) => {
                            let p = r.data["settings"]["market_rate_provider"]
                                .as_str()
                                .map(model::provider_label)
                                .unwrap_or_default();
                            (
                                t("settings:marketData.providerUpdatedTitle"),
                                tr(
                                    "settings:marketData.providerUpdatedBody",
                                    &[("provider", &p)],
                                ),
                            )
                        }
                        ("coarse", true) => (
                            t("settings:marketData.coarse.heldTitle"),
                            t("settings:marketData.coarse.heldBody"),
                        ),
                        ("coarse", false) => (
                            t("settings:marketData.coarse.acceptedTitle"),
                            t("settings:marketData.coarse.acceptedBody"),
                        ),
                        ("rails", true) => (
                            t("settings:marketData.bitcoinRails.enabledTitle"),
                            t("settings:marketData.bitcoinRails.enabledBody"),
                        ),
                        _ => (
                            t("settings:marketData.bitcoinRails.disabledTitle"),
                            t("settings:marketData.bitcoinRails.disabledBody"),
                        ),
                    };
                    app.toast(ToastTone::Success, &title, Some(body), None);
                    // These two change how the book's rows are priced and held: its reads
                    // (readiness, journals state) refresh.
                    if matches!(what, "coarse" | "rails") {
                        app.refresh_reads();
                    }
                }
                (Some(e), Some((what, _))) => {
                    let title = match what {
                        "live" => t("settings:marketData.autoFailedTitle"),
                        "provider" => t("settings:marketData.providerFailedTitle"),
                        "coarse" => t("settings:marketData.coarse.errorTitle"),
                        _ => t("settings:marketData.bitcoinRails.errorTitle"),
                    };
                    app.toast(ToastTone::Error, &title, Some(e.clone()), None);
                }
                _ => {}
            }
            send(
                app,
                "ui.maintenance.settings",
                Value::Null,
                "settings.maintenance",
            );
        }
        "settings.kraken" => {
            app.settings.market.pending = None;
            match &r.error {
                None => app.settings.market.import_result = Some(r.data.clone()),
                Some(e) => app.settings.market.import_error = Some(e.clone()),
            }
        }
        "settings.rebuild" => match &r.error {
            None => {
                app.settings.market.rebuild_open = false;
                let d = r.data.clone();
                let fetched: f64 = d["sync"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|r| r["samples"].as_f64().unwrap_or(0.0))
                    .sum();
                let summary = tr(
                    "settings:marketData.rebuiltBody",
                    &[
                        (
                            "cleared",
                            &crate::format::count(
                                d["deleted"]["transaction_prices"].as_f64().unwrap_or(0.0) as usize,
                            ),
                        ),
                        ("fetched", &crate::format::count(fetched as usize)),
                        ("blocker", ""),
                    ],
                );
                app.settings.market.rebuild_result = Some(d);
                app.shell.note(
                    &t("settings:marketData.resultRebuilt"),
                    Some(summary),
                    ToastTone::Success,
                    app.clock,
                );
            }
            Some(e) => app.settings.market.rebuild_error = Some(e.clone()),
        },
        _ => {}
    }
}

/// Sends the Kraken import (`startKrakenImport` once a path is known).
fn run_import(app: &mut App, op: &'static str, bundled: bool) {
    let path = app.settings.market.kraken_path.text.trim().to_owned();
    app.settings.market.import_error = None;
    app.settings.market.import_result = None;
    if !bundled && path.is_empty() {
        app.settings.market.import_error = Some(t("settings:marketData.importPathError"));
        return;
    }
    app.settings.market.pending = Some((op, bundled));
    let args = if bundled {
        json!({"use_bundled": true, "operation": op})
    } else {
        json!({"path": path, "operation": op})
    };
    send(app, "ui.rates.kraken_csv.import", args, "settings.kraken");
}

/// A full import without a path asks for the extracted folder; an update for a file. The
/// dialog picks a real path, so it is wired but not exercised in tests.
fn start_import(app: &mut App, tx: &mut Tx, op: &'static str) {
    if app.settings.market.kraken_path.text.trim().is_empty() {
        let title = t(if op == "full" {
            "settings:marketData.pickFullTitle"
        } else {
            "settings:marketData.pickUpdateTitle"
        });
        let opts = FileDialogOptions {
            title,
            ..Default::default()
        };
        let answer = move |a: &mut App, r: rune::FileDialogResult, _: &mut Tx| {
            if let Ok(Some(p)) = r {
                a.settings.market.kraken_path = TextEdit::new(p.to_string_lossy().into_owned());
                run_import(a, op, false);
            }
        };
        if op == "full" {
            tx.open_folder(opts, answer);
        } else {
            tx.open_file(
                opts.filter(
                    t("settings:marketData.pickArchiveFilterName"),
                    &["zip", "csv"],
                ),
                answer,
            );
        }
        return;
    }
    run_import(app, op, false);
}

pub fn view(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let m = &app.settings.market;
    let live = live_enabled(app);
    let busy = busy(app);
    let prov = provider(app);
    let prov_label = model::provider_label(&prov);
    let pair = active_pair(app);
    let importing = app.settings.is_busy("settings.kraken");
    let rebuilding = app.settings.is_busy("settings.rebuild");
    let providers: Vec<String> = settings(app)
        .and_then(|s| s["market_rate_providers"].as_array())
        .filter(|a| !a.is_empty())
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_else(|| {
            model::MARKET_PROVIDERS
                .iter()
                .map(|s| s.to_string())
                .collect()
        });

    let live_sw = switch(
        tk,
        "set-market-live",
        live,
        &t("settings:marketData.autoRefreshLabel"),
        !busy,
        |a, on| {
            let Some(s) = settings(a).cloned() else {
                return;
            };
            let mut classes = s["source_classes"].clone();
            if !classes.is_object() {
                classes = json!({});
            }
            classes["market_rates"] = json!(on);
            let any = classes
                .as_object()
                .is_some_and(|o| o.values().any(|v| v.as_bool() == Some(true)));
            let background = if on {
                true
            } else {
                s["background_enabled"].as_bool().unwrap_or(false) && any
            };
            configure(
                a,
                json!({"background_enabled": background, "source_classes": classes}),
                Change::Live(on),
            );
        },
    );
    let source = select(
        app,
        tk,
        layer,
        "set-market-provider",
        &t("settings:marketData.priceSourceLabel"),
        &prov,
        providers
            .iter()
            .map(|p| opt(p.clone(), model::provider_label(p)))
            .collect(),
        !busy,
        None,
        |a, v| {
            if v != provider(a) {
                configure(a, json!({"market_rate_provider": v}), Change::Provider);
            }
        },
    );
    let can_rebuild = !rebuilding && !importing && live;
    let rebuild_btn = btn(
        tk,
        "set-market-rebuild",
        &t(if rebuilding {
            "settings:marketData.rebuildDialogPending"
        } else {
            "settings:marketData.rebuildButton"
        }),
        Btn::Outline,
        can_rebuild,
    )
    .press_if(can_rebuild, |a| {
        a.settings.market.rebuild_error = None;
        a.settings.market.rebuild_open = true;
    });
    let mut live_group = group()
        .title(t("settings:marketData.liveHeading"))
        .row(srow_a(
            app,
            tk,
            t("settings:marketData.autoRefreshLabel"),
            desc(
                tk,
                tr(
                    "settings:marketData.autoRefreshHint",
                    &[("provider", &prov_label)],
                ),
            ),
            vec![live_sw],
            Some("liveRates"),
        ))
        .row(srow_a(
            app,
            tk,
            t("settings:marketData.priceSourceLabel"),
            desc(tk, t("settings:marketData.priceSourceHint")),
            vec![source],
            Some("priceSource"),
        ))
        .row(srow_a(
            app,
            tk,
            t("settings:marketData.rebuildHeading"),
            desc(
                tk,
                if live {
                    tr("settings:marketData.rebuildDescription", &[("pair", &pair)])
                } else {
                    t("settings:marketData.rebuildOptInRequired")
                },
            ),
            vec![rebuild_btn],
            Some("rebuildPricing"),
        ));
    if rebuilding {
        live_group = live_group.row(
            col()
                .w_full()
                .px(tk.r(1.0))
                .py(tk.r(0.75))
                .gap(tk.r(0.25))
                .role("status")
                .child(
                    text(format!(
                        "{} · {}",
                        t("settings:marketData.rebuildingTitle"),
                        t("settings:marketData.rebuildingCounting")
                    ))
                    .font(tk.tsm(400.0))
                    .color(tk.fg)
                    .wrap(),
                )
                .child(
                    text(t("settings:marketData.rebuildingFootnote"))
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg)
                        .wrap(),
                ),
        );
    }
    if let Some(res) = &m.rebuild_result {
        live_group = live_group.row(super::controls::dismissable(
            tk,
            "set-rebuild-result-dismiss",
            rebuild_result(tk, res, &pair),
            tk.muted_fg,
            |a| a.settings.market.rebuild_result = None,
        ));
    }

    let coarse = settings(app)
        .and_then(|s| s["require_coarse_review"].as_bool())
        .unwrap_or(false);
    let rails = settings(app)
        .and_then(|s| s["bitcoin_rail_carrying_value"].as_bool())
        .unwrap_or(true);
    let coarse_count = settings(app)
        .and_then(|s| s["coarse_priced_count"].as_u64())
        .unwrap_or(0) as usize;
    let mut policy = group()
        .title(t("settings:marketData.policyHeading"))
        .row(srow_a(
            app,
            tk,
            t("settings:marketData.coarse.reviewToggleLabel"),
            desc(tk, t("settings:marketData.coarse.reviewToggleDescription")),
            vec![switch(
                tk,
                "set-market-coarse",
                coarse,
                &t("settings:marketData.coarse.reviewToggleLabel"),
                !busy,
                move |a, on| {
                    if on != coarse {
                        configure(a, json!({"require_coarse_review": on}), Change::Coarse(on));
                    }
                },
            )],
            Some("coarsePricingReview"),
        ));
    if coarse_count > 0 && !coarse {
        policy = policy.row(srow(
            app,
            tk,
            tc("settings:marketData.coarse.countNotice", coarse_count, &[]),
            desc(tk, t("settings:marketData.coarse.countDescription")),
            vec![],
        ));
    }
    policy = policy.row(srow_a(
        app,
        tk,
        t("settings:marketData.bitcoinRails.toggleLabel"),
        desc(tk, t("settings:marketData.bitcoinRails.toggleDescription")),
        vec![switch(
            tk,
            "set-market-rails",
            rails,
            &t("settings:marketData.bitcoinRails.toggleLabel"),
            !busy,
            move |a, on| {
                if on != rails {
                    configure(
                        a,
                        json!({"bitcoin_rail_carrying_value": on}),
                        Change::Rails(on),
                    );
                }
            },
        )],
        Some("bitcoinRails"),
    ));

    let backends = model::backends(
        app.read("ui.backends.settings.list"),
        app.read("ui.maintenance.settings"),
    );
    let fx: Vec<_> = backends
        .iter()
        .filter(|b| b.net_kind == model::Net::Fx)
        .collect();
    // Rate providers and the offline imports are rare: behind Advanced, with the cache note.
    let mut extra = vec![
        col().w_full().px(tk.r(0.25)).child(
            text(t("settings:marketData.intro"))
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .wrap(),
        ),
    ];
    if !fx.is_empty() {
        let mut g = group().title(t("settings:marketData.providersHeading"));
        for (i, b) in fx.iter().enumerate() {
            let role = t(if i == 0 {
                "settings:marketData.primary"
            } else {
                "settings:marketData.fallback"
            });
            g = g.row(srow(
                app,
                tk,
                b.name.clone(),
                Some(
                    text(format!("{role} · {}", b.url))
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg)
                        .truncate(),
                ),
                vec![
                    text(b.health.clone())
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg),
                ],
            ));
        }
        extra.push(g.build(app, tk));
    }
    extra.push(kraken(app, tk));
    col()
        .w_full()
        .gap(tk.r(1.5))
        .child(live_group.build(app, tk))
        .child(policy.build(app, tk))
        .child(super::controls::advanced(app, tk, ADVANCED, extra))
}

/// The Advanced fold's open-state key (`search::reveal_key`).
pub const ADVANCED: &str = "set-adv-market";

fn rebuild_result(tk: &Tk, res: &Value, pair: &str) -> El {
    let blocker = match &res["journals"] {
        j if j.get("ok").and_then(Value::as_bool) == Some(false) => Some(
            j["error"]["message"]
                .as_str()
                .unwrap_or("Journal processing is still blocked.")
                .to_owned(),
        ),
        _ => None,
    };
    let journal = if res["journals"].get("ok").is_some() {
        &res["journals"]["result"]
    } else {
        &res["journals"]
    };
    let refreshed_src = res["reprice"]["auto_priced"]
        .as_f64()
        .or(journal["auto_priced"].as_f64())
        .unwrap_or(0.0);
    let deleted_prices = res["deleted"]["transaction_prices"].as_f64().unwrap_or(0.0);
    let refreshed = if refreshed_src != 0.0 {
        refreshed_src
    } else {
        deleted_prices
    };
    let total = refreshed.max(deleted_prices);
    let samples: f64 = res["sync"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| r["samples"].as_f64().unwrap_or(0.0))
        .sum();
    let c = |v: f64| crate::format::count(v as usize);
    let headline = if blocker.is_some() {
        t("settings:marketData.resultBlocked")
    } else if total > 0.0 {
        tr(
            "settings:marketData.resultProgress",
            &[("refreshed", &c(refreshed)), ("total", &c(total))],
        )
    } else {
        t("settings:marketData.resultRebuilt")
    };
    let mut col_ = col()
        .w_full()
        .px(tk.r(1.0))
        .py(tk.r(0.75))
        .gap(tk.r(0.25))
        .child(
            text(headline)
                .font(tk.tsm(400.0))
                .color(if blocker.is_some() {
                    amber(tk)
                } else {
                    emerald(tk)
                })
                .wrap(),
        );
    if let Some(b) = blocker {
        col_ = col_.child(text(b).font(tk.tsm(400.0)).color(amber(tk)).wrap());
    }
    let rows = tr(
        "settings:marketData.resultRowsFetched",
        &[
            ("pair", res["pair"].as_str().unwrap_or(pair)),
            ("rows", &c(samples)),
        ],
    );
    let removed = tr(
        "settings:marketData.resultRemoved",
        &[
            ("rates", &c(res["deleted"]["rates"].as_f64().unwrap_or(0.0))),
            (
                "minutes",
                &c(res["deleted"]["checked_minutes"].as_f64().unwrap_or(0.0)),
            ),
            ("prices", &c(deleted_prices)),
        ],
    );
    col_.child(
        text(format!("{rows} · {removed}"))
            .font(tk.tsm(400.0))
            .color(tk.muted_fg)
            .wrap(),
    )
}

/// Opens a market-data link in the browser (an explicit press; the web opens it externally).
fn link(tk: &Tk, key: &str, label: String, url: &'static str) -> El {
    text(label.clone())
        .font(tk.txs(400.0))
        .color(tk.muted_fg)
        .key(key.to_owned())
        .role("link")
        .label(label)
        .focusable(true)
        .on_press_tx(move |a: &mut App, tx: &mut Tx| {
            if tx.open_url(url).is_err() {
                a.toast(
                    ToastTone::Warning,
                    &t("settings:marketData.openLinkErrorTitle"),
                    Some(t("settings:marketData.openLinkErrorBody")),
                    None,
                );
            }
        })
}

fn kraken(app: &App, tk: &Tk) -> El {
    let m = &app.settings.market;
    let importing = app.settings.is_busy("settings.kraken");
    let label_for = |op: &str, normal: &str| {
        if m.pending == Some((if op == "full" { "full" } else { "incremental" }, false)) {
            t("settings:marketData.importing")
        } else {
            t(normal)
        }
    };
    let full = btn(
        tk,
        "set-kraken-full",
        &label_for("full", "settings:marketData.fullMinuteHistory"),
        Btn::Outline,
        !importing,
    );
    let inc = btn(
        tk,
        "set-kraken-inc",
        &label_for("incremental", "settings:marketData.incrementalMinuteUpdate"),
        Btn::Outline,
        !importing,
    );
    let (full, inc) = if importing {
        (full, inc)
    } else {
        (
            full.on_press_tx(|a: &mut App, tx: &mut Tx| start_import(a, tx, "full")),
            inc.on_press_tx(|a: &mut App, tx: &mut Tx| start_import(a, tx, "incremental")),
        )
    };
    let path = input(
        tk,
        &m.kraken_path,
        "set-kraken-path",
        &t("settings:marketData.krakenPathAria"),
        &t("settings:marketData.krakenPathPlaceholder"),
        false,
        !importing,
        tk.r(2.0),
        false,
        |a: &mut App| &mut a.settings.market.kraken_path,
    )
    .grow(1.0)
    .min_w(0.0);
    let pick = |key: &str, label: String, title: String, folder: bool| {
        let b = btn(tk, key, &label, Btn::Outline, !importing).help(title.clone());
        if importing {
            return b;
        }
        b.on_press_tx(move |a: &mut App, tx: &mut Tx| {
            a.settings.market.import_error = None;
            let answer = |a: &mut App, r: rune::FileDialogResult, _: &mut Tx| {
                if let Ok(Some(p)) = r {
                    a.settings.market.kraken_path = TextEdit::new(p.to_string_lossy().into_owned());
                }
            };
            if folder {
                tx.open_folder(
                    FileDialogOptions {
                        title: t("settings:marketData.pickFolderTitle"),
                        ..Default::default()
                    },
                    answer,
                );
            } else {
                tx.open_file(
                    FileDialogOptions {
                        title: t("settings:marketData.pickArchiveTitle"),
                        ..Default::default()
                    }
                    .filter(
                        t("settings:marketData.pickArchiveFilterName"),
                        &["zip", "csv"],
                    ),
                    answer,
                );
            }
        })
    };
    let path_row = row()
        .w_full()
        .px(tk.r(1.0))
        .py(tk.r(0.75))
        .gap(tk.r(0.5))
        .center_y()
        .child(path)
        .child(pick(
            "set-kraken-file",
            t("settings:marketData.chooseFile"),
            t("settings:marketData.chooseFileTitle"),
            false,
        ))
        .child(pick(
            "set-kraken-folder",
            t("settings:marketData.chooseFolder"),
            t("settings:marketData.chooseFolderTitle"),
            true,
        ));
    let bundled_label = if m.pending.is_some_and(|p| p.1) {
        t("settings:marketData.importing")
    } else {
        t("settings:marketData.importDailyValues")
    };
    let bundled = btn(
        tk,
        "set-kraken-bundled",
        &bundled_label,
        Btn::Outline,
        !importing,
    )
    .press_if(!importing, |a| run_import(a, "full", true));
    let footer = row()
        .gap(tk.r(0.25))
        .center_y()
        .child(link(
            tk,
            "set-kraken-archive",
            t("settings:marketData.krakenGetArchive"),
            model::KRAKEN_OHLCVT_SUPPORT_URL,
        ))
        .child(text(" · ").font(tk.txs(400.0)).color(tk.muted_fg))
        .child(link(
            tk,
            "set-kraken-blog",
            t("settings:marketData.krakenBlog"),
            model::KRAKEN_MARKET_DATA_BLOG_URL,
        ));
    let mut g = group()
        .anchor("krakenHistory")
        .title(t("settings:marketData.krakenHeading"))
        .footer_el(footer)
        .row(srow(
            app,
            tk,
            t("settings:marketData.krakenMinuteHeading"),
            desc(tk, t("settings:marketData.krakenMinuteDescription")),
            vec![full, inc],
        ))
        .row(path_row)
        .row(srow_a(
            app,
            tk,
            t("settings:marketData.bundledHeading"),
            desc(tk, t("settings:marketData.bundledDescription")),
            vec![bundled],
            Some("bundledRates"),
        ));
    if let Some(e) = &m.import_error {
        let row_ = col()
            .w_full()
            .px(tk.r(1.0))
            .py(tk.r(0.75))
            .child(error_line(tk, e.clone()));
        g = g.row(super::controls::dismissable(
            tk,
            "set-kraken-error-dismiss",
            row_,
            tk.destructive,
            |a| a.settings.market.import_error = None,
        ));
    }
    if let Some(res) = &m.import_result {
        let totals = &res["totals"];
        let pairs = totals["pairs"].as_u64().unwrap_or(0) as usize;
        let c = |v: &Value| crate::format::count(v.as_f64().unwrap_or(0.0) as usize);
        let headline = if pairs > 0 {
            tc(
                "settings:marketData.importedTotals",
                pairs,
                &[
                    ("rows", &c(&totals["samples"])),
                    ("pairs", &crate::format::count(pairs)),
                ],
            )
        } else {
            t("settings:marketData.noRowsImported")
        };
        let mut col_ = col()
            .w_full()
            .px(tk.r(1.0))
            .py(tk.r(0.75))
            .gap(tk.r(0.25))
            .child(text(headline).font(tk.tsm(400.0)).color(if pairs > 0 {
                emerald(tk)
            } else {
                amber(tk)
            }));
        for r in res["summary"].as_array().into_iter().flatten() {
            let range = match (r["first_timestamp"].as_str(), r["last_timestamp"].as_str()) {
                (Some(a), Some(b)) => format!("{a} to {b}"),
                _ => "No imported rows".into(),
            };
            let gran = tr(
                "settings:marketData.importedRowGranularity",
                &[
                    ("rows", &c(&r["samples"])),
                    ("granularity", r["granularity"].as_str().unwrap_or("")),
                ],
            );
            col_ = col_.child(
                text(format!(
                    "{} · {range} · {gran}",
                    r["pair"].as_str().unwrap_or("")
                ))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .wrap(),
            );
        }
        if totals["skipped_rows"].as_u64().unwrap_or(0) > 0
            || totals["skipped_files"].as_u64().unwrap_or(0) > 0
        {
            col_ = col_.child(
                text(tr(
                    "settings:marketData.skipped",
                    &[
                        ("rows", &c(&totals["skipped_rows"])),
                        ("files", &c(&totals["skipped_files"])),
                    ],
                ))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg),
            );
        }
        g = g.row(super::controls::dismissable(
            tk,
            "set-kraken-result-dismiss",
            col_,
            tk.muted_fg,
            |a| a.settings.market.import_result = None,
        ));
    }
    g.build(app, tk)
}

/// The rebuild confirmation (`Dialog` in the panel). Rebuilding contacts the selected provider.
pub fn rebuild_dialog(app: &App, tk: &Tk, layer: &mut Vec<El>) {
    use super::controls::{btn_lg, dialog, dialog_title, muted};
    let m = &app.settings.market;
    let rebuilding = app.settings.is_busy("settings.rebuild");
    let live = live_enabled(app);
    let prov_label = model::provider_label(&provider(app));
    let pair = active_pair(app);
    let mut c = col()
        .w_full()
        .gap(tk.r(1.0))
        .child(
            col()
                .gap(tk.r(0.5))
                .child(dialog_title(
                    tk,
                    t("settings:marketData.rebuildDialogTitle"),
                ))
                .child(muted(
                    tk,
                    tr(
                        "settings:marketData.rebuildDialogDescription",
                        &[("provider", &prov_label), ("pair", &pair)],
                    ),
                )),
        )
        .child(
            text(format!(
                "{} {}",
                t("settings:marketData.rebuildDialogWarningTitle"),
                t("settings:marketData.rebuildDialogWarningBody")
            ))
            .font(tk.tsm(400.0))
            .color(super::controls::amber_soft(tk))
            .wrap(),
        );
    if let Some(e) = &m.rebuild_error {
        c = c.child(error_line(tk, e.clone()));
    }
    let ok = !rebuilding && live;
    c = c.child(
        row()
            .w_full()
            .gap(tk.r(0.5))
            .justify_end()
            .child(
                btn_lg(
                    tk,
                    "set-rebuild-cancel",
                    &t("common:actions.cancel"),
                    Btn::Outline,
                    !rebuilding,
                )
                .press_if(!rebuilding, |a| a.settings.market.rebuild_open = false),
            )
            .child(
                btn_lg(
                    tk,
                    "set-rebuild-go",
                    &t(if rebuilding {
                        "settings:marketData.rebuildDialogPending"
                    } else {
                        "settings:marketData.rebuildDialogSubmit"
                    }),
                    Btn::Primary,
                    ok,
                )
                .press_if(ok, |a| {
                    a.settings.market.rebuild_error = None;
                    a.settings.market.rebuild_result = None;
                    let source = provider(a);
                    send(
                        a,
                        "ui.rates.rebuild",
                        json!({"source": source, "reprice_transactions": true}),
                        "settings.rebuild",
                    );
                }),
            ),
    );
    dialog(
        app,
        tk,
        layer,
        "set-rebuild-dialog",
        tk.r(32.0),
        c.label(t("settings:marketData.rebuildDialogTitle")),
        |a| a.settings.market.rebuild_open = false,
        true,
    );
}
