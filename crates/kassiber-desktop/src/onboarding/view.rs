//! Welcome's screens: the window frame (`WindowFrame` without a shell), the start page
//! (`StartChoicePanel`), the setup wizard (`index.tsx` + `steps/*`, `ConnectionsFields`,
//! `AiFields`) and the opened-folder panel (`ImportProjectPanel`).

use rune::{Color, Cx, El, TextStyle, col, el, row, text};
use rune_kit::Placement;
use rune_kit::TextInput;
use serde_json::Value;

use super::fields::{self as f, Choice, Text, ink, opt};
use super::model::{self as m, AiKind, AiMode, BackendKind, BackendMode, DbMode, Reach, Strength};
use super::{BackendRow, Flow, STEPS, Test};
use crate::app::App;
use crate::i18n::{Lang, t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::txdetail::widgets::{Size, Variant, button_surface, field, form_kit};
use crate::ui::ic;

const SM: f32 = 640.0;
const MD: f32 = 768.0;
const LG: f32 = 1024.0;

fn win(app: &App) -> (f32, f32) {
    let w = if app.shell.window_w > 0.0 {
        app.shell.window_w
    } else {
        1440.0
    };
    let h = if app.shell.window_h > 0.0 {
        app.shell.window_h
    } else {
        900.0
    };
    (w, h)
}

/// `WindowFrame` for a screen that draws no shell: the title bar row (window chrome on macOS,
/// the alpha notice at its right end) and the page on the inset panel.
pub fn root(app: &App, cx: &mut Cx<App>, layer: &mut Vec<El>) -> El {
    cx.web_focus();
    crate::shell::wordmark::observe(app, cx);
    let tk = app.tk();
    let (w, h) = win(app);
    let mut bar = row()
        .key("titlebar")
        .w_full()
        .h(40.0)
        .center_y()
        .justify_end()
        .pr(tk.r(0.75))
        .fill(tk.chrome)
        .window_drag();
    if app.prefs.alpha_visible {
        bar = bar.child(crate::shell::titlebar::alpha_chip(app, &tk, layer, false));
    }
    let wide = w >= MD;
    let gutter = if wide { tk.r(0.375) } else { 0.0 };
    let panel_h = (h - 40.0 - gutter).max(tk.r(20.0));
    let page = page(app, &tk, layer, panel_h - 2.0);
    let panel = col()
        .key("onb-panel")
        .w_full()
        .h_full()
        .fill(tk.background)
        .radius(if wide { tk.r(1.0) } else { 0.0 })
        .border(
            if wide { 1.0 } else { 0.0 },
            tk.border.alpha(tk.border.a * 0.7),
        )
        .border_layout()
        .clip_rounded(if wide { tk.r(1.0) } else { 0.0 })
        .child(page);
    let body = col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .px(gutter)
        .pb(gutter)
        .child(panel);
    let mut root = col()
        .key("sh-root")
        .w_full()
        .h_full()
        .fill(tk.chrome)
        .on_layout(|s: &mut App, r: [f32; 4]| {
            s.shell.window_w = r[2];
            s.shell.window_h = r[3];
        })
        .child(bar)
        .child(body);
    if matches!(app.shell.popup, Some(crate::shell::Popup::Alpha)) {
        root = root
            .on_background_press(|s: &mut App| s.shell.popup = None)
            .label(t("common:actions.close"));
    }
    let sys = cx.system_dark();
    if app.prefs.theme == crate::app::ThemeMode::System && sys != app.system_dark {
        root = root.child(
            el().key(format!("sh-sysdark-{sys}"))
                .absolute(0.0, 0.0)
                .size(1.0, 1.0)
                .on_layout(move |s: &mut App, _| s.system_dark = sys),
        );
    }
    crate::shell::overlays::toasts(app, &tk, layer);
    root
}

fn page(app: &App, tk: &Tk, layer: &mut Vec<El>, panel_h: f32) -> El {
    let ui = &app.onboarding;
    if ui.flow == Flow::Setup && ui.import_selection.is_none() {
        return setup(app, tk, layer, panel_h);
    }
    let content = if ui.import_selection.is_some() {
        import_panel(app, tk, panel_h)
    } else {
        // `errorBanner` above the start page (`px-6 pt-6`).
        match ui.finish_error.as_ref().or(ui.import_error.as_ref()) {
            Some(e) => col()
                .w_full()
                .child(
                    col()
                        .w_full()
                        .px(tk.r(1.5))
                        .pt(tk.r(1.5))
                        .child(f::error_banner(tk, e)),
                )
                .child(start(app, tk, layer, panel_h - tk.r(4.5))),
            None => start(app, tk, layer, panel_h),
        }
    };
    col()
        .key("onb-scroll")
        .w_full()
        .h(panel_h)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .child(content)
}

// ---- Start (`StartChoicePanel`) -----------------------------------------------------------------

/// "Try the demo": a fresh local copy of the bundled demo book, separate from the user's books.
pub fn demo_button(app: &App, tk: &Tk, blocked: bool) -> El {
    let busy = app.demo.busy();
    let label = t(if busy {
        "onboarding:start.tryDemo.titleOpening"
    } else {
        "onboarding:start.tryDemo.title"
    });
    let enabled = !busy && !blocked;
    let mut demo = f::button_lg(
        tk,
        "onb-demo",
        Variant::Ghost,
        Some(&I::FLASK_CONICAL),
        &label,
        None,
        enabled,
    )
    .help(t("onboarding:start.tryDemo.tooltip"));
    if enabled {
        demo = demo.on_press(crate::demo::start);
    }
    demo
}

fn start(app: &App, tk: &Tk, layer: &mut Vec<El>, panel_h: f32) -> El {
    let i = ink(tk);
    let ui = &app.onboarding;
    let (w, _) = win(app);
    let lg = w >= LG;
    let px = if w >= SM { tk.r(2.5) } else { tk.r(1.5) };
    let gap = if lg { tk.r(3.0) } else { tk.r(2.0) };
    let panel_w = w - 2.0 * if w >= MD { tk.r(0.375) } else { 0.0 } - 2.0;
    let inner_w = panel_w.min(tk.r(72.0)) - 2.0 * px;

    let mut actions = col().w_full().gap(tk.r(1.0));
    if ui.regtest.is_some() && !ui.loading_regtest {
        actions = actions.child(regtest_tile(app, tk));
    }
    let create = f::button_lg(
        tk,
        "onb-create",
        Variant::Default,
        Some(&I::BOOK_PLUS),
        &t("onboarding:start.createNew.title"),
        Some(&I::ARROW_RIGHT),
        true,
    )
    .on_press(super::begin_setup);
    let open_label = t(if ui.importing {
        "onboarding:start.openExisting.titleOpening"
    } else {
        "onboarding:start.openExisting.title"
    });
    let mut open = f::button_lg(
        tk,
        "onb-open",
        Variant::Outline,
        Some(&I::FOLDER_OPEN),
        &open_label,
        None,
        !ui.importing,
    )
    .help(t("onboarding:start.openExisting.tooltipAvailable"));
    if !ui.importing {
        open = open.on_press_tx(|a: &mut App, tx: &mut rune::Tx| super::begin_import(a, tx));
    }
    let mut buttons = row()
        .w_full()
        .flex_wrap()
        .gaps(tk.r(0.75), tk.r(0.75))
        .center_y()
        .child(create)
        .child(open);
    // The bundled demo book (`crate::demo`): with developer tools on (the lock screen offers it
    // too).
    if app.prefs.developer_tools {
        buttons = buttons.child(demo_button(app, tk, ui.importing));
    }
    actions = actions.child(buttons);
    // `text-balance`: two lines of even length where the column holds it in two.
    let words = t("onboarding:start.headline");
    let shown = if lg { balance(&words) } else { words.clone() };
    let headline = text(shown)
        .label(words)
        .font(
            TextStyle::new(tk.r(3.0), 600.0)
                .lh(tk.r(3.15))
                .tracking(-tk.r(3.0) * 0.025),
        )
        .color(i.ink)
        .role("heading")
        .level(1)
        .wrap();
    let left_w = if lg { (inner_w - gap) / 2.1 } else { inner_w };
    let left = col()
        .w(left_w)
        .gap(tk.r(2.25))
        .child(crate::shell::wordmark::animated(
            app,
            crate::shell::wordmark::Site::Start,
            2.0,
            tk.fg,
            false,
        ))
        .child(headline)
        .child(actions);

    let art_h = if lg { tk.r(30.0) } else { tk.r(14.0) };
    let art_w = if lg {
        (inner_w - gap) * 1.1 / 2.1
    } else {
        inner_w
    };
    let state = ui.chain;
    let art = super::chain::view(
        super::chain::Paint {
            tk,
            mined: 3,
            state,
            since_beat: (app.clock - state.at).max(0.0),
            clock: app.clock,
            live: true,
        },
        art_w,
        art_h,
    );

    let mut highlights = row().gap(tk.r(1.5)).role("list");
    for (glyph, key) in [
        (&I::DATABASE, "localDatabase"),
        (&I::LOCK_KEYHOLE, "encrypted"),
        (&I::EYE, "watchOnly"),
    ] {
        let s = t(&format!("onboarding:start.highlights.{key}.title"));
        highlights = highlights.child(
            row()
                .gap(tk.r(0.5))
                .center_y()
                .role("listitem")
                .label(s.clone())
                .child(ic(glyph, tk.r(0.875), i.ink3))
                .child(text(s).font(tk.tsm(400.0)).color(i.ink2)),
        );
    }
    let footer = row()
        .w(inner_w)
        .flex_wrap()
        .gaps(tk.r(1.0), tk.r(1.0))
        .center_y()
        .justify_between()
        .pt(tk.r(1.5))
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
        .border_layout()
        .child(highlights)
        .child(language_select(app, tk, layer));

    let grid = if lg {
        col()
            .gap(gap)
            .child(row().w(inner_w).gap(gap).center_y().child(left).child(art))
            .child(footer)
    } else {
        col().gap(gap).child(art).child(left).child(footer)
    };
    col()
        .key("onb-start")
        .w_full()
        .min_h(panel_h)
        .items_center()
        .justify_center()
        .px(px)
        .py(tk.r(2.5))
        .child(grid)
}

/// Splits a short heading into two lines at the word break that evens their lengths.
fn balance(s: &str) -> String {
    let words: Vec<&str> = s.split(' ').collect();
    let total = s.chars().count() as i64;
    let best = (1..words.len()).min_by_key(|&k| {
        let left = words[..k].join(" ").chars().count() as i64;
        (2 * left - total).abs()
    });
    match best {
        Some(k) => format!("{}\n{}", words[..k].join(" "), words[k..].join(" ")),
        None => s.to_owned(),
    }
}

fn regtest_tile(app: &App, tk: &Tk) -> El {
    let i = ink(tk);
    let opening = app.onboarding.opening_regtest;
    let emerald = rune::color::hex(if tk.dark { "#34d399" } else { "#10b981" }).expect("hex");
    let title = t(if opening {
        "onboarding:start.regtest.titleOpening"
    } else {
        "onboarding:start.regtest.title"
    });
    let mut b = row()
        .key("onb-regtest")
        .w_full()
        .max_w(tk.r(28.0))
        .gap(tk.r(0.75))
        .center_y()
        .padding(tk.r(0.75))
        .radius(tk.r(0.75))
        .border(1.0, emerald.alpha(if tk.dark { 0.3 } else { 0.35 }))
        .fill(emerald.alpha(0.1))
        .hover(emerald.alpha(0.15))
        .role("button")
        .label(title.clone())
        .focusable(!opening)
        .disabled(opening)
        .child(
            row()
                .size(tk.r(2.0), tk.r(2.0))
                .center()
                .radius(tk.r(0.5))
                .fill(rune::color::hex("#10b981").expect("hex"))
                .child(ic(&I::SERVER, tk.r(1.0), Color::WHITE)),
        )
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .child(text(title).font(tk.tsm(600.0)).color(i.ink))
                .child(
                    text(t("onboarding:start.regtest.body"))
                        .font(tk.txs(400.0).lh(tk.r(1.25)))
                        .color(i.ink2)
                        .wrap()
                        .mt(tk.r(0.125)),
                ),
        )
        .child(ic(&I::ARROW_RIGHT, tk.r(1.0), i.ink));
    if opening {
        b = b.opacity(0.8);
    } else {
        b = b.on_press(super::open_regtest);
    }
    b
}

