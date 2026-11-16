//! Kassiber's desktop shell and Overview screen rebuilt with Rune, to compare against the
//! Tauri + React desktop. It drives the same `kassiber daemon` over the same JSONL protocol.
//!
//! `cargo run --release -- --data-root <book data root>` opens the window;
//! `-- --snap out.png --size 1440x900` renders headless (see README.md).

// Shipped Windows builds are GUI-subsystem executables: no console window opens beside the app.
// Tooling builds keep the console so headless runs can print.
#![cfg_attr(all(windows, not(feature = "tooling")), windows_subsystem = "windows")]

mod app;
mod assistant;
mod chart;
#[cfg(test)]
mod checks;
mod custody;
mod daemon;
mod demo;
mod fixtures;
mod format;
mod i18n;
mod icons;
mod kinds;
mod logos;
mod model;
mod native;
mod nav;
mod onboarding;
mod osleaks;
mod overview;
mod persist;
mod provenance;
mod reports;
mod review;
mod secret_floor;
mod secrets;
mod settings;
mod shell;
mod theme;
mod toastopen;
mod transactions;
mod txdetail;
mod ui;
mod wallets;

use rune::{AppSpec, Cx, El, KeyEvent, Tx};

use app::App;
use shell::crash;

static FONTS: &[&[u8]] = &[
    // DM Sans: only `<kbd>` hints draw in it (`font-sans` in ui/kbd.tsx), see `SANS`.
    include_bytes!("../assets/fonts/dm-sans-wght.ttf"),
    include_bytes!("../assets/fonts/jbmono-400.ttf"),
    include_bytes!("../assets/fonts/jbmono-500.ttf"),
    include_bytes!("../assets/fonts/jbmono-600.ttf"),
];

/// The sans family the shipped web app actually draws in. `ui-tauri/index.html` sets an
/// unlayered `body { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto,
/// "Helvetica Neue", Arial, sans-serif }` for its boot screen, and that beats globals.css's
/// `@layer base` DM Sans rule, so every screen renders in the platform's UI font. These lists are
/// that stack as each webview resolves it: WKWebView's `-apple-system` is the system UI font,
/// WebView2 finds Segoe UI, and Chromium/WebKitGTK on Linux skip the missing names and take
/// Arial's metric-compatible alias (Liberation Sans or Arimo) through fontconfig. Rune loads
/// installed families by exact name and applies no fontconfig aliases, hence the explicit names.
/// Glyphs the face lacks (₿) fall back through the platform, as the browser's do.
/// On macOS Rune names the system UI font `System Font` (SFNS with its `opsz` axis and `trak`
/// tracking, as Core Text and WKWebView draw `-apple-system`); `.AppleSystemUIFont` is not a
/// family Rune resolves, and falling through it lands on a face about 5 % narrower.
#[cfg(target_os = "macos")]
const SANS: &str = "System Font, Helvetica Neue, Arial, DM Sans 9pt";
#[cfg(target_os = "windows")]
const SANS: &str = "Segoe UI, Arial, DM Sans 9pt";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const SANS: &str =
    "Segoe UI, Roboto, Helvetica Neue, Arial, Liberation Sans, Arimo, DejaVu Sans, DM Sans 9pt";
/// Chromium on Linux draws a character the page's fonts lack in the first font fontconfig's
/// sorted match (`FcFontSort`) has it in (`gfx::GetFallbackFontForChar`). Rune's own Linux
/// symbol fallback only tries DejaVu/Noto symbol families, so the symbols the UI draws (`⌘⇧`
/// in the privacy menu, `₿`) are resolved here the way Chromium resolves them, once at launch,
/// and their families appended to the sans stack.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn sans_stack() -> &'static str {
    const UI_SYMBOLS: [char; 3] = ['⌘', '⇧', '₿'];
    let mut stack = SANS.to_owned();
    for ch in UI_SYMBOLS {
        if let Some(family) = chromium_fallback(ch) {
            if !stack.split(',').any(|f| f.trim() == family) {
                stack.push_str(", ");
                stack.push_str(&family);
            }
        }
    }
    Box::leak(stack.into_boxed_str())
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn sans_stack() -> &'static str {
    SANS
}

/// The family Chromium would draw `ch` in: the first font in fontconfig's sorted match for the
/// stack's Linux face whose charset contains it (None when it's the face itself, or no fc-match).
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn chromium_fallback(ch: char) -> Option<String> {
    let cp = ch as u32;
    let out = std::process::Command::new("fc-match")
        .args([
            "-s",
            &format!("Liberation Sans:charset={cp:x}"),
            "-f",
            "%{family[0]}|%{charset}\n",
        ])
        .output()
        .ok()?;
    let covers = |charset: &str| {
        charset.split_whitespace().any(|r| {
            let (a, b) = r.split_once('-').unwrap_or((r, r));
            matches!((u32::from_str_radix(a, 16), u32::from_str_radix(b, 16)), (Ok(a), Ok(b)) if (a..=b).contains(&cp))
        })
    };
    let first = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.split_once('|'))
        .find(|(_, cs)| covers(cs))
        .map(|(f, _)| f.to_owned())?;
    (first != "Liberation Sans").then_some(first)
}

