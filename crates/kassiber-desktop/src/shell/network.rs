//! The network status button and "Outbound connections" dropdown (NetworkStatusIndicator.tsx).
//! Mount reads are local (`ui.network.offline`, `ui.backends.settings.list`); probes run only
//! when the user presses "Check connections", one backend at a time.

use rune::{El, col, row, text};
use rune_kit::Placement;
use serde_json::{Value, json};

use super::{Popup, Probe, mount_popup};
use crate::app::{App, Reply};
use crate::i18n::t;
use crate::icons as I;
use crate::theme::{Tk, Tone};
use crate::ui::{dot, ic};

fn offline(app: &App) -> bool {
    app.read("ui.network.offline")
        .is_some_and(|v| v["offline"] == true || v["environment_blocked"] == true)
}

fn backends(app: &App) -> Vec<Value> {
    app.read("ui.backends.settings.list")
        .and_then(|v| v["backends"].as_array())
        .cloned()
        .unwrap_or_default()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Offline,
    Nothing,
    Failures,
    Issue,
    Online,
    Unchecked,
}

fn state(app: &App) -> State {
    if offline(app) {
        return State::Offline;
    }
    if backends(app).is_empty() {
        return State::Nothing;
    }
    let probes = &app.shell.probes;
    let failed = probes
        .iter()
        .filter(|(_, p)| matches!(p, Probe::Failed(_)))
        .count();
    let ok = probes.iter().filter(|(_, p)| *p == Probe::Ok).count();
    if failed > 0 && ok == 0 {
        State::Failures
    } else if failed > 0 {
        State::Issue
    } else if ok > 0 {
        State::Online
    } else {
        State::Unchecked
    }
}

fn label(s: State) -> String {
    t(match s {
        State::Offline => "chrome:network.indicator.offlineMode",
        State::Nothing => "chrome:network.indicator.none",
        State::Failures => "chrome:network.indicator.errors",
        State::Issue => "chrome:network.indicator.warning",
        State::Online => "chrome:network.indicator.online",
        State::Unchecked => "chrome:network.indicator.neutral",
    })
}

/// The request a backend's probe sends (`connectionProbeRequest`).
fn probe_request(b: &Value) -> (String, Value) {
    let name = b["name"].as_str().unwrap_or_default();
    let url = b["url"].as_str().unwrap_or_default();
    let proxy = b["tor_proxy"].as_str().filter(|p| !p.is_empty());
    match b["kind"].as_str().unwrap_or_default() {
        "electrum" => (
            "ui.backends.electrum.test".into(),
            json!({"url": url, "proxy": proxy, "timeout": 5}),
        ),
        "bitcoinrpc" => (
            "ui.backends.bitcoinrpc.test".into(),
            json!({"backend": name, "proxy": proxy, "timeout": 5}),
        ),
        "coreln" | "core-ln" | "lnd" | "nwc" | "lightning" => (
            "ui.backends.lightning.test".into(),
            json!({"backend": name, "timeout": 5}),
        ),
        "btcpay" => (
            "ui.backends.btcpay.test".into(),
            json!({"backend": name, "timeout": 5}),
        ),
        _ => (
            "ui.backends.http.test".into(),
            json!({"url": url, "proxy": proxy, "timeout": 5}),
        ),
    }
}

/// "Check connections": queue every backend and send the first probe. User action only.
pub fn check_connections(app: &mut App) {
    if app.shell.probing || offline(app) {
        return;
    }
    app.shell.probes = backends(app)
        .iter()
        .map(|b| {
            (
                b["name"].as_str().unwrap_or_default().to_owned(),
                Probe::Waiting,
            )
        })
        .collect();
    app.shell.probing = true;
    next_probe(app);
}

fn next_probe(app: &mut App) {
    let Some(i) = app
        .shell
        .probes
        .iter()
        .position(|(_, p)| *p == Probe::Waiting)
    else {
        app.shell.probing = false;
        return;
    };
    let name = app.shell.probes[i].0.clone();
    let Some(b) = backends(app)
        .into_iter()
        .find(|b| b["name"] == name.as_str())
    else {
        app.shell.probes[i].1 = Probe::Failed(t("chrome:network.checkFailed"));
        return next_probe(app);
    };
    app.shell.probes[i].1 = Probe::Running;
    let (kind, args) = probe_request(&b);
    if app.send(&kind, args, "shell.probe").is_none() {
        app.shell.probes[i].1 = Probe::Failed(t("chrome:network.checkFailed"));
        app.shell.probing = false;
    }
}