/// `StartLanguageSelect`: the UI language before setup (the same setting as Appearance).
fn language_select(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let i = ink(tk);
    let open = app.onboarding.lang_open;
    let current = app.prefs.lang;
    let trigger = row()
        .key("onb-lang")
        .h(tk.r(2.0))
        .px(tk.r(0.75))
        .gap(tk.r(0.5))
        .center_y()
        .radius(tk.r(0.375))
        .border(1.0, i.line)
        .border_layout()
        .fill(if tk.dark {
            tk.input.alpha(tk.input.a * 0.3)
        } else {
            Color::TRANSPARENT
        })
        .hover(if tk.dark {
            tk.input.alpha(tk.input.a * 0.5)
        } else {
            tk.accent
        })
        .role("combobox")
        .label(t("onboarding:start.language"))
        .a11y_value(current.label())
        .focusable(true)
        .child(ic(&I::LANGUAGES, tk.r(1.0), tk.muted_fg))
        .child(text(current.label()).font(tk.tsm(400.0)).color(i.ink2))
        .child(ic(&I::CHEVRON_DOWN, tk.r(1.0), tk.muted_fg.alpha(0.5)));
    let trigger = crate::ui::popup::trigger(
        trigger,
        crate::ui::popup::Kind::Listbox,
        open,
        |a: &mut App| a.onboarding.lang_open = !a.onboarding.lang_open,
    );
    if open {
        let mut p = crate::ui::popup::Popup::listbox("onb-lang", |a: &mut App| {
            a.onboarding.lang_open = false
        })
        .list_key("onb-lang-menu");
        for lang in Lang::ALL {
            let checked = lang == current;
            p.item(
                row()
                    .w_full()
                    .min_h(tk.r(2.0))
                    .pl(tk.r(0.5))
                    .pr(tk.r(2.0))
                    .center_y()
                    .radius(tk.r(0.25))
                    .role("option")
                    .checked(checked)
                    .label(lang.label())
                    .hover(tk.accent)
                    .focus_highlight(tk.accent)
                    .on_press(move |a: &mut App| {
                        a.prefs.lang = lang;
                        a.onboarding.lang_open = false;
                    })
                    .child(
                        text(lang.label())
                            .font(tk.tsm(400.0))
                            .color(tk.fg)
                            .grow(1.0),
                    )
                    .child(
                        row()
                            .absolute(0.0, 0.0)
                            .w_full()
                            .h(tk.r(2.0))
                            .justify_end()
                            .pr(tk.r(0.5))
                            .center_y()
                            .child(if checked {
                                ic(&I::CHECK, tk.r(1.0), tk.fg)
                            } else {
                                el()
                            }),
                    ),
                format!("onb-lang-{}", lang.code()),
                lang.label(),
                true,
                checked,
            );
        }
        let list = col()
            .min_w(tk.r(8.0))
            .padding(tk.r(0.25))
            .label(t("onboarding:start.language"));
        p.mount(
            tk,
            layer,
            list,
            |l| crate::shell::glass(tk, col().min_h(0.0).child(l)),
            Placement::BottomStart,
            tk.r(0.25),
        );
    }
    trigger
}

// ---- Setup wizard -------------------------------------------------------------------------------

fn setup(app: &App, tk: &Tk, layer: &mut Vec<El>, panel_h: f32) -> El {
    let i = ink(tk);
    let ui = &app.onboarding;
    let (w, _) = win(app);
    let sm = w >= SM;
    let labels: Vec<String> = STEPS
        .iter()
        .map(|s| t(&format!("onboarding:steps.{s}")))
        .collect();
    let summaries: Vec<Option<String>> = (0..STEPS.len())
        .map(|n| m::step_summary(n, &ui.form))
        .collect();
    let header = col()
        .w_full()
        .px(if sm { tk.r(1.5) } else { tk.r(1.25) })
        .pt(tk.r(1.25))
        .pb(tk.r(1.0))
        .child(f::stepper(tk, &labels, &summaries, ui.step, !sm));
    let mut body = col()
        .w_full()
        .gap(tk.r(1.0))
        .px(if sm { tk.r(1.5) } else { tk.r(1.25) })
        .py(tk.r(1.5))
        .fill(tk.background.alpha(0.6))
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border.alpha(tk.border.a * 0.7))
        .border_layout();
    if let Some(e) = &ui.finish_error {
        body = body.child(f::error_banner(tk, e));
    }
    body = body.child(match ui.step {
        0 => essentials(app, tk, layer),
        1 => sync_step(app, tk, layer),
        2 => ai_step(app, tk, layer),
        3 => security_step(app, tk),
        _ => review_step(app, tk),
    });
    let card = col()
        .key("onb-card")
        .w_full()
        .radius(tk.r(1.0))
        .border(1.0, tk.border)
        .border_layout()
        .fill(tk.card)
        .shadow(1.0, 2.0, Color::BLACK.alpha(0.05))
        .clip_rounded(tk.r(1.0))
        .child(header)
        .child(body);
    let brand = row()
        .w_full()
        .gap(tk.r(0.75))
        .center()
        .pb(tk.r(1.5))
        .child(crate::shell::wordmark::wordmark(2.0, tk.fg, false))
        .child(
            text(t("onboarding:frame.setup"))
                .font(tk.tbase(400.0))
                .color(i.ink3),
        );
    let inner = col()
        .key("onb-setup")
        .w_full()
        .max_w(tk.r(48.0))
        .min_h(panel_h)
        .justify_center()
        .px(if sm { tk.r(1.5) } else { tk.r(0.75) })
        .py(if sm { tk.r(2.5) } else { tk.r(1.5) })
        .child(brand)
        .child(card);
    // A step opens at its top: the pane is keyed by the step.
    col()
        .key(format!("onb-pane-{}", ui.step))
        .w_full()
        .h(panel_h)
        .items_center()
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .child(inner)
}

fn form_body(children: Vec<El>, tk: &Tk) -> El {
    let mut c = col().w_full().gap(tk.r(1.5));
    for ch in children {
        c = c.child(ch);
    }
    c
}

fn actions(app: &App, tk: &Tk, label: Option<String>, busy_label: Option<String>) -> El {
    let ui = &app.onboarding;
    let can = ui.form.step_complete(ui.step) && !ui.submitting;
    f::step_actions(
        tk,
        Some(super::go_back),
        label,
        busy_label,
        ui.submitting,
        !can,
        super::submit,
    )
}

fn touched(app: &App, key: &'static str) -> bool {
    app.onboarding.touched.contains(key)
}