/// `--font-mono` (globals.css): JetBrains Mono, bundled at 400/500/600 as the web bundles it.
const MONO: &str = "JetBrains Mono, ui-monospace, SFMono-Regular, Menlo, monospace";

/// The frame, under crash recovery: a panic while drawing shows the recovery screen in the
/// shell (or alone, when the shell is what fails) instead of ending the app.
fn view(app: &App, cx: &mut Cx<App>) -> El {
    i18n::set_lang(app.prefs.lang);
    let tk = app.tk();
    if crash::current().is_none()
        && let Some(el) = crash::guard("view", app.route.path(), || view_app(app, cx))
    {
        return el;
    }
    let rec = crash::current().unwrap_or_default();
    if !rec.bare {
        let panel_h = (app.shell.window_h - 40.0 - tk.r(0.375) - 2.0).max(tk.r(30.0));
        let shelled = crash::guard("view", app.route.path(), || {
            crash_trigger();
            let mut layer = Vec::new();
            let body = shell::view(app, cx, &tk, crash::page(&tk, &rec, panel_h), &mut layer);
            rune_kit::root(body, layer)
        });
        match shelled {
            Some(el) => return el,
            None => crash::mark_bare(),
        }
    }
    rune_kit::root(
        crash::page(&tk, &rec, app.shell.window_h.max(tk.r(30.0))),
        Vec::new(),
    )
}

/// Scenes `crash` and `crash-bare` make the next views panic, to show the recovery screen
/// through the real catch (tooling builds only).
#[cfg(feature = "tooling")]
static CRASH_VIEWS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn crash_trigger() {
    #[cfg(feature = "tooling")]
    if CRASH_VIEWS
        .try_update(
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst,
            |n| n.checked_sub(1),
        )
        .is_ok()
    {
        panic!("Synthetic view panic (crash scene)");
    }
}

fn view_app(app: &App, cx: &mut Cx<App>) -> El {
    crash_trigger();
    let tk = app.tk();
    // Overlays (popovers, menus, dialogs, sheets, tooltips) mount above everything.
    let mut layer: Vec<El> = Vec::new();
    // Welcome (`/` without an identity) draws its own frame, without the shell.
    if onboarding::shows(app) {
        let body = onboarding::root(app, cx, &mut layer);
        return rune_kit::root(body, layer)
            .on_window_focus(|a: &mut App, f: bool, _| settings::effects::window_focus(a, f));
    }
    let page = match app.route.path() {
        "/overview" | "/" => overview::view(app, cx, &tk, &mut layer),
        "/transactions" => transactions::view(app, cx, &tk, &mut layer),
        // Dev harnesses, in tooling builds only: the graph section full-page
        // (`/dev/graph?tx=<id>`) and the text-metric page (`/dev/typo`).
        #[cfg(feature = "tooling")]
        "/dev/graph" => txdetail::graph::dev_page(app, cx, &tk, &mut layer),
        #[cfg(feature = "tooling")]
        "/dev/typo" => ui::typo_page(&tk),
        p if wallets::owns(p) => wallets::view(app, cx, &tk, &mut layer),
        p if review::owns(p) => review::view(app, cx, &tk, &mut layer),
        p if reports::owns(p) => reports::view(app, cx, &tk, &mut layer),
        p if assistant::owns(p) => assistant::view(app, cx, &tk, &mut layer),
        p if custody::owns(p) => custody::view(app, cx, &tk, &mut layer),
        p if provenance::owns(p) => provenance::view(app, cx, &tk, &mut layer),
        _ => ui::not_found_page(app, &tk),
    };
    let body = shell::view(app, cx, &tk, page, &mut layer);
    native::overlay(app, &tk, &mut layer);
    rune_kit::root(body, layer)
        .on_window_focus(|a: &mut App, f: bool, _| settings::effects::window_focus(a, f))
}

fn setup(app: &mut App, waker: rune::Waker) {
    osleaks::set_waker(waker.clone());
    app.set_waker(waker);
    // Headless capture overrides, applied after the `--scene` so any scene can be shot in either
    // theme or at another UI scale: `--app-theme light|dark`, `--app-scale 0.9`. (Rune itself
    // consumes `--theme`, so the app's own `--theme` never reaches `extra_args`.)
    let args = rune::extra_args();
    match app::flag(args, "--app-theme").as_deref() {
        Some("light") => app.prefs.theme = app::ThemeMode::Light,
        Some("dark") => app.prefs.theme = app::ThemeMode::Dark,
        _ => {}
    }
    if let Some(s) = app::flag(args, "--app-scale").and_then(|v| v.parse::<f32>().ok()) {
        app.prefs.scale = s.clamp(0.8, 1.2);
    }
    native::setup(app);
}