pub fn on_probe(app: &mut App, r: &Reply) {
    if let Some(p) = app
        .shell
        .probes
        .iter_mut()
        .find(|(_, p)| *p == Probe::Running)
    {
        let ok = r.error.is_none() && r.data["ok"].as_bool().unwrap_or(true);
        p.1 = if ok {
            Probe::Ok
        } else {
            Probe::Failed(
                r.error
                    .clone()
                    .or_else(|| r.data["error"].as_str().map(str::to_owned))
                    .unwrap_or_else(|| t("chrome:network.checkFailed")),
            )
        };
    }
    next_probe(app);
}

pub fn on_offline_set(app: &mut App, r: &Reply) {
    if let Some(e) = &r.error {
        app.toast(
            crate::app::ToastTone::Error,
            &t("chrome:network.offlineMode.failed"),
            Some(e.clone()),
            Some("offline"),
        );
    }
    app.send("ui.network.offline", Value::Null, "app.offline");
}

fn tone(tk: &Tk, s: State) -> Tone {
    match s {
        State::Failures => tk.alert,
        State::Issue => tk.warn,
        State::Online => Tone {
            bg: tk.accent.alpha(0.4),
            border: tk.good.solid.alpha(0.2),
            text: tk.good.text,
            solid: tk.good.solid,
        },
        _ => Tone {
            bg: tk.accent.alpha(0.3),
            border: tk.border,
            text: tk.muted_fg,
            solid: tk.muted_fg,
        },
    }
}

pub fn button(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let s = state(app);
    let tn = tone(tk, s);
    let open = app.shell.popup == Some(Popup::Network);
    let glyph = if matches!(s, State::Offline | State::Nothing) {
        &I::WIFI_OFF
    } else {
        &I::WIFI
    };
    let b = row()
        .key("sh-network")
        .size(tk.r(2.0), tk.r(2.0))
        .center()
        .radius(tk.r(0.375))
        .fill(tn.bg)
        .border(1.0, tn.border)
        .border_layout()
        .hover(tk.row_hover)
        .role("button")
        .label(label(s))
        .help(label(s))
        .expanded(open)
        .focusable(true)
        .on_press(|s: &mut App| s.shell.toggle(Popup::Network))
        .child(ic(glyph, tk.r(1.0), tn.text));
    if open {
        mount_popup(
            tk,
            layer,
            "sh-network-pop",
            "sh-network",
            Placement::BottomEnd,
            panel(app, tk),
        );
    }
    b
}

/// `backendProtocolLabel` (settings/SettingsModel.ts).
fn protocol_label(b: &Value) -> String {
    match b["kind"]
        .as_str()
        .unwrap_or_default()
        .to_lowercase()
        .as_str()
    {
        "esplora" | "liquid-esplora" => "Explorer API".into(),
        "electrum" => "Electrum / Fulcrum".into(),
        "bitcoinrpc" => "Bitcoin Core RPC".into(),
        "btcpay" => "BTCPay".into(),
        "lnd" => "LND REST".into(),
        "coreln" => "Core Lightning".into(),
        _ if b["net"].as_str() == Some("FX") => "Rate provider".into(),
        _ => "Endpoint".into(),
    }
}

/// `abbreviateEndpointMiddle(endpoint, 44)`.
fn abbreviate_middle(endpoint: &str) -> String {
    let v: Vec<char> = endpoint.trim().chars().collect();
    if v.len() <= 44 {
        return v.into_iter().collect();
    }
    let visible = 43.0_f32;
    let (pre, suf) = (
        (visible * 0.58).ceil() as usize,
        (visible * 0.42).floor() as usize,
    );
    let head: String = v[..pre].iter().collect();
    let tail: String = v[v.len() - suf..].iter().collect();
    format!("{head}…{tail}")
}