/// "Your books".
fn essentials(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let form = &app.onboarding.form;
    let austrian = form.tax_country == "at";
    let mut ws = Text::new(
        "workspace",
        t("onboarding:essentials.booksName"),
        &form.workspace,
        t("onboarding:essentials.booksNamePlaceholder"),
        |a| &mut a.onboarding.form.workspace,
    );
    ws.description = Some(t("onboarding:essentials.booksNameDescription"));
    let countries: Vec<f::Opt> = crate::reports::books::model::COUNTRIES
        .iter()
        .map(|c| f::Opt {
            value: c.code.into(),
            label: m::region_label(c.code),
            badge: c.preview.then(|| t("common:taxCountry.previewBadge")),
        })
        .collect();
    let country = f::select(
        app,
        tk,
        layer,
        "onb-tax-country",
        &t("onboarding:essentials.taxJurisdiction"),
        &form.tax_country,
        countries,
        None,
        |a: &mut App, v: &str| {
            let f = &mut a.onboarding.form;
            if v == f.tax_country {
                return;
            }
            f.tax_country = v.to_owned();
            if let Some(fiat) = m::required_fiat(v) {
                f.fiat = fiat.to_owned();
                f.gains = m::default_gains(v);
            } else if !m::gains_for(v).contains(&f.gains) {
                f.gains = m::default_gains(v);
            }
        },
    );
    let mut tax = col()
        .w_full()
        .gap(tk.r(1.0))
        .padding(tk.r(1.0))
        .child(country);
    if crate::reports::books::model::is_preview(&form.tax_country) {
        let badge = t("common:taxCountry.previewBadge");
        let n = badge.len() + 2;
        tax = tax.child(
            row()
                .w_full()
                .px(tk.r(0.75))
                .py(tk.r(0.5))
                .radius(tk.r(0.375))
                .border(1.0, ink(tk).line)
                .role("note")
                .child(
                    rune::rich(
                        format!("{badge}: {}", t("common:taxCountry.previewNotice")),
                        vec![rune::Span::new(0, n).weight(500.0)],
                    )
                    .font(tk.txs(400.0).lh(tk.r(1.25)))
                    .color(ink(tk).ink)
                    .wrap(),
                ),
        );
    }
    let mut fiats: Vec<f::Opt> = m::FIAT_CURRENCIES.iter().map(|c| opt(*c, *c)).collect();
    if !m::FIAT_CURRENCIES.contains(&form.fiat.as_str()) {
        fiats.push(opt(form.fiat.clone(), form.fiat.clone()));
    }
    let sample = tr(
        "onboarding:essentials.fiatSample",
        &[("sample", &crate::format::fiat(1234.56, &form.fiat))],
    );
    let fiat = f::select(
        app,
        tk,
        layer,
        "onb-fiat",
        &t("onboarding:essentials.fiatCurrency"),
        &form.fiat,
        fiats,
        Some(sample),
        |a: &mut App, v: &str| a.onboarding.form.fiat = v.to_owned(),
    );
    let methods: Vec<f::Opt> = m::gains_for(&form.tax_country)
        .into_iter()
        .map(|g| opt(g.clone(), m::method_label(&g)))
        .collect();
    let method_label = t(if austrian {
        "onboarding:essentials.accountingMethod"
    } else {
        "onboarding:essentials.lotSelection"
    });
    let method = f::select(
        app,
        tk,
        layer,
        "onb-method",
        &method_label,
        &form.gains,
        methods,
        None,
        |a: &mut App, v: &str| a.onboarding.form.gains = v.to_owned(),
    );
    let (w, _) = win(app);
    let pair = if w >= SM {
        row()
            .w_full()
            .gap(tk.r(1.0))
            .items_start()
            .child(col().w(0.0).grow(1.0).min_w(0.0).child(fiat))
            .child(col().w(0.0).grow(1.0).min_w(0.0).child(method))
    } else {
        col().w_full().gap(tk.r(1.0)).child(fiat).child(method)
    };
    tax = tax.child(pair);
    if m::uses_long_term_days(&form.tax_country) {
        let mut days = Text::new(
            "taxLongTermDays",
            t("onboarding:essentials.longTermDays"),
            &form.long_term_days,
            t("onboarding:essentials.longTermDaysPlaceholder"),
            |a| &mut a.onboarding.form.long_term_days,
        );
        days.hint = m::long_term_days_hint(&form.long_term_days.text).map(str::to_owned);
        days.touched = touched(app, "taxLongTermDays");
        tax = tax.child(days.build(tk));
    }
    let content = form_body(
        vec![
            ws.build(tk),
            f::section(tk, &t("onboarding:essentials.taxHeading"), vec![tax]),
            actions(app, tk, None, None),
        ],
        tk,
    );
    f::step_page(tk, &t("onboarding:essentials.title"), None, content)
}

/// "Sync": the server choice and update checks.
fn sync_step(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let form = &app.onboarding.form;
    let offline = form.backend_mode == Some(BackendMode::Skip);
    let updates = form.update_checks && !offline;
    let i = ink(tk);
    let status = if updates {
        row()
            .flex_wrap()
            .gaps(tk.r(0.25), tk.r(0.5))
            .center_y()
            .child(f::network_status(
                tk,
                Reach::Internet,
                &f::reach_label(Reach::Internet, "updates"),
            ))
            .child(
                text("api.github.com")
                    .font(tk.t2xs(400.0).mono())
                    .color(i.ink2),
            )
    } else {
        f::network_status(
            tk,
            Reach::None,
            &t(if offline {
                "onboarding:network.offOffline"
            } else {
                "onboarding:network.none"
            }),
        )
    };
    let sw = crate::settings::controls::switch(
        tk,
        "allow-update-checks",
        updates,
        &t("onboarding:sync.updateChecks"),
        !offline,
        |a: &mut App, on: bool| a.onboarding.form.update_checks = on,
    );
    let desc = text(t("onboarding:sync.updateChecksDescription"))
        .font(tk.txs(400.0).lh(tk.r(1.125)))
        .color(i.ink3)
        .wrap();
    let row_el = f::srow(
        tk,
        f::row_title(tk, &t("onboarding:sync.updateChecks")),
        Some(desc),
        Some(status),
        Some(sw),
        offline,
    );
    let content = form_body(
        vec![
            connections(app, tk, layer),
            f::section(tk, &t("onboarding:sync.updateChecksHeading"), vec![row_el]),
            actions(app, tk, None, None),
        ],
        tk,
    );
    f::step_page(tk, &t("onboarding:sync.title"), None, content)
}

/// Built-in servers as host lines (`serverList`).
fn server_list(rows: &[&BackendRow]) -> Vec<(String, Option<String>)> {
    let mut seen = std::collections::HashSet::new();
    let mut out = vec![];
    for b in rows {
        let host = m::endpoint_host(&b.url);
        if host.is_empty() || !seen.insert(host.clone()) {
            continue;
        }
        out.push((host, Some(m::kind_label(&b.kind).to_owned())));
    }
    out
}