/// Every frame, under crash recovery: a panic in a poll shows the recovery screen; the next
/// polls still run, so the daemon keeps being served.
fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
    let route = app.route.path().to_owned();
    let changed = crash::guard("poll", &route, || poll_app(app, clock, tx)).unwrap_or(true);
    crash::log_once(app);
    changed
}

fn poll_app(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
    app.clock = clock;
    let held = toastopen::poll(app, tx);
    let before = app.toasts.len();
    app.toasts.retain(|t| t.until > clock);
    let mut changed = app.pump() || app.toasts.len() != before || held;
    if let Some(next) = app.toasts.iter().map(|t| t.until).reduce(f64::min) {
        tx.wake_after(std::time::Duration::from_secs_f64((next - clock).max(0.05)));
    }
    changed |= shell::on_leave(app);
    changed |= transactions::poll(app, tx);
    changed |= overview::poll(app, tx);
    changed |= settings::poll(app, tx);
    changed |= wallets::poll(app, tx);
    changed |= review::poll(app, tx);
    changed |= reports::poll(app, tx);
    changed |= assistant::poll(app, tx);
    changed |= txdetail::graph::poll(app, tx);
    changed |= custody::poll(app, tx);
    changed |= provenance::poll(app, tx);
    changed |= onboarding::poll(app, tx);
    changed |= demo::poll(app, tx);
    changed |= persist::poll(app, tx);
    changed |= native::poll(app, tx);
    changed |= secrets::poll(app, tx);
    changed |= shell::wordmark::poll(app, tx);
    changed |= shell::skip_link_poll(app, tx);
    changed |= shell::search::poll(app, tx);
    changed |= nav::poll(app, tx);
    font_smoothing_flag(tx);
    changed
}

/// Parity harness: `--font-smoothing <mode>` picks a glyph rasterisation mode from the first
/// poll on (it applies from the next frame, so snapshot with `--at 0.2` or later).
fn font_smoothing_flag(tx: &mut Tx) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static DONE: AtomicBool = AtomicBool::new(false);
    if DONE.swap(true, Ordering::Relaxed) {
        return;
    }
    use rune::FontSmoothing as F;
    let mode = match app::flag(rune::extra_args(), "--font-smoothing").as_deref() {
        Some("on") => F::On,
        Some("off") => F::Off,
        Some("windfoil") => F::Windfoil,
        Some("windfoil-chromium") => F::WindfoilChromium,
        Some("slug") => F::Slug,
        Some("hinted-gpu") => F::HintedGpu,
        Some("hinted") => F::Hinted,
        _ => return,
    };
    tx.set_font_smoothing(mode);
}

/// Keys, under crash recovery. While the recovery screen shows, only its own buttons act.
fn on_key(app: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if crash::current().is_some() {
        return false;
    }
    let route = app.route.path().to_owned();
    crash::guard("key", &route, || on_key_app(app, ev, tx)).unwrap_or(true)
}

fn on_key_app(app: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    // Innermost first: an open chart dialog or detail sheet takes Escape before the shell.
    onboarding::on_key(app, ev, tx)
        || settings::on_key(app, ev, tx)
        || assistant::on_key(app, ev, tx)
        || chart::on_key(app, ev, tx)
        || txdetail::graph::on_key(app, ev, tx)
        || (assistant::detail_in_view(app) && txdetail::on_key(app, ev, tx))
        || transactions::on_key(app, ev, tx)
        || overview::on_key(app, ev, tx)
        || wallets::on_key(app, ev, tx)
        || review::on_key(app, ev, tx)
        || reports::on_key(app, ev, tx)
        || custody::on_key(app, ev, tx)
        || provenance::on_key(app, ev, tx)
        || shell::on_key(app, ev, tx)
        || native::on_key(app, ev, tx)
}

/// Rune's interaction timing posts spans to a loopback OpenTelemetry collector when `RUNE_PERF=1`
/// or when `OTEL_EXPORTER_OTLP_ENDPOINT` names a loopback address, which developer machines often
/// set globally. That is an unconsented loopback probe (docs/reference/privacy-and-security.md),
/// so it stays off unless a tooling build is started with an explicit `RUNE_PERF=1`.
fn keep_interaction_timing_off() {
    let explicit =
        cfg!(feature = "tooling") && std::env::var("RUNE_PERF").is_ok_and(|v| v.trim() == "1");
    if !explicit {
        // SAFETY: the first statement of `main`, before any other thread exists.
        unsafe { std::env::set_var("RUNE_PERF", "0") };
    }
}