fn panel(app: &App, tk: &Tk) -> El {
    let is_offline = offline(app);
    // `w-[min(64rem,calc(100vw-1rem))]` inside the menu surface (`p-1`, 1 px border).
    let width = tk.r(64.0).min(app.shell.window_w - tk.rem);
    let online_switch = crate::ui::switch(
        tk,
        "sh-online",
        !is_offline,
        &t("chrome:network.offlineMode.label"),
    )
    .on_press(move |s: &mut App| {
        let now_offline = offline(s);
        s.send(
            "ui.network.offline.set",
            json!({"enabled": !now_offline}),
            "shell.offline_set",
        );
    });
    // Ghost `size-7` icon button with a `size-3.5` RefreshCw.
    let check = row()
        .key("sh-check-connections")
        .size(tk.r(1.75), tk.r(1.75))
        .center()
        .radius(tk.r(0.375))
        .hover(if tk.dark {
            tk.accent.alpha(0.5)
        } else {
            tk.accent
        })
        .role("button")
        .label(t("chrome:network.checkConnections"))
        .help(t("chrome:network.checkConnections"))
        .focusable(true)
        .disabled(is_offline || app.shell.probing)
        .opacity(if is_offline { 0.5 } else { 1.0 })
        .on_press(check_connections)
        .child({
            let g = ic(&I::REFRESH_CW, tk.r(0.875), tk.muted_fg);
            if app.shell.probing {
                g.looping(rune::LoopSpec::rotation(1.0))
            } else {
                g
            }
        });
    // The glass header: the title, then a `gap-3` group of the switch (with its `text-xs
    // font-medium` caption after it) and the check button.
    let controls = row()
        .gap(tk.r(0.75))
        .center_y()
        .child(
            row().gap(tk.r(0.5)).center_y().child(online_switch).child(
                text(t("chrome:network.offlineMode.label"))
                    .font(tk.txs(500.0))
                    .color(tk.muted_fg),
            ),
        )
        .child(check);
    let header = super::popover_header(
        tk,
        &t("chrome:network.outboundConnections"),
        None,
        Some(controls),
    );
    let list = backends(app);
    let pad = super::menu_pad(tk);
    let mut wrap = col().key("sh-net-body").w_full().padding(pad);
    if app.read("ui.backends.settings.list").is_none() {
        wrap = wrap.child(
            row().w_full().px(tk.r(0.5)).py(tk.r(1.0)).child(
                text(t("chrome:network.loading"))
                    .font(tk.tsm(400.0))
                    .color(tk.muted_fg),
            ),
        );
    } else if list.is_empty() {
        wrap = wrap.child(
            col()
                .w_full()
                .px(tk.r(0.5))
                .py(tk.r(1.0))
                .gap(tk.r(0.75))
                .child(
                    text(t("chrome:network.noneConfigured"))
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg),
                )
                .child(
                    crate::ui::button::button(
                        tk,
                        "sh-add-connection",
                        crate::ui::button::Variant::Outline,
                        crate::ui::button::Size::Sm,
                        Some(&I::PLUS),
                        &t("chrome:network.addConnection"),
                        true,
                    )
                    .on_press(|s: &mut App| {
                        s.shell.popup = None;
                        s.shell.settings_open = true;
                    }),
                ),
        );
    } else {
        // `table-fixed` with columns 4rem / 30rem / 26rem, stretched to the table's width.
        let table_w = width - 2.0 - 2.0 * pad;
        let scale = table_w / tk.r(60.0);
        let cols = [tk.r(4.0) * scale, tk.r(30.0) * scale, tk.r(26.0) * scale];
        let head = |s: &str, w: f32| {
            row()
                .w(w)
                .h(tk.r(2.5))
                .px(tk.r(0.5))
                .center_y()
                .child(text(s.to_owned()).font(tk.tsm(500.0)).color(tk.fg))
        };
        wrap = wrap.child(
            row()
                .w_full()
                .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
                .border_layout()
                .child(head(&t("chrome:network.columnState"), cols[0]))
                .child(head(&t("chrome:network.columnConnection"), cols[1]))
                .child(head(&t("chrome:network.columnEndpoint"), cols[2])),
        );
        let mut rows = col()
            .w_full()
            .max_h(tk.r(8.0 * 3.25))
            .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
            .border_layout()
            .scroll(rune::ScrollSpec {
                stick_bottom: false,
            });
        let n = list.len();
        for (i, b) in list.iter().enumerate() {
            let name = b["name"].as_str().unwrap_or_default();
            let display = b["display_name"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or(name);
            let probe = app
                .shell
                .probes
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, p)| p.clone());
            // `connectionDotClassName`: unknown `bg-muted-foreground/35`, unavailable slate-400.
            let (color, state_text) = match &probe {
                _ if is_offline => (
                    rune::color::hex("#94a3b8").unwrap(),
                    t("chrome:network.health.offline"),
                ),
                None => (
                    tk.muted_fg.alpha(0.35),
                    t("chrome:network.health.notChecked"),
                ),
                Some(Probe::Waiting) | Some(Probe::Running) => (
                    rune::color::hex("#f59e0b").unwrap(),
                    t("chrome:network.health.checking"),
                ),
                Some(Probe::Ok) => (
                    rune::color::hex("#10b981").unwrap(),
                    t("chrome:network.health.healthy"),
                ),
                Some(Probe::Failed(_)) => (
                    rune::color::hex("#ef4444").unwrap(),
                    t("chrome:network.health.failed"),
                ),
            };
            let title = match &probe {
                Some(Probe::Failed(e)) => e.clone(),
                _ => state_text.clone(),
            };
            let subtitle = format!("{} · {state_text}", protocol_label(b));
            let url = b["url"].as_str().unwrap_or_default().to_owned();
            let endpoint = abbreviate_middle(&url);
            let open_settings = |s: &mut App| {
                s.shell.popup = None;
                s.shell.settings_open = true;
            };
            let mut r = row()
                .key(format!("sh-net-row{i}"))
                .w_full()
                .h(tk.r(3.25))
                .center_y()
                .hover(tk.muted.alpha(0.5))
                .child(
                    row().w(cols[0]).px(tk.r(0.5)).center_y().child(
                        dot(tk.r(0.625), color)
                            .label(state_text.clone())
                            .help(title.clone()),
                    ),
                )
                .child(
                    row().w(cols[1]).min_w(0.0).px(tk.r(0.5)).center_y().child(
                        col()
                            .key(format!("sh-net-conn{i}"))
                            .min_w(0.0)
                            .role("button")
                            .label(format!("{display} {subtitle}"))
                            .focusable(true)
                            .on_press(open_settings)
                            .child(
                                text(display.to_owned())
                                    .font(tk.tsm(500.0))
                                    .color(tk.fg)
                                    .truncate(),
                            )
                            .child(
                                text(subtitle.clone())
                                    .font(tk.txs(400.0))
                                    .color(tk.muted_fg)
                                    .truncate()
                                    .help(title),
                            ),
                    ),
                )
                .child(
                    row().w(cols[2]).min_w(0.0).px(tk.r(0.5)).center_y().child(
                        row()
                            .key(format!("sh-net-endpoint{i}"))
                            .min_w(0.0)
                            .role("button")
                            .label(endpoint.clone())
                            .help(url.clone())
                            .focusable(true)
                            .on_press(open_settings)
                            .child(
                                text(endpoint)
                                    .font(tk.txs(400.0).mono())
                                    .color(tk.muted_fg)
                                    .truncate(),
                            ),
                    ),
                );
            if i + 1 < n {
                r = r
                    .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
                    .border_layout();
            }
            rows = rows.child(r);
        }
        wrap = wrap.child(rows);
    }
    let mut out = col()
        .w(width)
        .role("dialog")
        .label(t("chrome:network.outboundConnections"))
        .child(header)
        .child(wrap);
    if is_offline {
        out = out.child(
            row()
                .w_full()
                .px(tk.r(1.0))
                .py(tk.r(0.75))
                .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
                .border_layout()
                .child(
                    text(t("chrome:network.offlineMode.on"))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .wrap(),
                ),
        );
    }
    out
}