/// `ConnectionsFields`.
fn connections(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let ui = &app.onboarding;
    let form = &ui.form;
    let defaults: Vec<BackendRow> = ui.defaults.clone().unwrap_or_default();
    let liquid: Vec<&BackendRow> = defaults.iter().filter(|b| b.liquid_opt_in).collect();
    let shown: Vec<&BackendRow> = defaults
        .iter()
        .filter(|b| form.include_liquid || !b.liquid_opt_in)
        .collect();
    let custom = form.backend_mode == Some(BackendMode::Custom);
    let electrum = custom && form.backend_kind == BackendKind::Electrum;
    let electrum_url = m::electrum_url(
        &form.backend_host.text,
        &form.backend_port.text,
        form.use_ssl,
    );
    let custom_endpoint = if !custom {
        String::new()
    } else if electrum {
        if form.backend_host.text.trim().is_empty() {
            String::new()
        } else {
            electrum_url.clone()
        }
    } else {
        form.backend_url.text.trim().to_owned()
    };
    let onion = custom && m::is_onion(&form.effective_backend_url());
    let hint = if custom {
        m::endpoint_hint(form.backend_kind, &form.effective_backend_url())
    } else {
        None
    };

    // Built-in servers.
    let mut default_children = vec![];
    if !liquid.is_empty() {
        let i = ink(tk);
        let mut d = col().gap(tk.r(0.25)).child(f::check_desc(
            tk,
            &t("onboarding:connections.default.liquidDescription"),
        ));
        for (h, l) in server_list(&liquid) {
            d = d.child(
                row()
                    .gap(tk.r(1.0))
                    .child(
                        text(h)
                            .font(tk.t2xs(400.0).mono().lh(tk.r(1.25)))
                            .color(i.ink2)
                            .w(tk.r(15.0))
                            .wrap(),
                    )
                    .child(
                        text(l.unwrap_or_default())
                            .font(tk.t2xs(400.0).lh(tk.r(1.25)))
                            .color(i.ink3),
                    ),
            );
        }
        default_children.push(f::check_row(
            tk,
            "include-liquid-backends",
            form.include_liquid,
            &t("onboarding:connections.default.liquidLabel"),
            d,
            |a| a.onboarding.form.include_liquid = !a.onboarding.form.include_liquid,
        ));
    }
    let default_card = f::choice(
        tk,
        Choice {
            key: "onb-sync-default",
            active: form.backend_mode == Some(BackendMode::Default),
            glyph: Some(&I::EARTH),
            title: t("onboarding:connections.default.title"),
            description: t("onboarding:connections.default.description"),
            detail: Some(t("onboarding:connections.default.tradeoff")),
            network: Some(m::Network {
                reach: Reach::Internet,
                label: f::reach_label(Reach::Internet, "sync"),
                hosts: server_list(&shown),
                pending: Some(t("onboarding:network.loadingServers")),
            }),
            warning: false,
            on_press: |a| {
                let f = &mut a.onboarding.form;
                f.backend_mode = Some(BackendMode::Default);
                f.backend_kind = BackendKind::Electrum;
                m::set_text(&mut f.backend_name, m::DEFAULT_BACKEND_NAME);
                m::set_text(&mut f.backend_url, m::DEFAULT_BACKEND_URL);
                m::set_text(&mut f.backend_host, m::DEFAULT_ELECTRUM_HOST);
                m::set_text(&mut f.backend_port, m::DEFAULT_ELECTRUM_SSL_PORT);
                f.use_ssl = true;
                super::reset_test(a);
            },
        },
        default_children,
    );

    // Your own server.
    let mut cc: Vec<El> = vec![];
    if custom {
        let kinds: Vec<f::Opt> = BackendKind::ALL
            .iter()
            .map(|k| opt(k.id(), k.label()))
            .collect();
        cc.push(f::select(
            app,
            tk,
            layer,
            "onb-backend-kind",
            &t("onboarding:connections.protocolLabel"),
            form.backend_kind.id(),
            kinds,
            None,
            |a: &mut App, v: &str| {
                if let Some(k) = BackendKind::from_id(v) {
                    a.onboarding.form.backend_kind = k;
                }
                super::reset_test(a);
            },
        ));
        cc.push(
            Text::new(
                "backendName",
                t("onboarding:connections.displayName"),
                &form.backend_name,
                t("onboarding:connections.displayNamePlaceholder"),
                |a| &mut a.onboarding.form.backend_name,
            )
            .build(tk),
        );
        if electrum {
            let mut host = Text::new(
                "backendHost",
                t("onboarding:connections.host"),
                &form.backend_host,
                t("onboarding:connections.hostPlaceholder"),
                |a| &mut a.onboarding.form.backend_host,
            );
            host.hint = hint.map(str::to_owned);
            host.touched = touched(app, "backendHost");
            host.on_change = Some(super::reset_test);
            let port_ph = if form.use_ssl {
                m::DEFAULT_ELECTRUM_SSL_PORT
            } else {
                m::DEFAULT_ELECTRUM_TCP_PORT
            };
            let mut port = Text::new(
                "backendPort",
                t("onboarding:connections.port"),
                &form.backend_port,
                port_ph.into(),
                |a| &mut a.onboarding.form.backend_port,
            );
            // The port is prefilled, so its hint waits for a host (it would point at the wrong field).
            port.hint = if form.backend_host.text.trim().is_empty() {
                None
            } else {
                hint.map(str::to_owned)
            };
            port.touched = touched(app, "backendPort");
            port.on_change = Some(super::reset_test);
            cc.push(
                row()
                    .w_full()
                    .gap(tk.r(0.75))
                    .items_start()
                    .child(col().grow(1.0).min_w(0.0).child(host.build(tk)))
                    .child(col().w(130.0).child(port.build(tk))),
            );
            cc.push(f::check_row(
                tk,
                "backend-use-ssl",
                form.use_ssl,
                &t("onboarding:connections.useSsl"),
                f::check_desc(tk, &t("onboarding:connections.useSslDescription")),
                |a| {
                    let f = &mut a.onboarding.form;
                    let ssl = !f.use_ssl;
                    f.use_ssl = ssl;
                    if !ssl {
                        f.trust_ssl = false;
                        m::set_text(&mut f.certificate, "");
                    }
                    if f.backend_port.text == m::DEFAULT_ELECTRUM_SSL_PORT
                        || f.backend_port.text == m::DEFAULT_ELECTRUM_TCP_PORT
                    {
                        m::set_text(
                            &mut f.backend_port,
                            if ssl {
                                m::DEFAULT_ELECTRUM_SSL_PORT
                            } else {
                                m::DEFAULT_ELECTRUM_TCP_PORT
                            },
                        );
                    }
                    super::reset_test(a);
                },
            ));
            if form.use_ssl {
                cc.push(f::check_row(
                    tk,
                    "backend-trust-ssl",
                    form.trust_ssl,
                    &t("onboarding:connections.trustSsl"),
                    f::check_desc(tk, &t("onboarding:connections.trustSslDescription")),
                    |a| {
                        a.onboarding.form.trust_ssl = !a.onboarding.form.trust_ssl;
                        super::reset_test(a);
                    },
                ));
            }
            let mut cert = Text::new(
                "backendCertificate",
                t("onboarding:connections.certificate"),
                &form.certificate,
                t("onboarding:connections.certificatePlaceholder"),
                |a| &mut a.onboarding.form.certificate,
            );
            cert.description = Some(t(if form.trust_ssl {
                "onboarding:connections.certificateIgnored"
            } else {
                "onboarding:connections.certificateSystemStore"
            }));
            cert.disabled = form.trust_ssl;
            cert.on_change = Some(super::reset_test);
            cc.push(cert.build(tk));
            // Test connection: the one network action here, on its press.
            let testing = ui.test == Test::Testing;
            let glyph = match ui.test {
                Test::Ok => &I::CIRCLE_CHECK,
                Test::Fail => &I::CIRCLE_X,
                _ => &I::CIRCLE_HELP,
            };
            let label = t(if testing {
                "onboarding:connections.testing"
            } else {
                "onboarding:connections.testConnection"
            });
            let enabled = hint.is_none() && !testing;
            let mut b = f::button(
                tk,
                "onb-test",
                Variant::Outline,
                Size::Default,
                Some(glyph),
                &label,
                enabled,
            );
            if enabled {
                b = b.on_press(super::run_electrum_test);
            }
            let mut test = col().w_full().gap(tk.r(0.75)).child(
                row()
                    .flex_wrap()
                    .gaps(tk.r(0.75), tk.r(0.75))
                    .center_y()
                    .child(b)
                    .child(
                        text(t("onboarding:connections.testHint"))
                            .font(tk.txs(400.0))
                            .color(ink(tk).ink2),
                    ),
            );
            if ui.test != Test::Idle || !ui.test_log.is_empty() {
                test = test.child(
                    col()
                        .key("onb-test-log")
                        .w_full()
                        .min_h(tk.r(7.0))
                        .padding(tk.r(0.75))
                        .radius(tk.r(0.5))
                        .border(1.0, tk.border)
                        .fill(ink(tk).paper2)
                        .role("textbox")
                        .label(t("onboarding:connections.testLogLabel"))
                        .a11y_value(ui.test_log.clone())
                        .child(
                            text(ui.test_log.clone())
                                .font(tk.txs(400.0).mono().lh(tk.r(1.25)))
                                .color(ink(tk).ink)
                                .wrap(),
                        ),
                );
            }
            cc.push(test);
        } else {
            let mut url = Text::new(
                "backendUrl",
                t("onboarding:connections.endpointUrl"),
                &form.backend_url,
                t("onboarding:connections.endpointUrlPlaceholder"),
                |a| &mut a.onboarding.form.backend_url,
            );
            url.hint = hint.map(str::to_owned);
            url.touched = touched(app, "backendUrl");
            url.description = Some(m::endpoint_description(form.backend_kind).to_owned());
            url.on_change = Some(super::reset_test);
            cc.push(url.build(tk));
        }
        if onion {
            let h = |s: &str| rune::color::hex(s).expect("hex");
            let (bd, bg, fg) = if tk.dark {
                (
                    h("#3b82f6").alpha(0.3),
                    h("#3b82f6").alpha(0.1),
                    h("#dbeafe"),
                )
            } else {
                (h("#bfdbfe"), h("#eff6ff"), h("#172554"))
            };
            cc.push(
                col()
                    .w_full()
                    .padding(tk.r(0.75))
                    .radius(tk.r(0.5))
                    .border(1.0, bd)
                    .fill(bg)
                    .child(
                        text(t("onboarding:connections.onionDetectedTitle"))
                            .font(tk.txs(500.0).lh(tk.r(1.25)))
                            .color(fg),
                    )
                    .child(
                        text(t("onboarding:connections.onionDetectedBody"))
                            .font(tk.txs(400.0).lh(tk.r(1.25)))
                            .color(fg)
                            .wrap()
                            .mt(tk.r(0.25)),
                    ),
            );
        }
        cc.push(f::check_row(
            tk,
            "backend-use-proxy",
            form.use_proxy,
            &t("onboarding:connections.useProxy"),
            f::check_desc(tk, &t("onboarding:connections.useProxyDescription")),
            |a| {
                a.onboarding.form.use_proxy = !a.onboarding.form.use_proxy;
                super::reset_test(a);
            },
        ));
        if form.use_proxy {
            let mut ph = Text::new(
                "backendProxyHost",
                t("onboarding:connections.proxyHost"),
                &form.proxy_host,
                t("onboarding:connections.proxyHostPlaceholder"),
                |a| &mut a.onboarding.form.proxy_host,
            );
            ph.on_change = Some(super::reset_test);
            let mut pp = Text::new(
                "backendProxyPort",
                t("onboarding:connections.port"),
                &form.proxy_port,
                t("onboarding:connections.proxyPortPlaceholder"),
                |a| &mut a.onboarding.form.proxy_port,
            );
            pp.on_change = Some(super::reset_test);
            cc.push(
                row()
                    .w_full()
                    .gap(tk.r(0.75))
                    .items_start()
                    .child(col().grow(1.0).min_w(0.0).child(ph.build(tk)))
                    .child(col().w(130.0).child(pp.build(tk))),
            );
        }
        cc.push(f::note(
            tk,
            &I::KEY_ROUND,
            &t("onboarding:connections.credentialsWarning"),
        ));
    }
    let reach = if custom_endpoint.is_empty() {
        Reach::Internet
    } else {
        m::endpoint_reach(&custom_endpoint)
    };
    let custom_label = if custom_endpoint.is_empty() {
        tr(
            "onboarding:network.label",
            &[
                ("reach", &t("onboarding:network.reach.yourServer")),
                ("when", &t("onboarding:network.when.sync")),
            ],
        )
    } else {
        f::reach_label(reach, "sync")
    };
    let custom_hosts = if custom_endpoint.is_empty() {
        vec![]
    } else {
        vec![(m::endpoint_host(&custom_endpoint), None)]
    };
    let custom_card = f::choice(
        tk,
        Choice {
            key: "onb-sync-custom",
            active: custom,
            glyph: Some(&I::SERVER_COG),
            title: t("onboarding:connections.custom.title"),
            description: t("onboarding:connections.custom.description"),
            detail: Some(t("onboarding:connections.custom.tradeoff")),
            network: Some(m::Network {
                reach,
                label: custom_label,
                hosts: custom_hosts,
                pending: Some(t("onboarding:network.customPending")),
            }),
            warning: false,
            on_press: |a| {
                let f = &mut a.onboarding.form;
                f.backend_mode = Some(BackendMode::Custom);
                if f.backend_name.text == m::DEFAULT_BACKEND_NAME {
                    m::set_text(&mut f.backend_name, "");
                }
                if f.backend_url.text == m::DEFAULT_BACKEND_URL {
                    m::set_text(&mut f.backend_url, "");
                }
                if f.backend_host.text == m::DEFAULT_ELECTRUM_HOST {
                    m::set_text(&mut f.backend_host, "");
                }
                super::reset_test(a);
            },
        },
        cc,
    );
    let skip_card = f::choice(
        tk,
        Choice {
            key: "onb-sync-skip",
            active: form.backend_mode == Some(BackendMode::Skip),
            glyph: Some(&I::WIFI_OFF),
            title: t("onboarding:connections.skip.title"),
            description: t("onboarding:connections.skip.description"),
            detail: None,
            network: Some(m::no_connections()),
            warning: true,
            on_press: |a| {
                // Offline keeps the AI and update answers, so going back online restores them.
                a.onboarding.form.backend_mode = Some(BackendMode::Skip);
                super::reset_test(a);
            },
        },
        vec![],
    );
    col()
        .w_full()
        .gap(tk.r(0.75))
        .child(default_card)
        .child(custom_card)
        .child(skip_card)
}