fn main() {
    keep_interaction_timing_off();
    // The operator broker's Touch ID helper (`--operator-native-auth`) runs and exits before
    // anything else starts, no window or daemon among it (lib.rs `run`).
    if let Some(code) = secrets::operator::run_if_requested() {
        std::process::exit(code);
    }
    // The helper flags and CLI forwarding exit before anything starts; a relaunch hands its
    // links to the running app and exits.
    if let Some(code) = native::early_exit() {
        std::process::exit(code);
    }
    if !native::claim_instance() {
        return;
    }
    crash::install();
    // The web sets `-webkit-font-smoothing: antialiased`; Rune's `Off` is that rendering (and
    // measured closest to Chromium on Linux too, parity/font_smoothing.py).
    let app = rune::App::new(AppSpec::<App> {
        fonts: FONTS,
        families: (sans_stack(), MONO),
        // `tauri.conf.json`'s minWidth / minHeight.
        min_size: Some(persist::window::MIN_SIZE),
        setup: Some(setup),
        poll: Some(poll),
        on_key: Some(on_key),
        // Shipped builds (no `tooling` feature) carry no scenes: `--scene` is development state.
        scenes: if cfg!(feature = "tooling") {
            all_scenes()
        } else {
            &[]
        },
        ..AppSpec::new("Kassiber", persist::window::FALLBACK_SIZE, App::new, view)
    })
    .font_smoothing(rune::FontSmoothing::Off)
    // The shell draws its own 40 pt title bar (WINDOW_TITLEBAR_HEIGHT_PX). Declaring it gives the
    // window drag-regions-only chrome: only `.window_drag()` areas move the window and take the
    // double-click zoom, so a double press on a bar button stays that button's (with no titlebar
    // declared, AppKit's native band zoomed the window over the buttons too). The traffic lights
    // sit where Tauri's `trafficLightPosition` puts them: x 14, centred on the bar's 20 pt line
    // (Rune centres the 14 pt button in 14 + 2y, so y = 13).
    .titlebar(rune::Titlebar::new(40.0).traffic_lights(14.0, 13.0))
    // The transaction graph's 3D glass view renders through Rune's compositor hook. It needs a
    // depth buffer, which Rune's native Metal backend (its macOS default) doesn't support, so
    // draw with wgpu everywhere (wgpu still uses Metal on macOS). RUNE_GPU_BACKEND overrides.
    .gpu_backend(rune::GpuBackend::Wgpu)
    .compositor(txdetail::graph::gpu::compositor())
    // Preferences and the window frame survive the launch (`persist`): the frame is sampled
    // through the window's handle and written as the window closes or the app quits.
    .on_window(persist::window::remember_handle)
    // One close handler (a second `.on_close` replaces the first): "Lock when the window
    // closes" (AppShell's `pagehide` lock), then the pending preferences are written.
    .on_close(|a: &mut App, tx: &mut Tx| {
        app::on_window_close(a, tx);
        persist::on_close(a, tx);
    })
    .on_quit(persist::on_quit);
    let app = native::install(app);
    // Opens the window at its restored frame (tooling runs exactly as `App::run`).
    persist::window::run(app);
}

/// The assistant's visuals scene, one figure opened larger and a point of `chart` hovered.
fn scene_visual_hover(a: &mut App, call: &str, chart: &str, index: usize) {
    if let Some((_, f)) = assistant::SCENES
        .iter()
        .find(|(n, _)| *n == "assistant-thread-visuals")
    {
        f(a);
    }
    a.assistant.thread.expanded_visual = Some(format!("asst-visual-{call}"));
    a.assistant.thread.chart_hover = Some((chart.to_owned(), index));
}

/// A synthetic book refresh at 46 %: the backend fetch of the second of three sources.
fn scene_sync_running(a: &mut App) {
    a.busy.syncing = true;
    shell::maint::start_refresh(a, false);
    let p = serde_json::json!({"data": {"phase": "backend_fetch", "job_id": "j1", "job_index": 2, "job_total": 3, "source_label": "Regtest Cold Storage", "source_type": "onchain_wallet", "processed": 120, "total": 200}});
    shell::maint::on_progress(a, "app.freshness_run", &p);
}

/// A daemon answer for scenes that show an outcome without asking the daemon.
fn scene_reply(data: serde_json::Value, error: Option<&str>) -> app::Reply {
    app::Reply {
        kind: "ui.freshness.run".into(),
        data,
        error: error.map(str::to_owned),
        error_detail: serde_json::Value::Null,
        elapsed: std::time::Duration::ZERO,
    }
}

/// Shipped builds route neither dev page (`view` gates them on `tooling`). The graph harness
/// keeps its scene loader for the scene table, so its page is referenced here, never called,
/// and the linker strips it.
#[cfg(not(feature = "tooling"))]
const _: fn(&App, &mut Cx<App>, &theme::Tk, &mut Vec<El>) -> El = txdetail::graph::dev_page;

/// The shared scenes below plus each area's own (`<area>::SCENES`).
fn all_scenes() -> &'static [crate::app::Scene] {
    let mut all: Vec<crate::app::Scene> = SCENES.to_vec();
    all.extend_from_slice(app::SCENES);
    for area in [
        settings::SCENES,
        wallets::SCENES,
        review::SCENES,
        reports::SCENES,
        assistant::SCENES,
        custody::SCENES,
        provenance::SCENES,
        provenance::SOF_SCENES,
        overview::SCENES,
        txdetail::SCENES,
        native::SCENES,
        secrets::SCENES,
        onboarding::SCENES,
        demo::SCENES,
        chart::SCENES,
        nav::SCENES,
        transactions::EXPORT_SCENES,
        toastopen::SCENES,
        osleaks::SCENES,
    ] {
        all.extend_from_slice(area);
    }
    Box::leak(all.into_boxed_slice())
}