/// "AI".
fn ai_step(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let form = &app.onboarding.form;
    let content = if form.backend_mode == Some(BackendMode::Skip) {
        let i = ink(tk);
        row()
            .w_full()
            .gap(tk.r(0.75))
            .items_start()
            .padding(tk.r(1.0))
            .radius(tk.r(0.75))
            .border(1.0, tk.border)
            .child(ic(&I::WIFI_OFF, tk.r(1.0), i.ink3).mt(tk.r(0.25)))
            .child(
                text(t("onboarding:ai.offlineNote"))
                    .font(tk.tsm(400.0).lh(tk.r(1.5)))
                    .color(i.ink2)
                    .wrap()
                    .grow(1.0),
            )
    } else {
        ai_fields(app, tk, layer)
    };
    f::step_page(
        tk,
        &t("onboarding:aiStep.title"),
        None,
        form_body(vec![content, actions(app, tk, None, None)], tk),
    )
}

/// `AiFields`'s provider fields, inside whichever endpoint card is chosen.
fn provider_fields(app: &App, tk: &Tk, layer: &mut Vec<El>) -> Vec<El> {
    let form = &app.onboarding.form;
    let local = form.ai_mode == AiMode::Local;
    let remote = form.ai_mode == AiMode::Remote;
    let hint = match form.ai_mode {
        AiMode::Disabled => None,
        AiMode::Local => m::local_ai_url_hint(&form.ai_url.text),
        AiMode::Remote => m::ai_url_hint(&form.ai_url.text),
    };
    let mut v: Vec<El> = vec![];
    if remote {
        let kinds = vec![
            opt(AiKind::Remote.id(), AiKind::Remote.id()),
            opt(AiKind::Tee.id(), AiKind::Tee.id()),
        ];
        v.push(f::select(
            app,
            tk,
            layer,
            "onb-ai-kind",
            &t("onboarding:ai.providerPrivacy"),
            form.ai_kind.id(),
            kinds,
            Some(t("onboarding:ai.providerPrivacyDescription")),
            |a: &mut App, v: &str| {
                if let Some(k) = AiKind::from_id(v) {
                    a.onboarding.form.ai_kind = k;
                }
            },
        ));
    }
    let name_ph = t(if local {
        "onboarding:ai.providerNamePlaceholderLocal"
    } else {
        "onboarding:ai.providerNamePlaceholderRemote"
    });
    v.push(
        Text::new(
            "aiProviderName",
            t("onboarding:ai.providerName"),
            &form.ai_name,
            name_ph,
            |a| &mut a.onboarding.form.ai_name,
        )
        .build(tk),
    );
    let url_ph = if local {
        m::DEFAULT_AI_BASE_URL.to_owned()
    } else {
        t("onboarding:ai.baseUrlPlaceholderRemote")
    };
    let mut url = Text::new(
        "aiBaseUrl",
        t("onboarding:ai.baseUrl"),
        &form.ai_url,
        url_ph,
        |a| &mut a.onboarding.form.ai_url,
    );
    url.hint = hint.map(str::to_owned);
    url.touched = touched(app, "aiBaseUrl");
    url.description = Some(t("onboarding:ai.baseUrlDescription"));
    v.push(url.build(tk));
    if remote {
        v.push(f::check_row(
            tk,
            "ai-remote-ack",
            form.ai_remote_ack,
            &t("onboarding:ai.remoteAck"),
            f::check_desc(tk, &t("onboarding:ai.remoteAckDescription")),
            |a| a.onboarding.form.ai_remote_ack = !a.onboarding.form.ai_remote_ack,
        ));
    }
    v
}

/// `AiFields`.
fn ai_fields(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let form = &app.onboarding.form;
    let local = form.ai_mode == AiMode::Local;
    let remote = form.ai_mode == AiMode::Remote;
    // What each option would contact. The URL field is shared, so an option that isn't
    // chosen shows its default rather than the other one's entry.
    let network_note = |url: String, pending: String| {
        let reach = if url.is_empty() {
            Reach::Internet
        } else {
            m::endpoint_reach(&url)
        };
        m::Network {
            reach,
            label: f::reach_label(reach, "assistant"),
            hosts: if url.is_empty() {
                vec![]
            } else {
                vec![(m::endpoint_host(&url), None)]
            },
            pending: Some(pending),
        }
    };
    let local_url = if local {
        form.ai_url.text.trim().to_owned()
    } else {
        m::DEFAULT_AI_BASE_URL.to_owned()
    };
    let remote_url = if remote {
        form.ai_url.text.trim().to_owned()
    } else {
        String::new()
    };
    let local_children = if local {
        provider_fields(app, tk, layer)
    } else {
        vec![]
    };
    let local_card = f::choice(
        tk,
        Choice {
            key: "onb-ai-local",
            active: local,
            glyph: Some(&I::BRAIN_CIRCUIT),
            title: t("onboarding:ai.local.title"),
            description: t("onboarding:ai.local.description"),
            detail: None,
            network: Some(network_note(
                local_url,
                t("onboarding:network.localPending"),
            )),
            warning: false,
            on_press: |a| {
                let f = &mut a.onboarding.form;
                f.ai_mode = AiMode::Local;
                f.ai_kind = AiKind::Local;
                m::set_text(&mut f.ai_name, m::DEFAULT_AI_PROVIDER_NAME);
                m::set_text(&mut f.ai_url, m::DEFAULT_AI_BASE_URL);
            },
        },
        local_children,
    );
    let remote_children = if remote {
        provider_fields(app, tk, layer)
    } else {
        vec![]
    };
    let remote_card = f::choice(
        tk,
        Choice {
            key: "onb-ai-remote",
            active: remote,
            glyph: Some(&I::CLOUD),
            title: t("onboarding:ai.remote.title"),
            description: t("onboarding:ai.remote.description"),
            detail: None,
            network: Some(network_note(
                remote_url,
                t("onboarding:network.remotePending"),
            )),
            warning: true,
            on_press: |a| {
                let f = &mut a.onboarding.form;
                f.ai_mode = AiMode::Remote;
                if f.ai_kind == AiKind::Local {
                    f.ai_kind = AiKind::Remote;
                }
                if f.ai_name.text == m::DEFAULT_AI_PROVIDER_NAME {
                    m::set_text(&mut f.ai_name, "");
                    m::set_text(&mut f.ai_url, "");
                }
            },
        },
        remote_children,
    );
    let off_card = f::choice(
        tk,
        Choice {
            key: "onb-ai-off",
            active: form.ai_mode == AiMode::Disabled,
            glyph: Some(&I::POWER),
            title: t("onboarding:ai.disabled.title"),
            description: t("onboarding:ai.disabled.description"),
            detail: None,
            network: Some(m::no_connections()),
            warning: false,
            on_press: |a| a.onboarding.form.ai_mode = AiMode::Disabled,
        },
        vec![],
    );
    col()
        .w_full()
        .gap(tk.r(0.75))
        .child(local_card)
        .child(remote_card)
        .child(off_card)
}

/// "Security".
fn security_step(app: &App, tk: &Tk) -> El {
    let ui = &app.onboarding;
    let form = &ui.form;
    let encrypted = form.db_mode == DbMode::Sqlcipher;
    let issue = if encrypted {
        m::passphrase_issue(&form.passphrase.text, &form.confirm.text)
    } else {
        None
    };
    // Only the not-yet-strong ratings speak.
    let strength_note = match if encrypted {
        m::passphrase_strength(&form.passphrase.text)
    } else {
        None
    } {
        Some(Strength::Weak) => Some(t("onboarding:security.strength.weak")),
        Some(Strength::Fair) => Some(t("onboarding:security.strength.fair")),
        _ => None,
    };
    let mut enc_children = vec![];
    if encrypted {
        let mut p = Text::new(
            "database-passphrase",
            t("onboarding:security.passphrase"),
            &form.passphrase,
            t("onboarding:security.passphrasePlaceholder"),
            |a| &mut a.onboarding.form.passphrase,
        );
        p.secure = true;
        p.revealed = ui.reveal.contains("database-passphrase");
        p.description = strength_note;
        enc_children.push(p.build(tk));
        let mut c = Text::new(
            "database-passphrase-confirm",
            t("onboarding:security.confirmPassphrase"),
            &form.confirm,
            t("onboarding:security.confirmPassphrasePlaceholder"),
            |a| &mut a.onboarding.form.confirm,
        );
        c.secure = true;
        c.revealed = ui.reveal.contains("database-passphrase-confirm");
        c.hint = issue.map(|i| i.hint());
        c.touched = touched(app, "database-passphrase-confirm");
        enc_children.push(c.build(tk));
        enc_children.push(f::note(
            tk,
            &I::KEY_ROUND,
            &t("onboarding:security.passphraseNote"),
        ));
        enc_children.push(f::check_row(
            tk,
            "recovery-ack",
            form.recovery_ack,
            &t("onboarding:security.recoveryAck"),
            f::check_desc(tk, &t("onboarding:security.recoveryAckDescription")),
            |a| a.onboarding.form.recovery_ack = !a.onboarding.form.recovery_ack,
        ));
        // Touch ID is offered only where the Keychain bridge exists.
        if crate::settings::security::touch_id_supported() {
            enc_children.push(f::check_row(
                tk,
                "enable-touch-id",
                form.enable_touch_id,
                &t("onboarding:security.touchId"),
                f::check_desc(tk, &t("onboarding:security.touchIdDescription")),
                |a| a.onboarding.form.enable_touch_id = !a.onboarding.form.enable_touch_id,
            ));
        }
    }
    let enc_card = f::choice(
        tk,
        Choice {
            key: "onb-db-encrypted",
            active: encrypted,
            glyph: Some(&I::LOCK_KEYHOLE),
            title: t("onboarding:security.encrypt.title"),
            description: t("onboarding:security.encrypt.description"),
            detail: None,
            network: None,
            warning: false,
            on_press: |a| a.onboarding.form.db_mode = DbMode::Sqlcipher,
        },
        enc_children,
    );
    let plain_children = if form.db_mode == DbMode::Plaintext {
        vec![f::check_row(
            tk,
            "plaintext-ack",
            form.plaintext_ack,
            &t("onboarding:security.plaintextAck"),
            f::check_desc(tk, &t("onboarding:security.plaintextAckDescription")),
            |a| a.onboarding.form.plaintext_ack = !a.onboarding.form.plaintext_ack,
        )]
    } else {
        vec![]
    };
    let plain_card = f::choice(
        tk,
        Choice {
            key: "onb-db-plaintext",
            active: form.db_mode == DbMode::Plaintext,
            glyph: Some(&I::SHIELD_OFF),
            title: t("onboarding:security.plaintext.title"),
            description: t("onboarding:security.plaintext.description"),
            detail: None,
            network: None,
            warning: true,
            on_press: |a| a.onboarding.form.db_mode = DbMode::Plaintext,
        },
        plain_children,
    );
    let mut parts = vec![
        col()
            .w_full()
            .gap(tk.r(0.75))
            .child(enc_card)
            .child(plain_card),
    ];
    if encrypted {
        let i = ink(tk);
        let open = ui.migrate_open;
        let summary = row()
            .key("onb-existing-credentials")
            .gap(tk.r(0.375))
            .center_y()
            .role("button")
            .expanded(open)
            .focusable(true)
            .label(t("onboarding:security.existingCredentials"))
            .on_press(|a: &mut App| a.onboarding.migrate_open = !a.onboarding.migrate_open)
            .child(ic(
                if open {
                    &I::CHEVRON_DOWN
                } else {
                    &I::CHEVRON_RIGHT
                },
                tk.r(0.75),
                i.ink3,
            ))
            .child(
                text(t("onboarding:security.existingCredentials"))
                    .font(tk.tsm(500.0))
                    .color(i.ink2),
            );
        let mut d = col()
            .w_full()
            .pt(tk.r(1.25))
            .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
            .border_layout()
            .child(summary);
        if open {
            let mc = f::check_row(
                tk,
                "migrate-credentials",
                form.migrate_credentials,
                &t("onboarding:security.migrateCredentials"),
                f::check_desc(tk, &t("onboarding:security.migrateCredentialsDescription")),
                |a| a.onboarding.form.migrate_credentials = !a.onboarding.form.migrate_credentials,
            );
            d = d.child(col().pt(tk.r(1.0)).child(mc));
        }
        parts.push(d);
    }
    parts.push(actions(app, tk, None, None));
    f::step_page(
        tk,
        &t("onboarding:security.title"),
        Some(t("onboarding:security.lead")),
        form_body(parts, tk),
    )
}

// ---- Review ------------------------------------------------------------------------------------

struct ReviewRow {
    area: &'static str,
    network_group: bool,
    value: String,
    note: Option<String>,
    step: Option<usize>,
    details: Vec<(String, String)>,
    network: Option<m::Network>,
}

fn quiet_network(reach: Reach, when: &str) -> m::Network {
    m::Network {
        reach,
        label: f::reach_label(reach, when),
        hosts: vec![],
        pending: None,
    }
}