/// Headless: the owning area loads what the current route needs before the first frame.
fn prime_route(a: &mut App) {
    let path = a.route.path().to_owned();
    if onboarding::shows(a) {
        onboarding::prime(a);
    } else if wallets::owns(&path) {
        wallets::prime(a);
    } else if review::owns(&path) {
        review::prime(a);
    } else if reports::owns(&path) {
        reports::prime(a);
    } else if assistant::owns(&path) {
        assistant::prime(a);
    } else if custody::owns(&path) {
        custody::prime(a);
    } else if provenance::owns(&path) {
        provenance::prime(a);
    }
    if a.shell.settings_open {
        settings::prime(a);
    }
}

/// Named states for headless snapshots (`--scene`) and the playground.
static SCENES: &[crate::app::Scene] = &[
    // Parity harness: `--scene route --route <path>` navigates (Transactions reads its query);
    // `--scene open-tx --tx <id> [--tab <tab>] [--route <path>]` opens the detail panel.
    ("route", |a| {
        let args = rune::extra_args();
        if let Some(r) = app::flag(args, "--route").map(|r| fixtures::expand(&r)) {
            a.navigate(&r);
            if r.starts_with("/transactions") {
                transactions::prime(a, r.contains("period=all"));
            }
            prime_route(a);
        }
    }),
    ("open-tx", |a| {
        let args = rune::extra_args();
        if let Some(r) = app::flag(args, "--route").map(|r| fixtures::expand(&r)) {
            a.navigate(&r);
            if r.starts_with("/transactions") {
                transactions::prime(a, false);
            }
        }
        // `--tx {fx:name}` names a demo-book fixture (parity/fixtures.json).
        if let Some(id) = app::flag(args, "--tx").map(|id| fixtures::expand(&id)) {
            let tab = app::flag(args, "--tab").unwrap_or_else(|| "details".into());
            txdetail::open(a, &id, &tab);
            txdetail::settle(a);
        }
    }),
    ("light", |a| a.prefs.theme = app::ThemeMode::Light),
    ("de", |a| a.prefs.lang = i18n::Lang::De),
    ("transactions", |a| {
        a.navigate("/transactions");
        transactions::prime(a, false);
    }),
    ("transactions-all", |a| {
        a.navigate("/transactions?period=all");
        transactions::prime(a, true);
    }),
    ("transactions-de", |a| {
        a.prefs.lang = i18n::Lang::De;
        a.navigate("/transactions");
        transactions::prime(a, false);
    }),
    ("transactions-light", |a| {
        a.prefs.theme = app::ThemeMode::Light;
        a.navigate("/transactions");
        transactions::prime(a, false);
    }),
    ("transactions-filters", |a| {
        a.navigate("/transactions");
        transactions::prime(a, false);
        a.txs.popup = Some(transactions::Popup::FiltersSub("flow"));
    }),
    ("transactions-rowmenu", |a| {
        a.navigate("/transactions");
        transactions::prime(a, false);
        if let Some(r) = a.txs.filtered.get(1).map(|&i| a.txs.rows[i].id.clone()) {
            a.txs.popup = Some(transactions::Popup::RowMenu(r));
        }
    }),
    ("transactions-chips", |a| {
        a.navigate("/transactions?period=all&status=pending&flow=incoming&fees=with-fees&sort=amount&order=desc");
        transactions::prime(a, false);
    }),
    ("transactions-expanded", |a| {
        a.navigate("/transactions");
        transactions::prime(a, false);
        a.txs.expanded = true;
    }),
    ("transactions-export", |a| {
        a.navigate("/transactions");
        transactions::prime(a, false);
        transactions::open_export_primed(a);
    }),
    ("transactions-year", |a| {
        a.navigate("/transactions");
        transactions::prime(a, false);
        a.txs.popup = Some(transactions::Popup::Year);
    }),
    ("dark", |a| a.prefs.theme = app::ThemeMode::Dark),
    ("typo", |a| a.navigate("/dev/typo")),
    ("hide-amounts", |a| {
        a.prefs.privacy = app::Privacy::HideAmounts
    }),
    ("fiat", |a| a.prefs.currency = app::Currency::Fiat),
    // Shell scenes.
    ("collapsed", |a| a.prefs.nav_collapsed = true),
    ("search", |a| a.shell.toggle(shell::Popup::Search)),
    ("search-typed", |a| {
        a.shell.toggle(shell::Popup::Search);
        a.shell.search = rune::TextEdit::new("cold");
        a.shell.search.focused = true;
    }),
    ("books", |a| a.shell.toggle(shell::Popup::Books)),
    ("user-menu", |a| a.shell.toggle(shell::Popup::UserMenu)),
    ("refresh-menu", |a| {
        a.shell.toggle(shell::Popup::RefreshMenu)
    }),
    // The notification centre as the web shows it on this book: nothing recorded yet.
    ("notifications", |a| {
        a.shell.toggle(shell::Popup::Notifications)
    }),
    // With two recent entries (the web's RAM-only store, seeded).
    ("notifications-recent", |a| {
        a.shell.note(
            &i18n::t("chrome:notifications.result.bookRefreshed.title"),
            None,
            app::ToastTone::Success,
            0.0,
        );
        a.shell.note(
            &i18n::t("chrome:notifications.result.journalsProcessed.title"),
            Some(i18n::tc(
                "chrome:notifications.result.journalsProcessed.transactions",
                42,
                &[],
            )),
            app::ToastTone::Success,
            0.0,
        );
        a.shell.toggle(shell::Popup::Notifications);
    }),
    ("privacy-menu", |a| a.shell.toggle(shell::Popup::Privacy)),
    ("theme-menu", |a| a.shell.toggle(shell::Popup::Theme)),
    ("network", |a| a.shell.toggle(shell::Popup::Network)),
    ("alpha", |a| a.shell.toggle(shell::Popup::Alpha)),
    ("locked", |a| a.locked = true),
    // An unlock attempt while the daemon has not answered yet: the wordmark decrypts again.
    ("locked-attempt", |a| {
        a.locked = true;
        a.lock.attempt = 1;
        a.lock.submitting = true;
    }),
    ("toasts", |a| {
        a.toast(
            app::ToastTone::Success,
            "BTC price refreshed",
            None,
            Some("market-rate-refresh"),
        );
        a.toast(
            app::ToastTone::Error,
            "Could not refresh the BTC price",
            Some("The rate provider did not answer.".into()),
            Some("rate-error"),
        );
    }),
    // The sync card mid-refresh: a source's fetch reported 46 % (SYNTHETIC progress; nothing
    // is sent, so no refresh runs).
    ("sync-card", scene_sync_running),
    // Its first moment, before any progress arrived.
    ("sync-card-start", |a| {
        a.busy.syncing = true;
        shell::maint::start_refresh(a, false);
    }),
    // A refresh the daemon refused: the failed card, toast and condition.
    ("sync-card-failed", |a| {
        scene_sync_running(a);
        a.busy.syncing = false;
        shell::maint::on_refresh_reply(
            a,
            &scene_reply(
                serde_json::Value::Null,
                Some("The wallet backend did not answer in time."),
            ),
        );
    }),
    // A refresh that returned with a failed source: "Book refresh needs attention".
    ("sync-card-attention", |a| {
        scene_sync_running(a);
        a.busy.syncing = false;
        let data = serde_json::json!({"completed": [{"id": "j1", "job_type": "wallet_sync", "status": "error", "source_label": "Regtest Cold Storage", "error": {"message": "Connection refused"}}, {"id": "j2", "job_type": "journal_refresh", "status": "done", "result": {"quarantined": 2}}]});
        shell::maint::on_refresh_reply(a, &scene_reply(data, None));
    }),
    // The running job in the notification centre, its bar at 46 %.
    ("sync-notifications", |a| {
        scene_sync_running(a);
        a.shell.sync_card_open = false;
        a.shell.toggle(shell::Popup::Notifications);
    }),
    // Journal processing at its "Building journal entries" phase: the progress line, and the
    // running job in the notification centre (SYNTHETIC progress).
    ("journals-processing", |a| {
        a.busy.processing = true;
        shell::maint::start_journals(a, true);
        shell::maint::on_progress(
            a,
            "app.journals_process",
            &serde_json::json!({"data": {"phase": "building"}}),
        );
        a.shell.toggle(shell::Popup::Notifications);
    }),
    // A journal run that failed: its condition with Retry, and the Recent entry.
    ("journals-failed", |a| {
        a.busy.processing = true;
        shell::maint::start_journals(a, true);
        a.busy.processing = false;
        shell::maint::on_journals_reply(
            a,
            &scene_reply(
                serde_json::Value::Null,
                Some("The database is locked by another process."),
            ),
        );
        a.shell.toggle(shell::Popup::Notifications);
    }),
    // Crash recovery: the next view panics (a real catch), and the recovery card shows in the
    // shell; `crash-bare` makes the shell fail too, so the card shows alone.
    #[cfg(feature = "tooling")]
    ("crash", |_| {
        CRASH_VIEWS.store(1, std::sync::atomic::Ordering::SeqCst)
    }),
    #[cfg(feature = "tooling")]
    ("crash-bare", |_| {
        CRASH_VIEWS.store(2, std::sync::atomic::Ordering::SeqCst)
    }),
    // An imported book that cannot be opened: the restore screen with the real error from
    // inspecting a folder this scene makes unreadable (a temp folder; nothing else is touched).
    ("import-root-error", |a| {
        let dir = std::env::temp_dir().join("kassiber-gapfix-unreadable");
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("kassiber.sqlite3"), b"SQLite format 3\0");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(
                dir.join("kassiber.sqlite3"),
                std::fs::Permissions::from_mode(0o000),
            );
        }
        let err = onboarding::projects::inspect(&dir)
            .err()
            .unwrap_or_default();
        onboarding::import_root_failed(a, err);
    }),
    // Assistant chart tooltips: the overview figure opened larger with its 9th point hovered,
    // and the cash-flow figure's October bar (SYNTHETIC visuals, as the assistant scenes).
    ("assistant-visual-tooltip", |a| {
        scene_visual_hover(a, "v1", "asst-visual-v1-x-chart-x", 8)
    }),
    ("assistant-visual-tooltip-bars", |a| {
        scene_visual_hover(a, "v5", "asst-visual-v5-flow-x", 3)
    }),
    // A route nothing renders: the not-found page inside the shell.
    ("not-found", |a| a.navigate("/no-such-page")),
    // The title bar's book status only shows when it isn't OK: a failed refresh.
    ("freshness-failed", |a| a.shell.sync_failed = true),
    ("settings", |a| a.shell.settings_open = true),
    ("dev-tools", |a| {
        a.prefs.developer_tools = true;
        a.shell.support_open = true;
        a.shell.extras_open = true;
    }),
    // Overview scenes.
    ("tx-detail", |a| {
        txdetail::scene_open(a, "details", |_| true)
    }),
    ("add-connection", overview::open_add_connection),
    // Chart scenes.
    ("chart-30d", |a| a.chart.set_period(chart::Period::D30)),
    ("chart-1y", |a| a.chart.set_period(chart::Period::Y1)),
    ("chart-all-log", |a| {
        // Power-law view: log time (switches to All) and log price.
        a.chart.log_y = true;
        a.chart.enable_log_time(true);
    }),
    ("chart-fit", |a| a.chart.fit = true),
    // The parity recipe's pointer: 45 % / 55 % across the plot surface (613, 477 in the window).
    ("chart-hover", |a| {
        chart::scene_hover_at(a, 613.0 - 261.2, 477.0 - 293.9)
    }),
    ("chart-gear", |a| {
        a.chart.popup = Some(chart::ChartPopup::Gear(chart::View::Compact))
    }),
    ("chart-gear-incoming", |a| {
        a.chart.popup = Some(chart::ChartPopup::GearIncoming(chart::View::Compact))
    }),
    ("chart-year", |a| {
        a.chart.popup = Some(chart::ChartPopup::Year(chart::View::Compact))
    }),
    ("chart-expanded", |a| a.chart.expanded = true),
    ("chart-ungrouped", |a| a.chart.group = false),
    ("chart-basis-gap", |a| {
        // SYNTHETIC: the demo book's cost basis is complete, so this scene marks it incomplete
        // from 2025-03-01 (3 quarantined, 2 unpriced rows) to show the region and popover.
        if let app::Load::Ready(s) = &mut a.overview {
            let c = &mut s.fiat.completeness;
            c.state = "incomplete".into();
            c.cost_basis_complete = false;
            c.reasons = vec!["quarantines".into(), "missing_prices".into()];
            c.quarantine_count = 3.0;
            c.missing_price_count = 2.0;
            c.basis_uncovered_msat = Some(150_000_000_000.0);
            c.earliest_incomplete_at = Some("2025-03-01T00:00:00Z".into());
        }
        a.chart.popup = Some(chart::ChartPopup::BasisGap(chart::View::Compact, 1));
    }),
    ("detail-details", |a| {
        txdetail::scene_open(a, "details", |_| true)
    }),
    ("detail-classify", |a| {
        txdetail::scene_open(a, "classify", |_| true)
    }),
    ("detail-pricing", |a| {
        txdetail::scene_open(a, "pricing", |_| true)
    }),
    ("detail-tax", |a| txdetail::scene_open(a, "tax", |_| true)),
    ("detail-tax-send", |a| {
        txdetail::scene_open(a, "tax", |t| t.amount_sat < 0.0 && !t.internal)
    }),
    ("detail-linked", |a| {
        txdetail::scene_open(a, "linked", |t| t.pair.is_some())
    }),
    // Opened by id: the loan leg isn't among the Overview's recent or activity rows.
    ("detail-loan", |a| {
        txdetail::open(a, fixtures::id("loan"), "linked");
        txdetail::settle(a);
    }),
    ("detail-history", |a| {
        txdetail::scene_open(a, "details", |_| true);
        if let Some(o) = a.detail.open.as_mut() {
            o.history_open = true;
        }
    }),
    ("detail-explorer", |a| {
        txdetail::scene_open(a, "details", |_| true);
        if let Some(o) = a.detail.open.as_mut() {
            o.explorer_open = true;
        }
    }),
    ("detail-links", |a| {
        txdetail::scene_open(a, "details", |_| true);
        if let Some(o) = a.detail.open.as_mut() {
            o.links_open = true;
        }
        // As the web opens it: empty, the field focused (`autoFocus`).
        let mut e = rune::TextEdit::new("");
        e.focused = true;
        a.detail.links = e;
    }),
    ("detail-reuse", |a| {
        txdetail::scene_open(a, "details", |_| true);
        let c = txdetail::dialogs::reuse_candidates(a);
        txdetail::actions::open_reuse(a, &c);
        txdetail::settle(a);
        for _ in 0..100 {
            a.pump();
            if a.detail
                .open
                .as_ref()
                .and_then(|o| o.reuse.as_ref())
                .is_some_and(|r| r.list.is_some())
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }),
    ("detail-dirty", |a| {
        txdetail::scene_open(a, "classify", |_| true);
        if let Some(o) = a.detail.open.as_mut() {
            o.draft.label = "Income".into();
            o.draft.excluded = true;
        }
    }),
    ("detail-acq-preview", |a| {
        txdetail::scene_open(a, "tax", |_| true);
        if let Some(o) = a.detail.open.as_mut() {
            o.acq_kind = Some("income".into());
        }
        txdetail::actions::acq_preview(a);
        txdetail::pump_until(a, |a| a.detail.open.as_ref().is_some_and(|o| !o.busy.acq));
    }),
    ("detail-acq-apply", |a| {
        txdetail::scene_open(a, "tax", |_| true);
        if let Some(o) = a.detail.open.as_mut() {
            o.acq_kind = Some("income".into());
        }
        txdetail::actions::acq_preview(a);
        txdetail::pump_until(a, |a| a.detail.open.as_ref().is_some_and(|o| !o.busy.acq));
        txdetail::actions::acq_apply(a);
        txdetail::pump_until(a, |a| a.detail.open.as_ref().is_some_and(|o| !o.busy.acq));
    }),
    ("detail-second", |a| {
        let id = a
            .snapshot()
            .and_then(|s| s.txs.get(1))
            .map(|t| t.id.clone());
        if let Some(id) = id {
            txdetail::open(a, &id, "details");
            txdetail::settle(a);
        }
    }),
    ("detail-maximized", |a| {
        txdetail::scene_open(a, "details", |_| true);
        a.detail.maximized = true;
    }),
    ("detail-tabs", |a| {
        // A kept tab plus a new preview: pin the first row, then open the second.
        txdetail::scene_open(a, "details", |_| true);
        txdetail::pin(a);
        let id = a
            .snapshot()
            .and_then(|s| s.txs.get(1))
            .map(|t| t.id.clone());
        if let Some(id) = id {
            txdetail::open(a, &id, "details");
            txdetail::settle(a);
        }
        a.detail.tab_hover = Some(0);
    }),
    ("detail-add-menu", |a| {
        txdetail::scene_open(a, "details", |_| true);
        a.detail.add_open = true;
    }),
    ("graph-dev", |a| {
        txdetail::graph::scene_open(a, fixtures::id("partial"))
    }),
    ("graph-full", |a| {
        txdetail::graph::scene_open(a, fixtures::id("full"))
    }),
    ("graph-multi", |a| {
        txdetail::graph::scene_open(a, fixtures::id("multi"))
    }),
    ("graph-fanout", |a| {
        txdetail::graph::scene_open(a, fixtures::id("fanout"))
    }),
    ("graph-swap", |a| {
        txdetail::graph::scene_open(a, fixtures::id("swap"))
    }),
    ("graph-liquid", |a| {
        txdetail::graph::scene_open(a, fixtures::id("liquid"))
    }),
    ("graph-graphless", |a| {
        txdetail::graph::scene_open(a, fixtures::id("graphless"))
    }),
    ("graph-hidden", |a| {
        a.prefs.privacy = app::Privacy::HideAmounts;
        txdetail::graph::scene_open(a, fixtures::id("full"));
    }),
    ("graph-light", |a| {
        a.prefs.theme = app::ThemeMode::Light;
        txdetail::graph::scene_open(a, fixtures::id("multi"));
    }),
    ("graph-expanded", |a| {
        txdetail::graph::scene_open(a, fixtures::id("multi"));
        a.graph.expanded = true;
    }),
    ("graph-2d", |a| {
        txdetail::graph::scene_open(a, fixtures::id("full"));
        a.graph.flat = true;
    }),
    ("graph-2d-multi", |a| {
        txdetail::graph::scene_open(a, fixtures::id("multi"));
        a.graph.flat = true;
    }),
    ("graph-hover", |a| {
        txdetail::graph::scene_open(a, fixtures::id("full"));
        a.graph.views[0].hover = Some("output:out-0".into());
        a.graph.active = Some("output:out-0".into());
    }),
    ("graph-de", |a| {
        a.prefs.lang = i18n::Lang::De;
        txdetail::graph::scene_open(a, fixtures::id("swap"));
    }),
];