fn review_rows(app: &App) -> Vec<ReviewRow> {
    let form = &app.onboarding.form;
    let defaults = app.onboarding.defaults.clone().unwrap_or_default();
    let offline = form.backend_mode == Some(BackendMode::Skip);
    let updates = !offline && form.update_checks;
    let tax_note = if form.tax_country == "at" {
        (!(form.fiat == "EUR" && form.gains == m::default_gains("at")))
            .then(|| t("onboarding:review.tax.noteAtCustom"))
    } else if crate::reports::books::model::is_preview(&form.tax_country) {
        Some(t("common:taxCountry.previewNotice"))
    } else if m::uses_long_term_days(&form.tax_country) {
        Some(tr(
            "onboarding:review.tax.noteGeneric",
            &[("days", m::or(&form.long_term_days.text, "365"))],
        ))
    } else {
        None
    };
    let custom_endpoint = form.custom_endpoint();
    let fallback = |s: &str, key: &str| {
        if s.trim().is_empty() {
            t(key)
        } else {
            s.trim().to_owned()
        }
    };
    let sync_value = match form.backend_mode {
        Some(BackendMode::Skip) => t("onboarding:review.sync.valueManual"),
        Some(BackendMode::Default) => t("onboarding:review.sync.valueDefault"),
        _ => {
            let kind = form.backend_kind.label();
            let detail = if custom_endpoint.is_empty() {
                kind.to_owned()
            } else {
                format!("{kind}, {custom_endpoint}")
            };
            tr(
                "onboarding:review.sync.valueCustom",
                &[
                    (
                        "name",
                        &fallback(
                            &form.backend_name.text,
                            "onboarding:review.sync.customFallbackName",
                        ),
                    ),
                    ("detail", &detail),
                ],
            )
        }
    };
    let sync_note = match form.backend_mode {
        Some(BackendMode::Default) => Some(t(if defaults.is_empty() {
            "onboarding:review.sync.noteDefaultLoading"
        } else {
            "onboarding:connections.default.tradeoff"
        })),
        Some(BackendMode::Custom) if form.use_proxy => {
            Some(t("onboarding:review.sync.noteCustomProxy"))
        }
        _ => None,
    };
    let sync_details = if form.backend_mode == Some(BackendMode::Default) {
        defaults
            .iter()
            .filter(|b| form.include_liquid || !b.liquid_opt_in)
            .map(|b| {
                (
                    format!("{} ({})", b.name, m::kind_label(&b.kind)),
                    b.url.clone(),
                )
            })
            .collect()
    } else {
        vec![]
    };
    let sync_network = if offline {
        m::no_connections()
    } else {
        quiet_network(
            if form.backend_mode == Some(BackendMode::Custom) {
                m::endpoint_reach(&custom_endpoint)
            } else {
                Reach::Internet
            },
            "sync",
        )
    };
    let ai_off = offline || form.ai_mode == AiMode::Disabled;
    let ai_url = form.ai_url.text.trim().to_owned();
    let ai_value = if ai_off {
        t("onboarding:review.ai.valueDisabled")
    } else {
        tr(
            "onboarding:review.ai.value",
            &[
                ("kind", form.ai_kind.label()),
                (
                    "name",
                    &fallback(&form.ai_name.text, "onboarding:review.ai.fallbackName"),
                ),
            ],
        )
    };
    let ai_network = if ai_off {
        m::no_connections()
    } else {
        quiet_network(
            if ai_url.is_empty() {
                Reach::Internet
            } else {
                m::endpoint_reach(&ai_url)
            },
            "assistant",
        )
    };
    let enc = form.db_mode == DbMode::Sqlcipher;
    let books = tr(
        "onboarding:review.books.value",
        &[
            ("workspace", m::or(form.workspace.text.trim(), "My Books")),
            ("profile", m::or(form.profile.trim(), "Private")),
        ],
    );
    let tax = tr(
        "onboarding:review.tax.value",
        &[
            ("region", &m::region_label(&form.tax_country)),
            ("currency", &form.fiat),
            ("method", &m::method_label(&form.gains)),
        ],
    );
    let storage_note = if enc {
        tr(
            "onboarding:review.storage.noteEncrypted",
            &[(
                "touchId",
                &t(if form.enable_touch_id {
                    "onboarding:review.storage.touchIdEnabled"
                } else {
                    "onboarding:review.storage.touchIdNotEnabled"
                }),
            )],
        )
    } else {
        t("onboarding:review.storage.notePlaintext")
    };
    vec![
        ReviewRow {
            area: "books",
            network_group: false,
            value: books,
            note: None,
            step: Some(0),
            details: vec![],
            network: None,
        },
        ReviewRow {
            area: "tax",
            network_group: false,
            value: tax,
            note: tax_note,
            step: Some(0),
            details: vec![],
            network: None,
        },
        ReviewRow {
            area: "sync",
            network_group: true,
            value: sync_value,
            note: sync_note,
            step: Some(1),
            details: sync_details,
            network: Some(sync_network),
        },
        ReviewRow {
            area: "ai",
            network_group: true,
            value: ai_value,
            note: (!offline && form.ai_mode == AiMode::Remote)
                .then(|| t("onboarding:review.ai.noteRemote")),
            step: Some(2),
            details: if ai_off || ai_url.is_empty() {
                vec![]
            } else {
                vec![(t("onboarding:ai.baseUrl"), ai_url.clone())]
            },
            network: Some(ai_network),
        },
        ReviewRow {
            area: "updates",
            network_group: true,
            value: t(if updates {
                "onboarding:review.updates.valueEnabled"
            } else {
                "onboarding:review.updates.valueDisabled"
            }),
            note: updates.then(|| t("onboarding:review.updates.noteEnabled")),
            step: Some(1),
            details: if updates {
                vec![("GitHub".into(), "api.github.com".into())]
            } else {
                vec![]
            },
            network: Some(if updates {
                quiet_network(Reach::Internet, "updates")
            } else {
                m::no_connections()
            }),
        },
        ReviewRow {
            area: "others",
            network_group: true,
            value: t("onboarding:review.others.value"),
            note: None,
            step: None,
            details: vec![],
            network: Some(m::Network {
                reach: Reach::None,
                label: t("onboarding:network.offUntilSetUp"),
                hosts: vec![],
                pending: None,
            }),
        },
        ReviewRow {
            area: "storage",
            network_group: false,
            value: t(if enc {
                "onboarding:review.storage.valueEncrypted"
            } else {
                "onboarding:review.storage.valuePlaintext"
            }),
            note: Some(storage_note),
            step: Some(3),
            details: vec![],
            network: None,
        },
        ReviewRow {
            area: "secrets",
            network_group: false,
            value: t(if enc && form.migrate_credentials {
                "onboarding:review.secrets.valueEncrypted"
            } else {
                "onboarding:review.secrets.valueNone"
            }),
            note: (enc && form.migrate_credentials)
                .then(|| t("onboarding:review.secrets.noteEncrypted")),
            step: Some(3),
            details: vec![],
            network: None,
        },
    ]
}

fn review_step(app: &App, tk: &Tk) -> El {
    let i = ink(tk);
    let rows = review_rows(app);
    let (w, _) = win(app);
    let mut parts: Vec<El> = vec![];
    // What reaches the network first.
    for (group, network) in [("network", true), ("local", false)] {
        let mut items = vec![];
        for r in rows.iter().filter(|r| r.network_group == network) {
            let area = t(&format!("onboarding:review.area.{}", r.area));
            let mut d = col().w_full().gap(tk.r(0.25));
            if let Some(n) = &r.network {
                d = d.child(f::network_status(tk, n.reach, &n.label));
            }
            d = d.child(
                text(r.value.clone())
                    .font(tk.tsm(400.0))
                    .color(i.ink2)
                    .wrap(),
            );
            if !r.details.is_empty() {
                let mut list = col().w_full().gap(tk.r(0.125));
                for (l, v) in &r.details {
                    let lt = text(l.clone())
                        .font(tk.txs(400.0).lh(tk.r(1.0)))
                        .color(i.ink2);
                    let vt = text(v.clone())
                        .font(tk.t2xs(400.0).mono().lh(tk.r(1.25)))
                        .color(i.ink2)
                        .wrap();
                    list = list.child(if w >= SM {
                        row()
                            .w_full()
                            .gap(tk.r(0.75))
                            .child(col().w(tk.r(10.0)).child(lt))
                            .child(col().grow(1.0).min_w(0.0).child(vt))
                    } else {
                        col().w_full().child(lt).child(vt)
                    });
                }
                d = d.child(list);
            }
            if let Some(n) = &r.note {
                d = d.child(
                    text(n.clone())
                        .font(tk.txs(400.0).lh(tk.r(1.125)))
                        .color(i.ink3)
                        .wrap(),
                );
            }
            let control = r.step.map(|s| {
                button_surface(
                    tk,
                    row()
                        .key(format!("onb-change-{}", r.area))
                        .h(tk.r(2.0))
                        .px(tk.r(0.75))
                        .center(),
                    Variant::Ghost,
                )
                .role("button")
                .label(tr("onboarding:review.changeArea", &[("area", &area)]))
                .focusable(true)
                .on_press(move |a: &mut App| super::jump(a, s))
                .child(
                    text(t("onboarding:review.change"))
                        .font(tk.tsm(500.0))
                        .color(i.ink3),
                )
            });
            items.push(f::srow(
                tk,
                f::row_title(tk, &area),
                Some(d),
                None,
                control,
                false,
            ));
        }
        let title = t(&format!("onboarding:review.group.{group}.title"));
        parts.push(
            col()
                .w_full()
                .gap(tk.r(0.625))
                .role("group")
                .label(title.clone())
                .child(
                    text(title)
                        .font(tk.tsm(500.0))
                        .color(i.ink)
                        .px(tk.r(0.25))
                        .role("heading")
                        .level(2),
                )
                .child(f::group(tk, items)),
        );
    }
    parts.push(actions(
        app,
        tk,
        Some(t("onboarding:review.createLocalBooks")),
        Some(t("onboarding:review.creatingBooks")),
    ));
    f::step_page(
        tk,
        &t("onboarding:review.title"),
        None,
        form_body(parts, tk),
    )
}

// ---- Opened folder (`ImportProjectPanel`) -------------------------------------------------------

fn import_panel(app: &App, tk: &Tk, panel_h: f32) -> El {
    let i = ink(tk);
    let ui = &app.onboarding;
    let Some(sel) = ui.import_selection.as_ref() else {
        return el();
    };
    let snapshot = ui.import_snapshot.as_ref();
    let needs_pass = sel.encrypted && snapshot.is_none();
    let count: usize = snapshot
        .and_then(|s| s["workspaces"].as_array())
        .map(|w| {
            w.iter()
                .map(|w| w["profiles"].as_array().map_or(0, Vec::len))
                .sum()
        })
        .unwrap_or(0);
    let pill = |s: String| {
        row()
            .px(tk.r(0.625))
            .py(tk.r(0.25))
            .radius(tk.r(9.0))
            .fill(i.paper2)
            .border(1.0, tk.border)
            .child(text(s).font(tk.txs(400.0)).color(i.ink2))
    };
    let mut chips = row()
        .flex_wrap()
        .gaps(tk.r(0.5), tk.r(0.5))
        .child(pill(t(if sel.encrypted {
            "onboarding:import.encrypted"
        } else {
            "onboarding:import.plaintext"
        })));
    if count > 0 {
        chips = chips.child(pill(tc("onboarding:import.bookCount", count, &[])));
    }
    let mono = |s: &str| {
        text(s.to_owned())
            .font(tk.txs(400.0).mono().lh(tk.r(1.0)))
            .color(i.ink3)
            .wrap()
    };
    let surface = col()
        .w_full()
        .gap(tk.r(0.75))
        .padding(tk.r(1.0))
        .radius(tk.r(0.75))
        .border(1.0, tk.border.alpha(tk.border.a * 0.7))
        .fill(tk.card)
        .shadow(
            1.0,
            2.0,
            Color::BLACK.alpha(if tk.dark { 0.35 } else { 0.035 }),
        )
        .child(
            row()
                .w_full()
                .gap(tk.r(0.75))
                .items_start()
                .child(
                    row()
                        .size(tk.r(2.25), tk.r(2.25))
                        .center()
                        .radius(tk.r(0.5))
                        .fill(i.paper2)
                        .border(1.0, tk.border)
                        .child(ic(&I::DATABASE, tk.r(1.0), i.ink)),
                )
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .child(
                            text(t("onboarding:import.selectedBooks"))
                                .font(tk.tsm(500.0))
                                .color(i.ink),
                        )
                        .child(mono(&sel.state_root).mt(tk.r(0.25))),
                ),
        )
        .child(chips)
        .child(
            col()
                .w_full()
                .gap(tk.r(0.25))
                .pt(tk.r(0.75))
                .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
                .border_layout()
                .child(mono(&sel.data_root))
                .child(mono(&sel.database)),
        );
    let mut parts = vec![surface];
    if needs_pass {
        let busy = ui.loading_profiles;
        let mut k = form_kit(tk);
        k.c.text_faint = tk.muted_fg;
        let revealed = ui.import_reveal;
        let reveal = row()
            .key("import-passphrase-reveal")
            .size(tk.r(2.0), tk.r(2.0))
            .center()
            .radius(tk.r(0.375))
            .hover(tk.accent)
            .role("button")
            .label(t("common:passphraseReveal.show"))
            .help(t(if revealed {
                "common:passphraseReveal.hide"
            } else {
                "common:passphraseReveal.show"
            }))
            .checked(revealed)
            .focusable(!busy)
            .disabled(busy)
            .on_press(|a: &mut App| a.onboarding.import_reveal = !a.onboarding.import_reveal)
            .child(ic(
                if revealed { &I::EYE_OFF } else { &I::EYE },
                tk.r(1.0),
                tk.muted_fg,
            ));
        let input = TextInput::new(&ui.import_pass)
            .label(t("onboarding:import.passphraseLabel"))
            .height(tk.r(2.25))
            .width(None)
            .text_style(tk.tsm(400.0))
            .secure()
            .revealed(revealed)
            .disabled(busy)
            .trailing(row().pr(tk.r(0.125)).child(reveal))
            .on_edit(|a: &mut App| &mut a.onboarding.import_pass)
            .on_change(|a: &mut App, _tx: &mut rune::Tx| a.onboarding.open_error = None)
            .on_submit(|a: &mut App, _tx: &mut rune::Tx| super::submit_import_pass(a))
            .build(&k)
            .key("import-passphrase");
        let can = !ui.import_pass.text.is_empty() && !busy;
        let mut unlock = f::button(
            tk,
            "onb-import-unlock",
            Variant::Default,
            Size::Default,
            Some(&I::KEY_ROUND),
            &t("onboarding:import.unlock"),
            can,
        )
        .w_full();
        if can {
            unlock = unlock.on_press(super::submit_import_pass);
        }
        let field_col = col()
            .w_full()
            .gap(tk.r(0.5))
            .child(f::label(tk, &t("onboarding:import.passphraseLabel")))
            .child(field(tk, input).border(1.0, i.line));
        parts.push(col().w_full().gap(tk.r(1.0)).child(field_col).child(unlock));
    } else {
        parts.push(profile_list(app, tk));
    }
    if let Some(e) = ui.open_error.as_ref().or(ui.import_error.as_ref()) {
        parts.push(
            row()
                .w_full()
                .px(tk.r(0.75))
                .py(tk.r(0.5))
                .radius(tk.r(0.5))
                .border(1.0, tk.destructive.alpha(0.25))
                .fill(tk.destructive.alpha(0.1))
                .role("alert")
                .label(e.clone())
                .child(
                    text(e.clone())
                        .font(tk.tsm(400.0))
                        .color(tk.destructive)
                        .wrap(),
                ),
        );
    }
    let back_enabled = !ui.loading_profiles && ui.opening_profile.is_none();
    let back_label = t("onboarding:import.backToSetup");
    let mut back = button_surface(
        tk,
        row()
            .key("onb-import-back")
            .h(tk.r(2.25))
            .px(tk.r(0.75))
            .gap(tk.r(0.5))
            .center(),
        Variant::Ghost,
    )
    .role("button")
    .label(back_label.clone())
    .focusable(back_enabled)
    .disabled(!back_enabled)
    .child(ic(&I::ARROW_LEFT, tk.r(1.0), tk.fg))
    .child(text(back_label).font(tk.tsm(500.0)).color(tk.fg));
    back = if back_enabled {
        back.on_press(super::cancel_import)
    } else {
        back.opacity(0.5)
    };
    parts.push(row().child(back));
    let mut content = col().w_full().gap(tk.r(1.25));
    for p in parts {
        content = content.child(p);
    }
    let page = f::step_page(
        tk,
        &t("onboarding:import.title"),
        Some(t("onboarding:import.rightBody")),
        content,
    );
    let inner = col()
        .key("onb-import")
        .w_full()
        .max_w(tk.r(36.0))
        .min_h(panel_h)
        .justify_center()
        .gap(tk.r(2.0))
        .px(tk.r(1.5))
        .py(tk.r(3.0))
        .child(crate::shell::wordmark::wordmark(2.0, tk.fg, false))
        .child(page);
    col().w_full().items_center().child(inner)
}

fn profile_list(app: &App, tk: &Tk) -> El {
    let i = ink(tk);
    let ui = &app.onboarding;
    let surface = || {
        col()
            .w_full()
            .radius(tk.r(0.75))
            .border(1.0, tk.border.alpha(tk.border.a * 0.7))
            .fill(tk.card)
    };
    if ui.loading_profiles {
        return surface().px(tk.r(1.0)).py(tk.r(2.0)).items_center().child(
            text(t("onboarding:import.loading"))
                .font(tk.tsm(500.0))
                .color(tk.fg),
        );
    }
    let workspaces: Vec<Value> = ui
        .import_snapshot
        .as_ref()
        .and_then(|s| s["workspaces"].as_array())
        .cloned()
        .unwrap_or_default();
    if workspaces.is_empty() {
        return surface()
            .padding(tk.r(1.0))
            .gap(tk.r(0.75))
            .child(
                text(t("onboarding:import.empty"))
                    .font(tk.tsm(400.0))
                    .color(tk.fg)
                    .wrap(),
            )
            .child(
                row().child(
                    f::button(
                        tk,
                        "onb-import-refresh",
                        Variant::Outline,
                        Size::Sm,
                        None,
                        &t("common:actions.refresh"),
                        true,
                    )
                    .on_press(super::refresh_import_profiles),
                ),
            );
    }
    let mut list = col().key("onb-import-books").w_full().gap(tk.r(1.0));
    for w in workspaces {
        let meta = format!(
            "{} · {}",
            w["currency"].as_str().unwrap_or_default(),
            w["jurisdiction"].as_str().unwrap_or_default()
        )
        .to_uppercase();
        let mut sec = col().w_full().gap(tk.r(0.5)).child(
            row()
                .flex_wrap()
                .gaps(tk.r(0.25), tk.r(0.5))
                .px(tk.r(0.25))
                .center_y()
                .child(
                    text(w["name"].as_str().unwrap_or_default().to_owned())
                        .font(tk.tsm(600.0))
                        .color(i.ink)
                        .role("heading")
                        .level(4),
                )
                .child(
                    text(meta)
                        .font(tk.t2xs(400.0).mono().tracking(tk.r(0.694) * 0.1))
                        .color(i.ink3),
                ),
        );
        for p in w["profiles"].as_array().cloned().unwrap_or_default() {
            let id = p["id"].as_str().unwrap_or_default().to_owned();
            let opening = ui.opening_profile.as_deref() == Some(id.as_str());
            let busy = ui.opening_profile.is_some();
            let active = p["active"] == true;
            let name = p["name"].as_str().unwrap_or_default().to_owned();
            let mut head = row().gap(tk.r(0.5)).center_y().child(
                text(name.clone())
                    .font(tk.tsm(500.0))
                    .color(i.ink)
                    .truncate(),
            );
            if active {
                head = head.child(
                    ic(&I::CIRCLE_CHECK, tk.r(1.0), tk.kb_accent)
                        .role("img")
                        .label(t("onboarding:import.active")),
                );
            }
            let n = |k: &str| {
                p[k].as_f64()
                    .map(|v| format!("{v}"))
                    .unwrap_or_else(|| "0".into())
            };
            let counts = tr(
                "onboarding:import.accountsWallets",
                &[("accounts", &n("accounts")), ("wallets", &n("wallets"))],
            )
            .to_uppercase();
            let left = col()
                .grow(1.0)
                .min_w(0.0)
                .gap(tk.r(0.5))
                .child(head)
                .child(
                    text(p["taxPolicy"].as_str().unwrap_or_default().to_owned())
                        .font(tk.txs(400.0).lh(tk.r(1.25)))
                        .color(i.ink2)
                        .wrap(),
                )
                .child(
                    text(counts)
                        .font(tk.t2xs(400.0).mono().tracking(tk.r(0.694) * 0.08))
                        .color(i.ink3),
                );
            let action = row()
                .gap(tk.r(0.25))
                .center_y()
                .child(
                    text(t(if opening {
                        "onboarding:import.opening"
                    } else {
                        "common:actions.open"
                    }))
                    .font(tk.tsm(500.0))
                    .color(i.ink),
                )
                .child(ic(&I::ARROW_RIGHT, tk.r(1.0), i.ink));
            let (wv, pv) = (w.clone(), p.clone());
            let mut card = row()
                .key(format!("onb-import-book-{id}"))
                .w_full()
                .min_h(tk.r(6.0))
                .padding(tk.r(1.0))
                .gap(tk.r(0.75))
                .items_start()
                .justify_between()
                .radius(tk.r(0.75))
                .border(1.0, if active { i.ink } else { tk.border })
                .fill(if active { tk.card } else { tk.card.alpha(0.7) })
                .hover(tk.card)
                .role("button")
                .label(name)
                .focusable(!busy)
                .disabled(busy)
                .child(left)
                .child(action);
            if busy {
                card = card.opacity(0.7);
            } else {
                card = card
                    .on_press(move |a: &mut App| super::open_profile(a, wv.clone(), pv.clone()));
            }
            sec = sec.child(card);
        }
        list = list.child(sec);
    }
    list
}
