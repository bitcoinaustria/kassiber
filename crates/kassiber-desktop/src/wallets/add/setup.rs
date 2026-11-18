//! The setup step (`renderSetupStep`): the source summary rail and the form
//! (`renderSetupFields` for every setup kind, sync progress, the error line and
//! `renderImportSummary`).

use rune::{Color, Cx, El, Span, TextEdit, TextStyle, Tx, col, el, rich, row, text, text_input};
use serde_json::Value;

use super::btcpay::form::{
    Variant, badge, badge_secondary, button, error_line, input, notice, select_menu,
    select_trigger, setup_field, xs,
};
use super::btcpay::model::Tone;
use super::libs::{address_list, backend_rows, wallet_material};
use super::meta::{SetupKind, is_exchange_evidence, meta};
use super::submit::{self, backend_option_label, name_of};
use super::{CoreTest, Coverage, core_default_url, ledger_preview, scanner, selected, st, stm};
use crate::app::App;
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::ic;

type Access = fn(&mut App) -> &mut TextEdit;

fn h(s: &str) -> Color {
    rune::color::hex(s).expect("hex")
}

fn err(app: &App, key: &str) -> Option<String> {
    st(app).errors.get(key).cloned()
}

/// Clears a field's error when it changes (`updateForm`).
fn clear(key: &'static str) -> impl Fn(&mut App) + 'static {
    move |a: &mut App| {
        stm(a).errors.remove(key);
    }
}

/// A text input bound to a form field.
#[allow(clippy::too_many_arguments)]
fn field_input(
    app: &App,
    tk: &Tk,
    key: &'static str,
    err_key: &'static str,
    name: &str,
    edit: &TextEdit,
    access: Access,
    placeholder: Option<&str>,
    secure: bool,
) -> El {
    let _ = app;
    input(
        tk,
        key,
        edit,
        access,
        name,
        placeholder,
        secure,
        clear(err_key),
    )
}

/// shadcn `Textarea` (`min-h-N rounded-md border border-input px-3 py-2`, `font-mono text-xs`).
#[allow(clippy::too_many_arguments)]
fn textarea(
    app: &App,
    tk: &Tk,
    key: &'static str,
    err_key: &'static str,
    name: &str,
    access: Access,
    edit: &TextEdit,
    min_h: f32,
    placeholder: Option<&str>,
    on_change: Option<fn(&mut App)>,
) -> El {
    let _ = app;
    let ck = clear(err_key);
    let mut f = text_input(edit, tk.txs(400.0).mono())
        .key(key.to_owned())
        .caret(tk.fg, tk.primary.alpha(0.3))
        .label(name.to_owned())
        .on_edit(access)
        .on_input_change(move |a: &mut App, _tx: &mut Tx| {
            ck(a);
            if let Some(c) = on_change {
                c(a);
            }
        })
        .w_full();
    if let Some(p) = placeholder {
        f = f.placeholder(p.to_owned(), tk.muted_fg);
    }
    let fill = if tk.dark {
        tk.input.alpha(tk.input.a * 0.3)
    } else {
        Color::TRANSPARENT
    };
    row()
        // The box the validation bubble hangs under (`<key>-box`).
        .key(format!("{key}-box"))
        .w_full()
        .min_w(0.0)
        .min_h(min_h)
        .px(tk.r(0.75))
        .py(tk.r(0.5))
        .items_start()
        .radius(tk.r(0.375))
        .border(1.0, if edit.focused { tk.ring } else { tk.input })
        .border_layout()
        .fill(fill)
        .box_shadow(rune::Shadow {
            y: 1.0,
            blur: 2.0,
            color: Color::BLACK.alpha(0.05),
            ..rune::Shadow::default()
        })
        .child(f)
}

/// shadcn `Checkbox` box.
fn checkbox_box(tk: &Tk, on: bool) -> El {
    let b = row().size(tk.r(1.0), tk.r(1.0)).center().radius(4.0);
    if on {
        b.fill(tk.primary)
            .child(ic(&I::CHECK, tk.r(0.875), tk.primary_fg))
    } else {
        b.border(1.0, tk.input)
            .border_layout()
            .fill(if tk.dark {
                tk.input.alpha(tk.input.a * 0.3)
            } else {
                Color::TRANSPARENT
            })
            .box_shadow(rune::Shadow {
                y: 1.0,
                blur: 2.0,
                color: Color::BLACK.alpha(0.05),
                ..rune::Shadow::default()
            })
    }
}

/// `<label className="flex items-center|start gap-2 text-sm"><Checkbox/>{text}</label>`.
fn checkbox(
    tk: &Tk,
    key: &str,
    on: bool,
    label: &str,
    start: bool,
    toggle: impl Fn(&mut App) + 'static,
) -> El {
    let mut r = row()
        .key(key.to_owned())
        .w_full()
        .gap(tk.r(0.5))
        .role("checkbox")
        .checked(on)
        .label(label.to_owned())
        .focusable(true)
        .on_press(toggle);
    r = if start { r.items_start() } else { r.center_y() };
    r.child(checkbox_box(tk, on).mt(if start { tk.r(0.125) } else { 0.0 }))
        .child(
            text(label.to_owned())
                .font(tk.tsm(400.0))
                .color(tk.fg)
                .wrap()
                .grow(1.0)
                .min_w(0.0),
        )
}

/// `renderSyncAfterCreate`.
fn sync_after(app: &App, tk: &Tk, label: &str) -> El {
    let on = st(app).form.sync_after;
    checkbox(tk, "add-sync-after", on, label, false, |a: &mut App| {
        let f = &mut stm(a).form;
        f.sync_after = !f.sync_after;
        stm(a).errors.remove("syncAfterCreate");
    })
}

/// `renderConnectionLabelField`.
fn label_field(app: &App, tk: &Tk) -> El {
    let name = t("connections:add.field.connectionLabel");
    setup_field(
        tk,
        &name,
        field_input(
            app,
            tk,
            "connection-label",
            "label",
            &name,
            &st(app).form.label,
            |a| &mut stm(a).form.label,
            None,
            false,
        ),
        err(app, "label"),
        None,
    )
}

/// A native `<select>`: the trigger here, its option list mounted into `layer` while open.
#[allow(clippy::too_many_arguments)]
fn native_select(
    app: &App,
    tk: &Tk,
    layer: &mut Vec<El>,
    key: &'static str,
    name: &str,
    options: Vec<(String, String, bool)>,
    current: &str,
    enabled: bool,
    choose: impl Fn(&mut App, &str) + Clone + 'static,
) -> El {
    let shown = options
        .iter()
        .find(|(v, _, _)| v == current)
        .or(options.first())
        .map(|(_, l, _)| l.clone())
        .unwrap_or_default();
    let open = st(app).select.as_deref() == Some(key);
    let mut trig = select_trigger(tk, key, name, &shown, enabled, false).expanded(open);
    if enabled {
        trig = crate::ui::popup::trigger(
            trig,
            crate::ui::popup::Kind::Listbox,
            open,
            move |a: &mut App| {
                let s = stm(a);
                s.select = if s.select.as_deref() == Some(key) {
                    None
                } else {
                    Some(key.to_owned())
                };
            },
        );
    }
    if open {
        let width = 420.0;
        let opts = options
            .into_iter()
            .map(|(v, l, en)| (v, l, en, None))
            .collect();
        select_menu(
            tk,
            layer,
            key,
            width,
            opts,
            current,
            |a: &mut App| stm(a).select = None,
            choose,
        );
    }
    trig
}

/// `renderBackendSelect`.
fn backend_select(
    app: &App,
    tk: &Tk,
    layer: &mut Vec<El>,
    label: &str,
    options: Vec<Value>,
    helper: Option<String>,
) -> El {
    let control = if !options.is_empty() {
        let mut opts = vec![(
            String::new(),
            t("connections:add.field.selectBackend"),
            false,
        )];
        for b in &options {
            opts.push((name_of(b).to_owned(), backend_option_label(b), true));
        }
        native_select(
            app,
            tk,
            layer,
            "connection-backend",
            label,
            opts,
            &st(app).form.backend,
            true,
            |a: &mut App, v: &str| {
                stm(a).form.backend = v.to_owned();
                stm(a).errors.remove("backend");
                submit::coverage_inputs_changed(a);
            },
        )
    } else {
        col()
            .w_full()
            .padding(tk.r(0.75))
            .gap(tk.r(0.5))
            .radius(tk.r(0.375))
            .border(1.0, tk.border)
            .border_layout()
            .fill(tk.background)
            .child(
                text(t("connections:add.field.noBackendConfigured"))
                    .font(tk.tsm(400.0))
                    .color(tk.muted_fg)
                    .wrap(),
            )
            .child(
                row().child(
                    button(
                        tk,
                        "add-open-backend-settings",
                        Variant::Outline,
                        true,
                        None,
                        &t("connections:add.field.openBackendSettings"),
                        true,
                    )
                    .on_press(submit::open_backend_settings),
                ),
            )
    };
    setup_field(tk, label, control, err(app, "backend"), helper)
}

/// A `Browse…` button that fills `field` from a native Open dialog.
fn browse(
    tk: &Tk,
    key: &str,
    label: String,
    title: String,
    filters: Vec<(String, Vec<&'static str>)>,
    on_pick: fn(&mut App, String),
) -> El {
    button(tk, key, Variant::Outline, false, None, &label, true).on_press_tx(
        move |_a: &mut App, tx: &mut Tx| {
            let mut opts = rune::FileDialogOptions {
                title: title.clone(),
                ..Default::default()
            };
            for (n, exts) in &filters {
                opts = opts.filter(n.clone(), exts);
            }
            tx.open_file(
                opts,
                move |a: &mut App, result, _tx: &mut Tx| match result {
                    Ok(Some(p)) => on_pick(a, p.to_string_lossy().into_owned()),
                    Ok(None) => {}
                    Err(e) => a.toast(
                        crate::app::ToastTone::Warning,
                        &t("connections:add.field.filePickerUnavailableTitle"),
                        Some(tr(
                            "connections:add.field.filePickerUnavailableBody",
                            &[("message", &e.0)],
                        )),
                        Some("file-picker"),
                    ),
                },
            );
        },
    )
}

/// `sourceFileFilters`.
fn file_filters(id: &str) -> Vec<(String, Vec<&'static str>)> {
    let m = meta(id);
    let f =
        |k: &str, e: Vec<&'static str>| vec![(t(&format!("connections:add.fileFilter.{k}")), e)];
    match (m.source_format, m.kind()) {
        (Some("phoenix_csv"), _) => f("phoenixCsv", vec!["csv"]),
        (Some("river_csv"), _) => f("riverCsv", vec!["csv"]),
        (Some("bullbitcoin_csv"), _) => f("bullbitcoinCsv", vec!["csv"]),
        (Some("bullbitcoin_wallet_csv"), _) => f("bullbitcoinWalletCsv", vec!["csv"]),
        (Some("coinfinity_csv"), _) => f("coinfinityCsv", vec!["csv"]),
        (Some("pocketbitcoin_csv"), _) => f("pocketbitcoinCsv", vec!["csv"]),
        (Some("21bitcoin_csv"), _) => f("twentyonebitcoinCsv", vec!["csv"]),
        (Some("strike_csv"), _) => f("strikeCsv", vec!["csv"]),
        (Some("bisq_csv"), _) => f("bisqCsv", vec!["csv"]),
        (Some("bitstamp_csv"), _) => f("bitstampCsv", vec!["csv"]),
        (Some("ledgerlive_csv"), _) => f("ledgerliveCsv", vec!["csv"]),
        (Some("cointracking_csv"), _) => f("cointrackingCsv", vec!["csv"]),
        (Some("blockpit_csv"), _) => f("blockpitCsv", vec!["csv"]),
        (Some("binance_supplemental_csv"), _) => f("binanceSupplementalCsv", vec!["csv"]),
        (Some("binance_history_csv"), _) => f("binanceHistoryCsv", vec!["csv"]),
        (Some("bitfinex_csv"), _) => f("bitfinexReports", vec!["zip", "csv"]),
        (Some("btcpay_csv"), _) => f("btcpayCsv", vec!["csv"]),
        (Some("wasabi_bundle"), _) => f("wasabiBundle", vec!["json"]),
        (_, SetupKind::Samourai) => f("samouraiSourceSet", vec!["json"]),
        (Some("generic_ledger"), _) => f("genericLedger", vec!["xlsx", "csv", "tsv"]),
        (_, SetupKind::PriorReport) => f("priorTaxReport", vec!["pdf", "xlsx", "csv"]),
        _ if id == "csv" => f("csvOrJson", vec!["csv", "json"]),
        _ => vec![],
    }
}

/// `fileWalletSourceField`: (label, helper).
fn source_file_field(id: &str) -> (String, Option<String>) {
    let m = meta(id);
    if m.kind() == SetupKind::PriorReport {
        return (
            t("connections:add.priorReport.fileLabel"),
            Some(t("connections:add.priorReport.fileHelper")),
        );
    }
    let k = match m.source_format {
        Some("wasabi_bundle") => "wasabi",
        Some("btcpay_csv") => "btcpay",
        Some("bisq_csv") => "bisq",
        Some("cointracking_csv") => "cointracking",
        Some("blockpit_csv") => "blockpit",
        _ => return (t("connections:add.exportFile.label"), None),
    };
    (
        t(&format!("connections:add.exportFile.{k}Label")),
        Some(t(&format!("connections:add.exportFile.{k}Helper"))),
    )
}

fn b64(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            out.push(if i <= c.len() {
                T[(n >> (18 - 6 * i) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    out
}

fn set_source_file(a: &mut App, path: String) {
    stm(a).form.source_file = TextEdit::new(&path);
    stm(a).errors.remove("sourceFile");
    stm(a).last_import = None;
}

/// The generic ledger's picker reads the file too (`pickFileWithContentsBase64`) and feeds the
/// preview.
pub fn pick_ledger_file(a: &mut App, path: String) {
    set_ledger_file(a, path);
}

fn set_ledger_file(a: &mut App, path: String) {
    set_source_file(a, path.clone());
    match std::fs::read(&path) {
        Ok(bytes) => ledger_preview::set_source(
            a,
            Some(ledger_preview::Source {
                filename: path,
                bytes_b64: b64(&bytes),
                importable: true,
            }),
        ),
        Err(e) => stm(a).setup_error = Some(e.to_string()),
    }
}

/// `renderSourceFileSetup`.
fn source_file_setup(app: &App, tk: &Tk, sync_label: Option<String>) -> Vec<El> {
    let src = selected(app);
    let m = meta(src.id);
    let ledger = m.source_format == Some("generic_ledger");
    let (label, helper) = source_file_field(src.id);
    let title = tr(
        "connections:add.field.selectExportFileTitle",
        &[("title", &src.title())],
    );
    let on_change: fn(&mut App) = |a| {
        stm(a).last_import = None;
        if meta(selected(a).id).source_format == Some("generic_ledger") {
            ledger_preview::set_source(a, None);
        }
    };
    let mut field = field_input(
        app,
        tk,
        "connection-source-file",
        "sourceFile",
        &label,
        &st(app).form.source_file,
        |a| &mut stm(a).form.source_file,
        None,
        false,
    );
    if ledger {
        // `readOnly` for the generic ledger: its path comes from the picker with the contents.
        field = field.opacity(1.0);
        let _ = on_change;
    }
    let pick: fn(&mut App, String) = if ledger {
        set_ledger_file
    } else {
        set_source_file
    };
    let control = row()
        .w_full()
        .gap(tk.r(0.5))
        .child(col().grow(1.0).min_w(0.0).child(field))
        .child(browse(
            tk,
            "connection-source-browse",
            t("connections:add.field.browse"),
            title,
            file_filters(src.id),
            pick,
        ));
    let mut out = vec![setup_field(
        tk,
        &label,
        control,
        err(app, "sourceFile"),
        helper,
    )];
    if m.source_format == Some("bisq_csv") {
        out.extend(bisq_fields(app, tk));
    }
    if ledger {
        let busy = st(app).template_pending;
        out.push(
            // `flex flex-wrap justify-between gap-2`: the hint is wider than the line, so the
            // buttons wrap under it.
            col()
                .w_full()
                .px(tk.r(0.75))
                .py(tk.r(0.5))
                .gap(tk.r(0.5))
                .radius(tk.r(0.375))
                .border(1.0, tk.border.alpha(tk.border.a * 0.7))
                .border_layout()
                .child(
                    text(t("connections:add.genericLedger.templateHint"))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .wrap()
                        .w_full(),
                )
                .child(
                    row()
                        .gap(tk.r(0.5))
                        .child({
                            let b = button(
                                tk,
                                "add-template-xlsx",
                                Variant::Outline,
                                true,
                                None,
                                &t(if busy {
                                    "connections:add.genericLedger.templateWorking"
                                } else {
                                    "connections:add.genericLedger.downloadXlsx"
                                }),
                                !busy,
                            );
                            if busy {
                                b
                            } else {
                                b.on_press(|a: &mut App| submit::download_template(a, "xlsx"))
                            }
                        })
                        .child({
                            let b = button(
                                tk,
                                "add-template-csv",
                                Variant::Ghost,
                                true,
                                None,
                                &t("connections:add.genericLedger.downloadCsv"),
                                !busy,
                            );
                            if busy {
                                b
                            } else {
                                b.on_press(|a: &mut App| submit::download_template(a, "csv"))
                            }
                        }),
                ),
        );
        if let Some(p) = ledger_preview::view(app, tk) {
            out.push(p);
        }
    }
    if let Some(l) = sync_label {
        out.push(sync_after(app, tk, &l));
    }
    out
}

fn set_companion_file(a: &mut App, path: String) {
    stm(a).form.companion_file = TextEdit::new(&path);
    stm(a).errors.remove("companionFile");
    stm(a).last_import = None;
}

/// Bisq reads its Funds > Transactions export beside the trade history and needs the zone the
/// exporting computer wrote its local times in.
fn bisq_fields(app: &App, tk: &Tk) -> Vec<El> {
    let src = selected(app);
    let label = t("connections:add.exportFile.bisqTransactionsLabel");
    let field = field_input(
        app,
        tk,
        "connection-companion-file",
        "companionFile",
        &label,
        &st(app).form.companion_file,
        |a| &mut stm(a).form.companion_file,
        None,
        false,
    );
    let control = row()
        .w_full()
        .gap(tk.r(0.5))
        .child(col().grow(1.0).min_w(0.0).child(field))
        .child(browse(
            tk,
            "connection-companion-browse",
            t("connections:add.field.browse"),
            label.clone(),
            file_filters(src.id),
            set_companion_file,
        ));
    let zone = t("connections:add.exportFile.bisqTimezoneLabel");
    vec![
        setup_field(
            tk,
            &label,
            control,
            err(app, "companionFile"),
            Some(t("connections:add.exportFile.bisqTransactionsHelper")),
        ),
        setup_field(
            tk,
            &zone,
            field_input(
                app,
                tk,
                "connection-source-timezone",
                "sourceTimezone",
                &zone,
                &st(app).form.source_timezone,
                |a| &mut stm(a).form.source_timezone,
                Some("Europe/Vienna"),
                false,
            ),
            err(app, "sourceTimezone"),
            Some(t("connections:add.exportFile.bisqTimezoneHelper")),
        ),
    ]
}

/// A two-button toggle (`grid grid-cols-2 gap-2`, the active one `secondary`).
fn toggle2(tk: &Tk, key: &str, a: (&str, bool, fn(&mut App)), b: (&str, bool, fn(&mut App))) -> El {
    let cell = |k: String, (label, on, f): (&str, bool, fn(&mut App))| {
        col().grow(1.0).min_w(0.0).child(
            button(
                tk,
                &k,
                if on {
                    Variant::Secondary
                } else {
                    Variant::Outline
                },
                false,
                None,
                label,
                true,
            )
            .w_full()
            .checked(on)
            .on_press(f),
        )
    };
    row()
        .w_full()
        .gap(tk.r(0.5))
        .child(cell(format!("{key}-a"), a))
        .child(cell(format!("{key}-b"), b))
}

/// Text with `<code>…</code>` spans (react-i18next `Trans` with `code: <InlineCode/>`).
fn code_text(tk: &Tk, template: &str, color: Color) -> El {
    let mut plain = String::new();
    let mut spans = Vec::new();
    let mut rest = template;
    while let Some(i) = rest.find("<code>") {
        plain.push_str(&rest[..i]);
        rest = &rest[i + 6..];
        let j = rest.find("</code>").unwrap_or(rest.len());
        let start = plain.len();
        plain.push_str(&rest[..j]);
        spans.push(
            Span::new(start, plain.len())
                .mono()
                .color(tk.fg)
                .bg(tk.muted, 2.0),
        );
        rest = rest.get(j + 7..).unwrap_or("");
    }
    plain.push_str(rest);
    rich(plain, spans).font(tk.txs(400.0)).color(color).wrap()
}

/// `CommandSnippet`.
fn command(tk: &Tk, s: &str) -> El {
    col()
        .w_full()
        .min_w(0.0)
        .px(tk.r(0.5))
        .py(tk.r(0.375))
        .radius(tk.r(0.375))
        .border(1.0, tk.border)
        .border_layout()
        .fill(tk.background)
        .child(
            text(s.to_owned())
                .font(tk.txs(400.0).mono().lh(tk.r(0.75) * 1.625))
                .color(tk.fg)
                .wrap()
                .w_full(),
        )
}

/// A muted info box (`rounded-md border bg-muted/25 p-3 text-xs text-muted-foreground`).
fn info_box(tk: &Tk, children: Vec<El>) -> El {
    let mut c = col()
        .w_full()
        .padding(tk.r(0.75))
        .gap(tk.r(0.5))
        .radius(tk.r(0.375))
        .border(1.0, tk.border)
        .border_layout()
        .fill(tk.muted.alpha(0.25));
    for ch in children {
        c = c.child(ch);
    }
    c
}

/// An outlined box (`rounded-md border bg-background p-3 space-y-*`).
fn plain_box(tk: &Tk, gap: f32, children: Vec<El>) -> El {
    let mut c = col()
        .w_full()
        .padding(tk.r(0.75))
        .gap(gap)
        .radius(tk.r(0.375))
        .border(1.0, tk.border)
        .border_layout()
        .fill(tk.background);
    for ch in children {
        c = c.child(ch);
    }
    c
}

fn grid2(tk: &Tk, a: El, b: El) -> El {
    row()
        .w_full()
        .gap(tk.r(0.75))
        .items_start()
        .child(col().grow(1.0).min_w(0.0).child(a))
        .child(col().grow(1.0).min_w(0.0).child(b))
}

fn date_placeholder() -> &'static str {
    if crate::i18n::lang() == crate::i18n::Lang::De {
        "TT.MM.JJJJ"
    } else {
        "mm/dd/yyyy"
    }
}

/// A `type="date"` input (typed, `mm/dd/yyyy` or ISO; normalised on submit).
fn date_input(
    app: &App,
    tk: &Tk,
    key: &'static str,
    err_key: &'static str,
    name: &str,
    edit: &TextEdit,
    access: Access,
) -> El {
    let _ = app;
    let ck = clear(err_key);
    input(
        tk,
        key,
        edit,
        access,
        name,
        Some(date_placeholder()),
        false,
        move |a: &mut App| {
            ck(a);
            submit::coverage_inputs_changed(a);
        },
    )
}

fn amber_ink(tk: &Tk) -> Color {
    if tk.dark { h("#fcd34d") } else { h("#b45309") }
}
fn emerald_ink(tk: &Tk) -> Color {
    if tk.dark { h("#6ee7b7") } else { h("#047857") }
}

/// `renderWalletCoreCoverage`.
fn core_coverage(app: &App, tk: &Tk) -> Option<El> {
    let b = submit::selected_backend(app)?;
    if b.get("kind").and_then(Value::as_str) != Some("bitcoinrpc") {
        return None;
    }
    let k = meta(selected(app).id).kind();
    let available =
        matches!(k, SetupKind::Descriptor | SetupKind::AddressList) && !name_of(&b).is_empty();
    let unknown = || t("connections:add.core.retainedDateUnknown");
    let retained = |p: &Value| {
        p.get("earliest_retained_at")
            .and_then(Value::as_str)
            .map(|s| s.chars().take(10).collect::<String>())
            .unwrap_or_else(unknown)
    };
    let birthday = st(app).form.birthday.text.trim().to_owned();
    let tinted = |border: Color, fill: Color, ink: Color, body: String| {
        row()
            .w_full()
            .padding(tk.r(0.75))
            .radius(tk.r(0.375))
            .border(1.0, border)
            .border_layout()
            .fill(fill)
            .child(text(body).font(tk.txs(400.0)).color(ink).wrap())
    };
    let amber = |body: String| {
        tinted(
            h("#f59e0b").alpha(0.4),
            h("#f59e0b").alpha(0.1),
            if tk.dark { h("#fde68a") } else { h("#92400e") },
            body,
        )
    };
    let green = |body: String| {
        tinted(
            h("#10b981").alpha(0.4),
            h("#10b981").alpha(0.1),
            if tk.dark { h("#a7f3d0") } else { h("#065f46") },
            body,
        )
    };
    Some(match &st(app).coverage {
        Coverage::Idle => {
            if !available {
                return None;
            }
            row()
                .w_full()
                .padding(tk.r(0.75))
                .gap(tk.r(0.5))
                .center_y()
                .flex_wrap()
                .radius(tk.r(0.375))
                .border(1.0, tk.border)
                .border_layout()
                .fill(tk.muted.alpha(0.3))
                .child(
                    text(t("connections:add.core.coverageIdle"))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .wrap(),
                )
                .child(
                    button(
                        tk,
                        "add-coverage-check",
                        Variant::Outline,
                        true,
                        None,
                        &t("connections:add.core.coverageCheck"),
                        true,
                    )
                    .on_press(submit::check_coverage),
                )
        }
        Coverage::Checking => row()
            .w_full()
            .padding(tk.r(0.75))
            .gap(tk.r(0.5))
            .center_y()
            .radius(tk.r(0.375))
            .border(1.0, tk.border)
            .border_layout()
            .fill(tk.muted.alpha(0.3))
            .child(crate::txdetail::widgets::spinner(tk, tk.r(1.0)))
            .child(
                text(t("connections:add.core.coverageChecking"))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg),
            ),
        Coverage::BirthdayRequired(p) => amber(tr(
            "connections:add.core.coverageBirthdayRequired",
            &[("retained", &retained(p))],
        )),
        Coverage::VerificationRequired(p) => amber(tr(
            "connections:add.core.coverageVerificationRequired",
            &[("birthday", &birthday), ("retained", &retained(p))],
        )),
        Coverage::Available(p) => {
            if p.get("pruned").and_then(Value::as_bool) != Some(true) {
                green(t("connections:add.core.coverageFullHistory"))
            } else {
                green(tr(
                    "connections:add.core.coverageAvailablePruned",
                    &[("birthday", &birthday), ("retained", &retained(p))],
                ))
            }
        }
        Coverage::Blocked { details, .. } | Coverage::Error { details, .. } => {
            let msg = match &st(app).coverage {
                Coverage::Blocked { .. } => {
                    let r = details
                        .get("earliest_retained_at")
                        .and_then(Value::as_str)
                        .map(|s| s.chars().take(10).collect())
                        .unwrap_or_else(unknown);
                    let b = if birthday.is_empty() {
                        t("connections:add.core.birthdayUnknown")
                    } else {
                        birthday.clone()
                    };
                    tr(
                        "connections:add.core.coverageBlocked",
                        &[("birthday", &b), ("retained", &r)],
                    )
                }
                Coverage::Error { message, .. } => message.clone(),
                _ => String::new(),
            };
            tinted(
                tk.destructive.alpha(0.4),
                tk.destructive.alpha(0.1),
                tk.destructive,
                msg,
            )
        }
    })
}

fn script_label(v: &str) -> String {
    let k = if v == "p2sh-p2wpkh" { "p2shp2wpkh" } else { v };
    t(&format!("connections:add.descriptor.scriptType.{k}"))
}

/// A backend row's endpoint, with the rows' placeholders (`backend_rows`) translated.
fn endpoint_text(endpoint: &str) -> String {
    match endpoint {
        "Configured endpoint" => t("connections:add.backendSettings.endpoint.configured"),
        "Missing endpoint" => t("connections:add.backendSettings.endpoint.missing"),
        e => e.to_owned(),
    }
}

/// `renderWalletMaterialFeedback`.
fn material_feedback(app: &App, tk: &Tk) -> Option<El> {
    let f = &st(app).form;
    let d = wallet_material::detect(&f.wallet_material.text);
    match d.kind {
        wallet_material::Kind::Empty => None,
        wallet_material::Kind::BareXpub => {
            let s = if f.script_type.is_empty() {
                tr(
                    "connections:add.descriptor.bareXpubAutoDetect",
                    &[("label", &d.label)],
                )
            } else {
                tr(
                    "connections:add.descriptor.bareXpubResolved",
                    &[
                        ("label", &d.label),
                        ("scriptType", &script_label(&f.script_type)),
                    ],
                )
            };
            Some(xs(tk, s, emerald_ink(tk)))
        }
        k => {
            let ink = if k == wallet_material::Kind::Unknown {
                amber_ink(tk)
            } else {
                emerald_ink(tk)
            };
            let s = match &d.hint {
                Some(hint) => tr(
                    "connections:add.descriptor.detectedWithHint",
                    &[("label", &d.label), ("hint", hint)],
                ),
                None => tr(
                    "connections:add.descriptor.detected",
                    &[("label", &d.label)],
                ),
            };
            Some(xs(tk, s, ink))
        }
    }
}

/// `renderDescriptorPreview`.
fn descriptor_preview(app: &App, tk: &Tk) -> Option<El> {
    let s = st(app);
    if let Some(e) = &s.preview_error {
        return Some(
            row()
                .w_full()
                .padding(tk.r(0.5))
                .radius(tk.r(0.375))
                .border(1.0, tk.destructive.alpha(0.3))
                .border_layout()
                .fill(tk.destructive.alpha(0.1))
                .child(
                    text(e.clone())
                        .font(tk.txs(400.0))
                        .color(tk.destructive)
                        .wrap(),
                ),
        );
    }
    let list = s.preview_addresses.as_ref().filter(|l| !l.is_empty())?;
    let mut ul = col().w_full().gap(tk.r(0.25));
    for (branch, index, address) in list {
        let branch_label = if branch == "change" {
            t("connections:add.descriptor.branchChange")
        } else if branch == "receive" {
            tr(
                "connections:add.descriptor.branchReceive",
                &[("index", &index.to_string())],
            )
        } else if branch.ends_with("change") {
            branch.clone()
        } else {
            format!("{branch} {index}")
        };
        let copied = s.copied_address.as_deref() == Some(address.as_str());
        let addr = address.clone();
        ul = ul.child(
            row()
                .w_full()
                .gap(tk.r(0.5))
                .center_y()
                .child(
                    text(branch_label)
                        .font(tk.txs(400.0).mono())
                        .color(tk.muted_fg)
                        .w(tk.r(8.0)),
                )
                .child(
                    text(address.clone())
                        .font(tk.txs(400.0).mono())
                        .color(tk.fg)
                        .truncate()
                        .grow(1.0)
                        .min_w(0.0),
                )
                .child(
                    row()
                        .key(format!("add-copy-{address}"))
                        .px(tk.r(0.5))
                        .py(tk.r(0.125))
                        .radius(tk.r(0.375))
                        .border(1.0, tk.border)
                        .border_layout()
                        .hover(tk.muted.alpha(0.4))
                        .role("button")
                        .label(t("connections:add.descriptor.copyAddress"))
                        .help(t("connections:add.descriptor.copyAddress"))
                        .focusable(true)
                        .on_press_tx(move |a: &mut App, tx: &mut Tx| {
                            crate::osleaks::clipboard::copy(a, tx, addr.clone());
                            stm(a).copied_address = Some(addr.clone());
                        })
                        .child(
                            text(
                                t(if copied {
                                    "connections:add.descriptor.copied"
                                } else {
                                    "connections:add.descriptor.copy"
                                })
                                .to_uppercase(),
                            )
                            .font(tk.t2xs(400.0).tracking(0.3))
                            .color(tk.muted_fg),
                        ),
                ),
        );
    }
    Some(plain_box(
        tk,
        tk.r(0.5),
        vec![
            text(t("connections:add.descriptor.firstDerived"))
                .font(tk.txs(500.0))
                .color(tk.muted_fg),
            ul,
        ],
    ))
}

fn descriptor_fields(app: &App, tk: &Tk, layer: &mut Vec<El>) -> Vec<El> {
    let f = &st(app).form;
    let mut out = vec![
        label_field(app, tk),
        backend_select(
            app,
            tk,
            layer,
            &t("connections:add.field.backend"),
            submit::address_options(app),
            None,
        ),
    ];
    let wm_name = t("connections:add.descriptor.walletExport");
    let title = tr(
        "connections:scanner.scanTitle",
        &[("title", &selected(app).title())],
    );
    let scan_row = row()
        .w_full()
        .gap(tk.r(0.75))
        .center_y()
        .justify_between()
        .child(
            text(t("connections:add.descriptor.scanHint"))
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .wrap()
                .grow(1.0)
                .min_w(0.0),
        )
        .child(
            button(
                tk,
                "add-scan",
                Variant::Outline,
                true,
                Some((&I::SCAN_LINE, tk.r(1.0), false)),
                &t("connections:add.descriptor.scan"),
                true,
            )
            .on_press(move |a: &mut App| scanner::open(a, title.clone())),
        );
    let mut wm = col()
        .w_full()
        .gap(tk.r(0.5))
        .child(scan_row)
        .child(textarea(
            app,
            tk,
            "connection-wallet-material",
            "walletMaterial",
            &wm_name,
            |a| &mut stm(a).form.wallet_material,
            &f.wallet_material,
            tk.r(8.0),
            None,
            Some(|a: &mut App| {
                let s = stm(a);
                s.form.script_type.clear();
                s.preview_addresses = None;
                s.preview_error = None;
            }),
        ));
    if let Some(fb) = material_feedback(app, tk) {
        wm = wm.child(fb);
    }
    out.push(setup_field(
        tk,
        &wm_name,
        wm,
        err(app, "walletMaterial"),
        None,
    ));
    if wallet_material::detect(&f.wallet_material.text).kind == wallet_material::Kind::BareXpub {
        let mut opts = vec![(
            String::new(),
            t("connections:add.descriptor.scriptTypeDetect"),
            true,
        )];
        for v in wallet_material::BARE_XPUB_SCRIPT_TYPES {
            opts.push((v.to_owned(), script_label(v), true));
        }
        let name = t("connections:add.descriptor.scriptTypeLabel");
        let sel = native_select(
            app,
            tk,
            layer,
            "connection-script-type",
            &name,
            opts,
            &f.script_type,
            true,
            |a: &mut App, v: &str| stm(a).form.script_type = v.to_owned(),
        );
        out.push(setup_field(
            tk,
            &name,
            sel,
            None,
            Some(t("connections:add.descriptor.scriptTypeHelperAuto")),
        ));
    }
    let gap_name = t("connections:add.descriptor.gapLimit");
    out.push(setup_field(
        tk,
        &gap_name,
        field_input(
            app,
            tk,
            "connection-gap-limit",
            "gapLimit",
            &gap_name,
            &st(app).form.gap_limit,
            |a| &mut stm(a).form.gap_limit,
            None,
            false,
        ),
        err(app, "gapLimit"),
        Some(t("connections:add.descriptor.gapLimitHelper")),
    ));
    let bd_name = t("connections:add.descriptor.birthday");
    let core = submit::selected_backend(app)
        .is_some_and(|b| b.get("kind").and_then(Value::as_str) == Some("bitcoinrpc"));
    out.push(setup_field(
        tk,
        &bd_name,
        date_input(
            app,
            tk,
            "connection-birthday",
            "birthday",
            &bd_name,
            &st(app).form.birthday,
            |a| &mut stm(a).form.birthday,
        ),
        err(app, "birthday"),
        Some(t(if core {
            "connections:add.descriptor.birthdayHelperCore"
        } else {
            "connections:add.descriptor.birthdayHelper"
        })),
    ));
    if let Some(c) = core_coverage(app, tk) {
        out.push(c);
    }
    let pending = st(app).preview_pending;
    let enabled = !pending && !f.wallet_material.text.trim().is_empty();
    let pb = button(
        tk,
        "add-preview-addresses",
        Variant::Outline,
        true,
        None,
        &t(if pending {
            "connections:add.descriptor.deriving"
        } else {
            "connections:add.descriptor.previewAddresses"
        }),
        enabled,
    );
    out.push(
        row()
            .w_full()
            .gap(tk.r(0.5))
            .center_y()
            .child(if enabled {
                pb.on_press(submit::preview_addresses)
            } else {
                pb
            })
            .child(
                text(t("connections:add.descriptor.previewHint"))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg)
                    .wrap()
                    .grow(1.0)
                    .min_w(0.0),
            ),
    );
    if let Some(p) = descriptor_preview(app, tk) {
        out.push(p);
    }
    out.push(sync_after(
        app,
        tk,
        &t("connections:add.descriptor.scanAfter"),
    ));
    out
}

fn address_list_fields(app: &App, tk: &Tk, layer: &mut Vec<El>) -> Vec<El> {
    let f = &st(app).form;
    let parsed = address_list::parse(&f.address_list.text);
    let mut out = vec![
        label_field(app, tk),
        backend_select(
            app,
            tk,
            layer,
            &t("connections:add.field.backend"),
            submit::descriptor_options(app),
            None,
        ),
    ];
    let core = submit::selected_backend(app)
        .is_some_and(|b| b.get("kind").and_then(Value::as_str) == Some("bitcoinrpc"));
    if core {
        let bd_name = t("connections:add.descriptor.birthday");
        out.push(setup_field(
            tk,
            &bd_name,
            date_input(
                app,
                tk,
                "connection-address-birthday",
                "birthday",
                &bd_name,
                &st(app).form.birthday,
                |a| &mut stm(a).form.birthday,
            ),
            err(app, "birthday"),
            Some(t("connections:add.descriptor.birthdayHelperCore")),
        ));
        if let Some(c) = core_coverage(app, tk) {
            out.push(c);
        }
    }
    let name = t("connections:add.addressList.addresses");
    let ph = t("connections:add.addressList.placeholder");
    let mut c = col().w_full().gap(tk.r(0.5)).child(textarea(
        app,
        tk,
        "connection-address-list",
        "addressList",
        &name,
        |a| &mut stm(a).form.address_list,
        &f.address_list,
        tk.r(8.0),
        Some(&ph),
        Some(apply_address_input),
    ));
    if let Some((p, k)) = st(app).purged {
        let mut s = String::new();
        if p > 0 {
            s.push_str(&tc("connections:add.addressList.privateKeysPurged", p, &[]));
        }
        if p > 0 && k > 0 {
            s.push(' ');
        }
        if k > 0 {
            s.push_str(&tc("connections:add.addressList.publicKeysPurged", k, &[]));
        }
        c = c.child(
            text(s)
                .font(tk.txs(500.0))
                .color(tk.destructive)
                .wrap()
                .role("alert"),
        );
    }
    let mut actions = row().w_full().gap(tk.r(0.75)).center_y().flex_wrap().child(
        browse(
            tk,
            "add-address-file",
            t("connections:add.addressList.loadFile"),
            t("connections:add.addressList.loadFile"),
            vec![("Text".into(), vec!["txt", "csv"])],
            |a: &mut App, path: String| match std::fs::read_to_string(&path) {
                Ok(loaded) => {
                    let loaded = loaded.trim().to_owned();
                    if loaded.is_empty() {
                        return;
                    }
                    let existing = stm(a).form.address_list.text.trim().to_owned();
                    let next = if existing.is_empty() {
                        loaded
                    } else {
                        format!("{existing}\n{loaded}")
                    };
                    stm(a).form.address_list = TextEdit::new(&next);
                    apply_address_input(a);
                }
                Err(_) => {
                    stm(a).setup_error = Some(t("connections:add.addressList.fileReadFailed"))
                }
            },
        )
        .h(tk.r(2.0)),
    );
    if !parsed.entries.is_empty() {
        let mut s = tc(
            "connections:add.addressList.summary",
            parsed.valid.len(),
            &[],
        );
        if parsed.duplicates > 0 {
            s.push_str(&format!(
                " · {}",
                tc(
                    "connections:add.addressList.summaryDuplicates",
                    parsed.duplicates,
                    &[]
                )
            ));
        }
        if !parsed.invalid.is_empty() {
            s.push_str(&format!(
                " · {}",
                tc(
                    "connections:add.addressList.summaryInvalid",
                    parsed.invalid.len(),
                    &[]
                )
            ));
        }
        actions = actions.child(text(s).font(tk.txs(400.0)).color(tk.muted_fg));
    }
    c = c.child(actions);
    if !parsed.invalid.is_empty() {
        let samples = parsed
            .invalid
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        c = c.child(xs(
            tk,
            tr(
                "connections:add.addressList.invalidHint",
                &[("samples", &samples)],
            ),
            amber_ink(tk),
        ));
    }
    out.push(setup_field(
        tk,
        &name,
        c,
        err(app, "addressList"),
        Some(t("connections:add.addressList.helper")),
    ));
    out.push(sync_after(
        app,
        tk,
        &t("connections:add.addressList.scanAfter"),
    ));
    out
}

/// `applyAddressInput`: scrub pasted key material out of the address list.
fn apply_address_input(a: &mut App) {
    let raw = stm(a).form.address_list.text.clone();
    let stripped = address_list::strip_key_material(&raw);
    let removed = stripped.private_keys + stripped.public_keys > 0;
    let s = stm(a);
    if removed {
        s.form.address_list = TextEdit::new(&stripped.text);
        let (p, k) = s.purged.unwrap_or((0, 0));
        s.purged = Some((p + stripped.private_keys, k + stripped.public_keys));
    } else if raw.trim().is_empty() {
        s.purged = None;
    }
}

fn silent_payment_fields(app: &App, tk: &Tk, layer: &mut Vec<El>) -> Vec<El> {
    let f = &st(app).form;
    let mut out = vec![
        label_field(app, tk),
        backend_select(
            app,
            tk,
            layer,
            &t("connections:add.silentPayments.backend"),
            submit::silent_payment_options(app),
            Some(t("connections:add.silentPayments.backendHelper")),
        ),
    ];
    let m_name = t("connections:add.silentPayments.material");
    out.push(setup_field(
        tk,
        &m_name,
        textarea(
            app,
            tk,
            "connection-sp-descriptor",
            "spDescriptor",
            &m_name,
            |a| &mut stm(a).form.sp_descriptor,
            &f.sp_descriptor,
            tk.r(7.0),
            Some("sp(spscan1q...)"),
            None,
        ),
        err(app, "spDescriptor"),
        Some(t("connections:add.silentPayments.materialHelper")),
    ));
    let mode_name = t("connections:add.silentPayments.scanMode");
    let opts = vec![
        (
            "local_index".into(),
            t("connections:add.silentPayments.scanModeLocal"),
            true,
        ),
        (
            "server_assisted".into(),
            t("connections:add.silentPayments.scanModeServer"),
            true,
        ),
    ];
    let sel = native_select(
        app,
        tk,
        layer,
        "connection-sp-scan-mode",
        &mode_name,
        opts,
        f.sp_scan_mode,
        true,
        |a: &mut App, v: &str| {
            stm(a).form.sp_scan_mode = if v == "server_assisted" {
                "server_assisted"
            } else {
                "local_index"
            };
            submit::apply_default_backend(a);
        },
    );
    out.push(setup_field(
        tk,
        &mode_name,
        sel,
        None,
        Some(t("connections:add.silentPayments.scanModeHelper")),
    ));
    let hn = t("connections:add.silentPayments.startHeight");
    let dn = t("connections:add.silentPayments.startDate");
    let full = f.sp_full_history;
    let height = field_input(
        app,
        tk,
        "connection-sp-start-height",
        "spScanStartHeight",
        &hn,
        &st(app).form.sp_start_height,
        |a| &mut stm(a).form.sp_start_height,
        None,
        false,
    );
    let date = date_input(
        app,
        tk,
        "connection-sp-start-date",
        "spScanStartDate",
        &dn,
        &st(app).form.sp_start_date,
        |a| &mut stm(a).form.sp_start_date,
    );
    let (height, date) = if full {
        (
            height.opacity(0.5).disabled(true),
            date.opacity(0.5).disabled(true),
        )
    } else {
        (height, date)
    };
    out.push(grid2(
        tk,
        setup_field(tk, &hn, height, err(app, "spScanStartHeight"), None),
        setup_field(
            tk,
            &dn,
            date,
            err(app, "spScanStartDate"),
            Some(t("connections:add.silentPayments.startDateHelper")),
        ),
    ));
    let mut hist = vec![checkbox(
        tk,
        "add-sp-full",
        full,
        &t("connections:add.silentPayments.fullHistory"),
        true,
        |a: &mut App| {
            let f = &mut stm(a).form;
            f.sp_full_history = !f.sp_full_history;
        },
    )];
    if full {
        hist.push(checkbox(
            tk,
            "add-sp-full-ack",
            f.sp_ack_full,
            &t("connections:add.silentPayments.fullHistoryAck"),
            true,
            |a: &mut App| {
                let f = &mut stm(a).form;
                f.sp_ack_full = !f.sp_ack_full;
                stm(a).errors.remove("spAcknowledgeFullHistoryWarning");
            },
        ));
    }
    if let Some(e) = err(app, "spAcknowledgeFullHistoryWarning") {
        hist.push(error_line(tk, e, false));
    }
    out.push(plain_box(tk, tk.r(0.5), hist));
    if f.sp_scan_mode == "server_assisted" {
        let mut c = col()
            .w_full()
            .padding(tk.r(0.75))
            .gap(tk.r(0.5))
            .radius(tk.r(0.375))
            .border(1.0, h("#f59e0b").alpha(0.3))
            .border_layout()
            .fill(h("#f59e0b").alpha(0.1))
            .child(checkbox(
                tk,
                "add-sp-server-ack",
                f.sp_ack_server,
                &t("connections:add.silentPayments.serverAck"),
                true,
                |a: &mut App| {
                    let f = &mut stm(a).form;
                    f.sp_ack_server = !f.sp_ack_server;
                    stm(a).errors.remove("spAcknowledgeServerWarning");
                },
            ));
        if let Some(e) = err(app, "spAcknowledgeServerWarning") {
            c = c.child(error_line(tk, e, false));
        }
        out.push(c);
    }
    // The BIP329 export block the web renders inside this form.
    let em = t("connections:add.bip329.exportModeLabel");
    let ew = t("connections:add.bip329.exportWalletLabel");
    let mode = native_select(
        app,
        tk,
        layer,
        "connection-bip329-export-mode",
        &em,
        vec![
            (
                "stored".into(),
                t("connections:add.bip329.exportModeStored"),
                true,
            ),
            (
                "synthesized".into(),
                t("connections:add.bip329.exportModeSynthesized"),
                true,
            ),
            (
                "all".into(),
                t("connections:add.bip329.exportModeAll"),
                true,
            ),
        ],
        f.bip329_export_mode,
        true,
        |a: &mut App, v: &str| {
            stm(a).form.bip329_export_mode = match v {
                "synthesized" => "synthesized",
                "all" => "all",
                _ => "stored",
            }
        },
    );
    let mut wopts = vec![(
        String::new(),
        t("connections:add.bip329.exportWalletProfile"),
        true,
    )];
    for w in submit::wallets(app) {
        let l = w
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        wopts.push((l.clone(), l, true));
    }
    let wallet = native_select(
        app,
        tk,
        layer,
        "connection-bip329-export-wallet",
        &ew,
        wopts,
        &f.bip329_wallet,
        true,
        |a: &mut App, v: &str| stm(a).form.bip329_wallet = v.to_owned(),
    );
    let busy = st(app).bip329_export_pending;
    let eb = button(
        tk,
        "add-bip329-export",
        Variant::Outline,
        false,
        None,
        &t(if busy {
            "connections:add.bip329.exporting"
        } else {
            "connections:add.bip329.exportButton"
        }),
        !busy,
    );
    out.push(plain_box(
        tk,
        tk.r(0.75),
        vec![
            grid2(
                tk,
                setup_field(tk, &em, mode, None, None),
                setup_field(tk, &ew, wallet, None, None),
            ),
            row().child(if busy {
                eb
            } else {
                eb.on_press(submit::export_bip329)
            }),
        ],
    ));
    out
}

fn samourai_fields(app: &App, tk: &Tk, layer: &mut Vec<El>) -> Vec<El> {
    let f = &st(app).form;
    let mut out = vec![
        label_field(app, tk),
        backend_select(
            app,
            tk,
            layer,
            &t("connections:add.field.backend"),
            submit::descriptor_options(app),
            None,
        ),
    ];
    let mut c = col().w_full().gap(tk.r(0.75)).child(info_box(
        tk,
        vec![
            text(t("connections:add.samourai.intro"))
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .wrap(),
        ],
    ));
    let fields: [(&'static str, &'static str, &str, Access); 4] = [
        (
            "connection-samourai-deposit",
            "samouraiDeposit",
            "deposit",
            |a| &mut stm(a).form.samourai[0],
        ),
        (
            "connection-samourai-badbank",
            "samouraiBadbank",
            "badbank",
            |a| &mut stm(a).form.samourai[1],
        ),
        (
            "connection-samourai-premix",
            "samouraiPremix",
            "premix",
            |a| &mut stm(a).form.samourai[2],
        ),
        (
            "connection-samourai-postmix",
            "samouraiPostmix",
            "postmix",
            |a| &mut stm(a).form.samourai[3],
        ),
    ];
    for (i, (key, ek, sec, acc)) in fields.into_iter().enumerate() {
        let name = t(&format!("connections:add.samourai.{sec}Label"));
        c = c.child(setup_field(
            tk,
            &name,
            textarea(
                app,
                tk,
                key,
                ek,
                &name,
                acc,
                &f.samourai[i],
                tk.r(5.0),
                None,
                None,
            ),
            err(app, ek),
            Some(t(&format!("connections:add.samourai.{sec}Helper"))),
        ));
    }
    out.push(c);
    let gn = t("connections:add.samourai.gapLimit");
    out.push(setup_field(
        tk,
        &gn,
        field_input(
            app,
            tk,
            "connection-samourai-gap-limit",
            "gapLimit",
            &gn,
            &st(app).form.gap_limit,
            |a| &mut stm(a).form.gap_limit,
            None,
            false,
        ),
        err(app, "gapLimit"),
        Some(t("connections:add.samourai.gapLimitHelper")),
    ));
    out.push(sync_after(
        app,
        tk,
        &t("connections:add.samourai.scanAfter"),
    ));
    out
}

fn wasabi_fields(app: &App, tk: &Tk) -> Vec<El> {
    let f = &st(app).form;
    let mut out = if super::history_uses_existing(app) {
        vec![]
    } else {
        vec![label_field(app, tk)]
    };
    let rpc = f.wasabi_mode == "rpc";
    out.push(setup_field(
        tk,
        &t("connections:add.wasabi.importSource"),
        toggle2(
            tk,
            "connection-wasabi-mode",
            (&t("connections:add.wasabi.pasteRpc"), rpc, |a: &mut App| {
                stm(a).form.wasabi_mode = "rpc";
                stm(a).last_import = None;
            }),
            (
                &t("connections:add.wasabi.bundleFile"),
                !rpc,
                |a: &mut App| {
                    stm(a).form.wasabi_mode = "bundle-file";
                    stm(a).last_import = None;
                },
            ),
        ),
        None,
        None,
    ));
    if !rpc {
        out.extend(source_file_setup(
            app,
            tk,
            Some(t("connections:add.wasabi.importAfter")),
        ));
        return out;
    }
    let intro1 = tr(
        "connections:add.wasabi.rpcIntro1",
        &[
            ("endpoint", "http://127.0.0.1:37128/WalletName"),
            ("placeholder", "WalletName"),
        ],
    );
    let intro2 = tr(
        "connections:add.wasabi.rpcIntro2",
        &[
            ("user", "JsonRpcUser"),
            ("password", "JsonRpcPassword"),
            ("auth", "-u user:password"),
        ],
    );
    let mut c = col().w_full().gap(tk.r(0.75)).child(info_box(
        tk,
        vec![
            code_text(tk, &intro1, tk.muted_fg),
            code_text(tk, &intro2, tk.muted_fg),
        ],
    ));
    let curl = |m: &str| {
        format!(
            "curl -s --data-binary '{{\"jsonrpc\":\"2.0\",\"id\":\"1\",\"method\":\"{m}\",\"params\":[]}}' http://127.0.0.1:37128/WalletName | jq"
        )
    };
    type Field<'a> = (
        &'static str,
        &'static str,
        &'a str,
        Option<&'a str>,
        f32,
        Access,
    );
    let fields: [Field; 3] = [
        (
            "connection-wasabi-history",
            "wasabiHistory",
            "history",
            Some("gethistory"),
            7.0,
            |a| &mut stm(a).form.wasabi[0],
        ),
        (
            "connection-wasabi-coins",
            "wasabiCoins",
            "coins",
            Some("listcoins"),
            6.0,
            |a| &mut stm(a).form.wasabi[1],
        ),
        (
            "connection-wasabi-wallet-info",
            "wasabiWalletInfo",
            "walletInfo",
            Some("getwalletinfo"),
            5.0,
            |a| &mut stm(a).form.wasabi[2],
        ),
    ];
    for (i, (key, ek, k, method, mh, acc)) in fields.into_iter().enumerate() {
        let name = t(&format!("connections:add.wasabi.{k}Label"));
        let ta = textarea(
            app,
            tk,
            key,
            ek,
            &name,
            acc,
            &f.wasabi[i],
            tk.r(mh),
            None,
            None,
        );
        let mut field = col()
            .w_full()
            .gap(tk.r(0.5))
            .child(super::btcpay::form::label(tk, &name))
            .child(ta);
        match err(app, ek) {
            Some(e) => field = field.child(error_line(tk, e, false)),
            None => {
                field = field.child(
                    col()
                        .w_full()
                        .gap(tk.r(0.375))
                        .child(xs(
                            tk,
                            t(&format!("connections:add.wasabi.{k}Helper")),
                            tk.muted_fg,
                        ))
                        .child(command(tk, &curl(method.unwrap_or("")))),
                );
            }
        }
        c = c.child(field);
    }
    let an = t("connections:add.wasabi.additionalLabel");
    let ta = textarea(
        app,
        tk,
        "connection-wasabi-additional",
        "wasabiAdditional",
        &an,
        |a| &mut stm(a).form.wasabi[3],
        &f.wasabi[3],
        tk.r(5.0),
        None,
        None,
    );
    let mut field = col()
        .w_full()
        .gap(tk.r(0.5))
        .child(super::btcpay::form::label(tk, &an))
        .child(ta);
    match err(app, "wasabiAdditional") {
        Some(e) => field = field.child(error_line(tk, e, false)),
        None => {
            field = field.child(code_text(
                tk,
                &tr(
                    "connections:add.wasabi.additionalHelper",
                    &[
                        ("listkeys", "listkeys"),
                        ("listpayments", "listpaymentsincoinjoin"),
                        ("walletJson", "wallet_json"),
                    ],
                ),
                tk.muted_fg,
            ))
        }
    }
    c = c.child(field);
    out.push(c);
    out
}

fn wallet_options(app: &App, with_chain: bool) -> Vec<(String, String, bool)> {
    submit::existing_wallets(app)
        .iter()
        .map(|w| {
            let l = w
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let c = w.get("chain").and_then(Value::as_str).unwrap_or("");
            (
                l.clone(),
                if with_chain && !c.is_empty() {
                    format!("{l} ({c})")
                } else {
                    l
                },
                true,
            )
        })
        .collect()
}

fn file_wallet_fields(app: &App, tk: &Tk, layer: &mut Vec<El>) -> Vec<El> {
    let src = selected(app);
    let m = meta(src.id);
    let f = &st(app).form;
    if m.source_format == Some("wasabi_bundle") {
        return wasabi_fields(app, tk);
    }
    if m.source_format == Some("btcpay_csv") && !super::history_import(app) {
        let existing = f.btcpay_csv_mode == "existing_wallet";
        let mut out = vec![setup_field(
            tk,
            &t("connections:add.btcpayCsv.importBehavior"),
            toggle2(
                tk,
                "connection-btcpay-csv-mode",
                (
                    &t("connections:add.btcpayCsv.createWallet"),
                    !existing,
                    |a: &mut App| {
                        stm(a).form.btcpay_csv_mode = "wallet_source";
                        stm(a).last_import = None;
                    },
                ),
                (
                    &t("connections:add.btcpayCsv.importIntoExisting"),
                    existing,
                    |a: &mut App| {
                        let first = submit::existing_wallets(a)
                            .first()
                            .and_then(|w| w.get("label").and_then(Value::as_str))
                            .unwrap_or("")
                            .to_owned();
                        let f = &mut stm(a).form;
                        f.btcpay_csv_mode = "existing_wallet";
                        if f.target_wallet.is_empty() {
                            f.target_wallet = first;
                        }
                        stm(a).last_import = None;
                    },
                ),
            ),
            None,
            Some(t(if existing {
                "connections:add.btcpayCsv.existingWalletHelper"
            } else {
                "connections:add.btcpayCsv.createWalletHelper"
            })),
        )];
        if existing {
            let wo = wallet_options(app, true);
            let none = wo.is_empty();
            let mut opts = vec![(
                String::new(),
                t(if none {
                    "connections:add.btcpayCsv.noWalletsYet"
                } else {
                    "connections:add.btcpayCsv.chooseWallet"
                }),
                true,
            )];
            opts.extend(wo);
            let name = t("connections:add.btcpayCsv.walletToImport");
            let sel = native_select(
                app,
                tk,
                layer,
                "connection-btcpay-csv-target-wallet",
                &name,
                opts,
                &f.target_wallet,
                !none,
                |a: &mut App, v: &str| {
                    stm(a).form.target_wallet = v.to_owned();
                    stm(a).errors.remove("targetWallet");
                },
            );
            out.push(setup_field(
                tk,
                &name,
                sel,
                err(app, "targetWallet"),
                Some(t("connections:add.btcpayCsv.walletToImportHelper")),
            ));
        } else {
            out.push(label_field(app, tk));
        }
        out.extend(source_file_setup(
            app,
            tk,
            if existing {
                None
            } else {
                Some(t("connections:add.fileWallet.importAfter"))
            },
        ));
        return out;
    }
    let existing = super::history_uses_existing(app);
    let mut out = if existing {
        vec![]
    } else {
        vec![label_field(app, tk)]
    };
    if src.id == "csv" {
        let name = t("connections:add.fileWallet.fileFormat");
        let sel = native_select(
            app,
            tk,
            layer,
            "connection-source-format",
            &name,
            vec![
                (
                    "csv".into(),
                    t("connections:add.fileWallet.formatCsv"),
                    true,
                ),
                (
                    "json".into(),
                    t("connections:add.fileWallet.formatJson"),
                    true,
                ),
            ],
            f.source_format,
            true,
            |a: &mut App, v: &str| {
                stm(a).form.source_format = if v == "json" { "json" } else { "csv" };
                stm(a).last_import = None;
            },
        );
        out.push(setup_field(tk, &name, sel, None, None));
    }
    out.extend(source_file_setup(
        app,
        tk,
        if existing {
            None
        } else {
            Some(t("connections:add.fileWallet.importAfter"))
        },
    ));
    out
}

/// Review's history import: the wallet the rows go into, above a file format's setup
/// (`review-history-target`).
pub fn history_target_block(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let name = t("assistant:evidence.importTarget");
    let mut opts = vec![(
        String::new(),
        t("connections:add.enrichment.errorChooseWallet"),
        true,
    )];
    for w in submit::wallets(app) {
        let id = w.get("id").and_then(Value::as_str).unwrap_or("").to_owned();
        let label = w
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        opts.push((id, label, true));
    }
    opts.push((
        "__new__".into(),
        t("assistant:evidence.newImportTarget"),
        true,
    ));
    let current = st(app)
        .history
        .as_ref()
        .map(|h| h.target.clone())
        .unwrap_or_default();
    let enabled = !super::submitting(app);
    let sel = native_select(
        app,
        tk,
        layer,
        "review-history-target",
        &name,
        opts,
        &current,
        enabled,
        |a: &mut App, v: &str| {
            if let Some(h) = stm(a).history.as_mut() {
                h.target = v.to_owned();
            }
            stm(a).errors.remove("targetWallet");
        },
    );
    let mut block = col()
        .key("review-history-target-block")
        .w_full()
        .px(tk.r(1.25))
        .py(tk.r(1.0))
        .gap(tk.r(0.25))
        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
        .border_layout()
        .child(text(name).font(tk.tsm(500.0)).color(tk.fg))
        .child(sel)
        .child(
            text(t("assistant:evidence.importTargetHint"))
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .wrap(),
        );
    if let Some(e) = err(app, "targetWallet") {
        block = block.child(
            text(e)
                .font(tk.txs(400.0))
                .color(tk.destructive)
                .wrap()
                .role("alert"),
        );
    }
    block
}

fn prior_report_fields(app: &App, tk: &Tk) -> Vec<El> {
    let mut out = vec![label_field(app, tk)];
    let pn = t("connections:add.priorReport.provider");
    let ph = t("connections:add.priorReport.providerPlaceholder");
    out.push(setup_field(
        tk,
        &pn,
        field_input(
            app,
            tk,
            "connection-report-provider",
            "reportProvider",
            &pn,
            &st(app).form.report_provider,
            |a| &mut stm(a).form.report_provider,
            Some(&ph),
            false,
        ),
        err(app, "reportProvider"),
        None,
    ));
    let yn = t("connections:add.priorReport.taxYear");
    out.push(setup_field(
        tk,
        &yn,
        field_input(
            app,
            tk,
            "connection-report-year",
            "taxYear",
            &yn,
            &st(app).form.tax_year,
            |a| &mut stm(a).form.tax_year,
            Some("2025"),
            false,
        ),
        err(app, "taxYear"),
        Some(t("connections:add.priorReport.taxYearHelper")),
    ));
    out.extend(source_file_setup(app, tk, None));
    out
}

fn bull_wallet_fields(app: &App, tk: &Tk, layer: &mut Vec<El>) -> Vec<El> {
    let f = &st(app).form;
    let existing = f.bull_mode == "existing_wallets";
    let mut out = vec![label_field(app, tk)];
    out.push(setup_field(
        tk,
        &t("connections:add.bullWallet.importBehavior"),
        toggle2(
            tk,
            "connection-bull-wallet-mode",
            (
                &t("connections:add.bullWallet.createBullWallets"),
                !existing,
                |a: &mut App| {
                    stm(a).form.bull_mode = "wallet_sources";
                    stm(a).last_import = None;
                },
            ),
            (
                &t("connections:add.bullWallet.mapExistingWallets"),
                existing,
                |a: &mut App| {
                    stm(a).form.bull_mode = "existing_wallets";
                    stm(a).last_import = None;
                },
            ),
        ),
        None,
        None,
    ));
    let mut nets = row().w_full().gap(tk.r(0.5)).items_stretch();
    for (id, lk, hk) in [
        ("bitcoin", "networkBitcoin", "networkBitcoinHelper"),
        ("liquid", "networkLiquid", "networkLiquidHelper"),
        ("lightning", "networkLightning", "networkLightningHelper"),
    ] {
        let on = f.bull_networks.contains(&id);
        let label = t(&format!("connections:add.bullWallet.{lk}"));
        nets = nets.child(
            row()
                .key(format!("add-bull-net-{id}"))
                .grow(1.0)
                .min_w(0.0)
                .min_h(tk.r(5.0))
                .padding(tk.r(0.75))
                .gap(tk.r(0.75))
                .items_start()
                .radius(tk.r(0.375))
                .border(1.0, tk.border.alpha(tk.border.a * 0.7))
                .border_layout()
                .role("checkbox")
                .checked(on)
                .label(label.clone())
                .focusable(true)
                .on_press(move |a: &mut App| {
                    let default = submit::wallet_for_bull_network(a, id);
                    let f = &mut stm(a).form;
                    if f.bull_networks.contains(&id) {
                        f.bull_networks.retain(|n| *n != id);
                    } else {
                        f.bull_networks.push(id);
                        if f.bull_routes.get(id).is_none_or(|w| w.is_empty()) {
                            f.bull_routes.insert(id, default);
                        }
                    }
                    stm(a).last_import = None;
                })
                .child(checkbox_box(tk, on))
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(tk.r(0.25))
                        .child(text(label).font(tk.tsm(400.0)).color(tk.fg))
                        .child(
                            text(t(&format!("connections:add.bullWallet.{hk}")))
                                .font(tk.txs(400.0))
                                .color(tk.muted_fg)
                                .wrap(),
                        ),
                ),
        );
    }
    out.push(setup_field(
        tk,
        &t("connections:add.bullWallet.networks"),
        nets,
        None,
        Some(t(if existing {
            "connections:add.bullWallet.networksHelperMap"
        } else {
            "connections:add.bullWallet.networksHelperCreate"
        })),
    ));
    if existing {
        let routes = submit::bull_routes(app);
        let mut c = col()
            .w_full()
            .padding(tk.r(0.75))
            .gap(tk.r(0.75))
            .radius(tk.r(0.375))
            .border(1.0, tk.border.alpha(tk.border.a * 0.7))
            .border_layout()
            .child(
                col()
                    .child(
                        text(t("connections:add.bullWallet.walletMapping"))
                            .font(tk.tsm(500.0))
                            .color(tk.fg),
                    )
                    .child(
                        text(t("connections:add.bullWallet.walletMappingHelper"))
                            .font(tk.txs(400.0))
                            .color(tk.muted_fg)
                            .wrap(),
                    ),
            );
        if routes.is_empty() {
            c = c.child(
                row()
                    .w_full()
                    .padding(tk.r(0.75))
                    .radius(tk.r(0.375))
                    .border(1.0, tk.border.alpha(tk.border.a * 0.7))
                    .border_layout()
                    .child(
                        text(t("connections:add.bullWallet.selectToMap"))
                            .font(tk.tsm(400.0))
                            .color(tk.muted_fg),
                    ),
            );
        }
        let wo = wallet_options(app, true);
        for (route_chain, wallet) in routes {
            let none = wo.is_empty();
            let mut opts = vec![(
                String::new(),
                t(if none {
                    "connections:add.bullWallet.noWalletsYet"
                } else {
                    "connections:add.bullWallet.selectWallet"
                }),
                none,
            )];
            opts.extend(wo.clone());
            let key: &'static str = match route_chain {
                "bitcoin" => "connection-bull-route-bitcoin",
                "liquid" => "connection-bull-route-liquid",
                _ => "connection-bull-route-lightning",
            };
            let name = t("connections:add.bullWallet.kassiberWallet");
            let sel = native_select(
                app,
                tk,
                layer,
                key,
                &name,
                opts,
                &wallet,
                !none,
                move |a: &mut App, v: &str| {
                    stm(a).form.bull_routes.insert(route_chain, v.to_owned());
                },
            );
            let mut cap = route_chain.to_owned();
            if let Some(f) = cap.get_mut(0..1) {
                f.make_ascii_uppercase();
            }
            c = c.child(
                row()
                    .w_full()
                    .gap(tk.r(0.75))
                    .items_start()
                    .child(
                        col()
                            .grow(0.8)
                            .min_w(0.0)
                            .gap(tk.r(0.5))
                            .child(super::btcpay::form::label(
                                tk,
                                &t("connections:add.bullWallet.exportNetwork"),
                            ))
                            .child(
                                row()
                                    .w_full()
                                    .h(tk.r(2.25))
                                    .px(tk.r(0.75))
                                    .center_y()
                                    .radius(tk.r(0.375))
                                    .border(1.0, tk.border.alpha(tk.border.a * 0.7))
                                    .border_layout()
                                    .fill(tk.muted.alpha(0.4))
                                    .child(text(cap).font(tk.tsm(400.0)).color(tk.fg)),
                            ),
                    )
                    .child(
                        col()
                            .grow(1.2)
                            .min_w(0.0)
                            .child(setup_field(tk, &name, sel, None, None)),
                    ),
            );
        }
        out.push(c);
    }
    out.extend(source_file_setup(
        app,
        tk,
        Some(t("connections:add.bullWallet.refreshAfter")),
    ));
    out
}

fn enrichment_fields(app: &App, tk: &Tk, layer: &mut Vec<El>) -> Vec<El> {
    let src = selected(app);
    let m = meta(src.id);
    let f = &st(app).form;
    let mut out = Vec::new();
    if is_exchange_evidence(m.source_format) {
        let name = t("connections:add.enrichment.importMode");
        let sel = native_select(
            app,
            tk,
            layer,
            "connection-bull-import-mode",
            &name,
            vec![
                (
                    "relevant".into(),
                    t("connections:add.enrichment.relevantOnly"),
                    true,
                ),
                (
                    "full".into(),
                    t("connections:add.enrichment.fullImport"),
                    true,
                ),
            ],
            f.bull_import_mode,
            true,
            |a: &mut App, v: &str| {
                stm(a).form.bull_import_mode = if v == "full" { "full" } else { "relevant" };
                stm(a).last_import = None;
            },
        );
        let helper = tr(
            if f.bull_import_mode == "full" {
                "connections:add.enrichment.importModeHelperFull"
            } else {
                "connections:add.enrichment.importModeHelperRelevant"
            },
            &[("title", &src.title())],
        );
        out.push(setup_field(tk, &name, sel, None, Some(helper)));
        let fname = t("connections:add.enrichment.csvPath");
        let title = tr(
            "connections:add.field.selectExportFileTitle",
            &[("title", &src.title())],
        );
        let control = row()
            .w_full()
            .gap(tk.r(0.5))
            .child(col().grow(1.0).min_w(0.0).child(field_input(
                app,
                tk,
                "connection-source-file",
                "sourceFile",
                &fname,
                &st(app).form.source_file,
                |a| &mut stm(a).form.source_file,
                None,
                false,
            )))
            .child(browse(
                tk,
                "connection-source-browse",
                t("connections:add.field.browse"),
                title,
                file_filters(src.id),
                set_source_file,
            ));
        out.push(setup_field(
            tk,
            &fname,
            control,
            err(app, "sourceFile"),
            Some(tr(
                "connections:add.enrichment.csvPathHelper",
                &[("title", &src.title())],
            )),
        ));
        return out;
    }
    let name = t("connections:add.enrichment.walletToEnrich");
    let mut opts = vec![(
        String::new(),
        t("connections:add.enrichment.chooseWallet"),
        true,
    )];
    opts.extend(wallet_options(app, false));
    let sel = native_select(
        app,
        tk,
        layer,
        "connection-target-wallet",
        &name,
        opts,
        &f.target_wallet,
        true,
        |a: &mut App, v: &str| {
            stm(a).form.target_wallet = v.to_owned();
            stm(a).errors.remove("targetWallet");
            stm(a).last_import = None;
        },
    );
    out.push(setup_field(
        tk,
        &name,
        sel,
        err(app, "targetWallet"),
        Some(t("connections:add.enrichment.walletToEnrichHelper")),
    ));
    let fname = t("connections:add.exportFile.label");
    let title = tr(
        "connections:add.field.chooseExportTitle",
        &[("title", &src.title())],
    );
    let control = row()
        .w_full()
        .gap(tk.r(0.5))
        .child(col().grow(1.0).min_w(0.0).child(field_input(
            app,
            tk,
            "connection-source-file",
            "sourceFile",
            &fname,
            &st(app).form.source_file,
            |a| &mut stm(a).form.source_file,
            None,
            false,
        )))
        .child(browse(
            tk,
            "connection-source-browse",
            t("connections:add.field.browseDots"),
            title,
            file_filters(src.id),
            set_source_file,
        ));
    out.push(setup_field(
        tk,
        &fname,
        control,
        err(app, "sourceFile"),
        None,
    ));
    if let Some(p) = &st(app).bip329_preview {
        out.push(bip329_preview(tk, p));
    }
    out
}

/// The BIP329 preview block (rendered in the enrichment branch, as the web does).
fn bip329_preview(tk: &Tk, p: &Value) -> El {
    let n = |k: &str| {
        p.pointer(&format!("/counts/{k}"))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    let mut tiles = row().w_full().gap(tk.r(0.5));
    for (k, key) in [
        ("exact", "exact"),
        ("ambiguous", "ambiguous"),
        ("unmatched", "unmatched"),
        ("preserved", "preserved"),
    ] {
        tiles = tiles.child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .padding(tk.r(0.5))
                .radius(tk.r(0.375))
                .border(1.0, tk.border)
                .border_layout()
                .fill(tk.background)
                .child(
                    text(t(&format!("connections:add.bip329.preview.{key}")))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg),
                )
                .child(
                    text(n(k).to_string())
                        .font(tk.tlg(600.0).features("'tnum' 1"))
                        .color(tk.fg),
                ),
        );
    }
    let conflicts = n("conflicts") as usize;
    let dups = n("duplicate_refs") as usize;
    let badges = row()
        .w_full()
        .flex_wrap()
        .gap(tk.r(0.5))
        .child(badge(
            tk,
            &tc("connections:add.bip329.preview.conflicts", conflicts, &[]),
            if conflicts > 0 {
                Some(Tone::Danger)
            } else {
                None
            },
            false,
            false,
        ))
        .child(if dups > 0 {
            badge_secondary(
                tk,
                &tc("connections:add.bip329.preview.duplicates", dups, &[]),
            )
        } else {
            badge(
                tk,
                &tc("connections:add.bip329.preview.duplicates", dups, &[]),
                None,
                false,
                false,
            )
        })
        .child(badge(
            tk,
            &tc(
                "connections:add.bip329.preview.tagAdditions",
                n("tag_additions") as usize,
                &[],
            ),
            None,
            false,
            false,
        ))
        .child(badge(
            tk,
            &tc(
                "connections:add.bip329.preview.ambiguousSkipped",
                n("tag_skipped_ambiguous") as usize,
                &[],
            ),
            None,
            false,
            false,
        ));
    let mut c = col()
        .w_full()
        .padding(tk.r(0.75))
        .gap(tk.r(0.75))
        .radius(tk.r(0.375))
        .border(1.0, tk.border)
        .border_layout()
        .fill(tk.muted.alpha(0.3))
        .child(tiles)
        .child(badges);
    let rows: Vec<Value> = p
        .get("rows")
        .and_then(Value::as_array)
        .map(|a| a.iter().take(6).cloned().collect())
        .unwrap_or_default();
    if !rows.is_empty() {
        let mut list = col().w_full().gap(tk.r(0.5));
        for r in rows {
            let status = r
                .get("match_status")
                .and_then(Value::as_str)
                .unwrap_or("unmatched");
            let label = t(&format!(
                "connections:add.bip329.preview.{}",
                if ["exact", "ambiguous", "preserved"].contains(&status) {
                    status
                } else {
                    "unmatched"
                }
            ));
            let b = match status {
                "ambiguous" => badge_secondary(tk, &label),
                "unmatched" => crate::ui::tag(tk, tk.destructive, None, label.clone()),
                _ => badge(tk, &label, None, false, false),
            };
            let mut item = col()
                .w_full()
                .padding(tk.r(0.5))
                .gap(tk.r(0.25))
                .radius(tk.r(0.375))
                .border(1.0, tk.border)
                .border_layout()
                .fill(tk.background)
                .child(
                    row()
                        .w_full()
                        .gap(tk.r(0.25))
                        .center_y()
                        .child(
                            text(
                                r.get("type")
                                    .and_then(Value::as_str)
                                    .unwrap_or("")
                                    .to_uppercase(),
                            )
                            .font(tk.txs(500.0))
                            .color(tk.muted_fg)
                            .w(64.0),
                        )
                        .child(
                            text(
                                r.get("ref_preview")
                                    .and_then(Value::as_str)
                                    .unwrap_or("")
                                    .to_owned(),
                            )
                            .font(tk.txs(400.0).mono())
                            .color(tk.fg)
                            .truncate()
                            .grow(1.0)
                            .min_w(0.0),
                        )
                        .child(b),
                );
            let wallets: Vec<String> = r
                .get("wallets")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|w| w.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            if !wallets.is_empty() {
                item = item.child(
                    text(wallets.join(", "))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .truncate(),
                );
            }
            list = list.child(item);
        }
        c = c.child(list);
    }
    if let Some(w) = p
        .get("warnings")
        .and_then(Value::as_array)
        .filter(|w| !w.is_empty())
    {
        let s = w
            .iter()
            .take(2)
            .filter_map(|x| x.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        c = c.child(text(s).font(tk.txs(400.0)).color(tk.muted_fg).wrap());
    }
    c
}

fn bip329_fields(app: &App, tk: &Tk) -> Vec<El> {
    let name = t("connections:add.bip329.labelFilePath");
    let control = row()
        .w_full()
        .gap(tk.r(0.5))
        .child(col().grow(1.0).min_w(0.0).child(input(
            tk,
            "connection-bip329-file",
            &st(app).form.bip329_file,
            |a| &mut stm(a).form.bip329_file,
            &name,
            None,
            false,
            |a: &mut App| {
                stm(a).errors.remove("bip329File");
                stm(a).bip329_preview = None;
            },
        )))
        .child(browse(
            tk,
            "connection-bip329-browse",
            t("connections:add.field.browse"),
            t("connections:add.bip329.selectLabelFileTitle"),
            vec![(
                t("connections:add.bip329.labelFileFilter"),
                vec!["jsonl", "json"],
            )],
            |a: &mut App, p: String| {
                stm(a).form.bip329_file = TextEdit::new(&p);
                stm(a).errors.remove("bip329File");
                stm(a).bip329_preview = None;
            },
        ));
    vec![setup_field(
        tk,
        &name,
        control,
        err(app, "bip329File"),
        None,
    )]
}

/// `coreNodeStatusKey`.
fn core_status(s: Option<&str>) -> String {
    let k = s.unwrap_or("unknown");
    t(&format!("connections:add.core.status.{k}"))
}

fn na(v: &Value, k: &str) -> String {
    v.get(k)
        .and_then(Value::as_i64)
        .map(|n| n.to_string())
        .unwrap_or_else(|| "n/a".into())
}

/// `coreReadinessMessages`.
fn core_readiness(p: &Value) -> Vec<String> {
    let mut m = Vec::new();
    if p.get("pruned").and_then(Value::as_bool) == Some(true) {
        m.push(
            match p.get("earliest_retained_at").and_then(Value::as_str) {
                Some(r) => tr(
                    "connections:add.core.prunedWarningWithDate",
                    &[("retained", &r.chars().take(10).collect::<String>())],
                ),
                None => t("connections:add.core.prunedWarning"),
            },
        );
    }
    if p.get("ibd").and_then(Value::as_bool) == Some(true) {
        m.push(t("connections:add.core.initialBlockDownloadWarning"));
    }
    if p.pointer("/wallet_rpc/available").and_then(Value::as_bool) == Some(false) {
        m.push(
            p.pointer("/wallet_rpc/error/hint")
                .or(p.pointer("/wallet_rpc/error/message"))
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| t("connections:add.core.walletRpcUnavailable")),
        );
    }
    if p.pointer("/block_filters/available")
        .and_then(Value::as_bool)
        == Some(false)
    {
        m.push(
            p.pointer("/block_filters/error/hint")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| t("connections:add.core.blockFiltersUnavailable")),
        );
    }
    m
}

/// `coreCandidateAuthLabel`.
fn core_auth_label(c: &Value) -> String {
    let auth = c.get("auth_source").and_then(Value::as_str);
    let cred = c.get("credential_source").and_then(Value::as_str);
    let k = match (auth, cred) {
        (Some("basic"), Some("bitcoin.conf")) => "authBasicBitcoinConf",
        (Some("basic"), _) => "authBasic",
        (_, Some("bitcoin.conf")) => "authCookiefileBitcoinConf",
        (Some("cookiefile"), _) => "authCookiefile",
        _ => "credentialUnknown",
    };
    t(&format!("connections:add.core.{k}"))
}

fn core_fields(app: &App, tk: &Tk, layer: &mut Vec<El>) -> Vec<El> {
    let s = st(app);
    let f = &s.form;
    let mut out = vec![label_field(app, tk)];
    let detecting = s.core_detect_pending;
    let db = button(
        tk,
        "add-core-detect",
        Variant::Outline,
        true,
        if detecting {
            Some((&I::LOADER_CIRCLE, tk.r(1.0), true))
        } else {
            None
        },
        &t("connections:add.core.detect"),
        !detecting,
    );
    let mut r = row()
        .w_full()
        .gap(tk.r(0.5))
        .center_y()
        .flex_wrap()
        .child(if detecting {
            db
        } else {
            db.on_press(submit::detect_core)
        });
    let candidates = s.core_detection.clone().unwrap_or_default();
    if s.core_detection.is_some() && candidates.is_empty() {
        r = r.child(
            text(t("connections:add.core.noneDetected"))
                .font(tk.txs(400.0))
                .color(tk.muted_fg),
        );
    }
    out.push(r);
    if !candidates.is_empty() {
        let mut list = col().w_full().gap(tk.r(0.5));
        for (i, c) in candidates.iter().enumerate() {
            let meta_line = tr(
                "connections:add.core.candidateMeta",
                &[
                    (
                        "status",
                        &core_status(c.get("status").and_then(Value::as_str)),
                    ),
                    ("blocks", &na(c, "blocks")),
                    ("headers", &na(c, "headers")),
                    ("peers", &na(c, "peers")),
                    ("auth", &core_auth_label(c)),
                ],
            );
            let head = format!(
                "{} · {}",
                c.get("network")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("main"),
                c.get("url").and_then(Value::as_str).unwrap_or("")
            );
            let ready = core_readiness(c);
            let cc = c.clone();
            let mut inner = col()
                .min_w(0.0)
                .grow(1.0)
                .child(
                    text(head.clone())
                        .font(tk.tsm(500.0))
                        .color(tk.fg)
                        .truncate(),
                )
                .child(
                    text(meta_line)
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .truncate(),
                );
            if !ready.is_empty() {
                inner = inner.child(
                    text(ready.join(" "))
                        .font(tk.txs(400.0))
                        .color(amber_ink(tk))
                        .wrap(),
                );
            }
            let b = crate::txdetail::widgets::button_surface(
                tk,
                row()
                    .key(format!("add-core-candidate-{i}"))
                    .w_full()
                    .px(tk.r(1.0))
                    .py(tk.r(0.5))
                    .items_start(),
                crate::txdetail::widgets::Variant::Outline,
            )
            .role("button")
            .label(head)
            .focusable(true)
            .on_press(move |a: &mut App| {
                submit::apply_core_candidate(a, &cc);
                stm(a).core_test = None;
            })
            .child(inner);
            list = list.child(b);
        }
        out.push(
            col()
                .w_full()
                .padding(tk.r(0.75))
                .gap(tk.r(0.5))
                .radius(tk.r(0.375))
                .border(1.0, tk.border.alpha(tk.border.a * 0.7))
                .border_layout()
                .child(
                    text(t("connections:add.core.detectedNodes"))
                        .font(tk.txs(500.0))
                        .color(tk.muted_fg),
                )
                .child(list),
        );
    }
    let un = t("connections:add.core.url");
    let url = input(
        tk,
        "connection-core-url",
        &f.core_url,
        |a| &mut stm(a).form.core_url,
        &un,
        None,
        false,
        |a: &mut App| core_changed(a, "coreRpcUrl"),
    );
    let nn = t("connections:add.core.network");
    let nets = vec![
        ("main".into(), t("connections:add.core.networkMain"), true),
        ("test".into(), t("connections:add.core.networkTest"), true),
        (
            "signet".into(),
            t("connections:add.core.networkSignet"),
            true,
        ),
        (
            "regtest".into(),
            t("connections:add.core.networkRegtest"),
            true,
        ),
    ];
    let network_select = native_select(
        app,
        tk,
        layer,
        "connection-core-network",
        &nn,
        nets,
        &f.core_network,
        true,
        |a: &mut App, v: &str| {
            let f = &mut stm(a).form;
            let cur = f.core_url.text.trim().to_owned();
            if super::CORE_DEFAULT_RPC_URLS.iter().any(|(_, u)| *u == cur) {
                f.core_url = TextEdit::new(core_default_url(v));
            }
            f.core_network = v.to_owned();
            f.core_credential_ref.clear();
            stm(a).core_test = None;
        },
    );
    out.push(
        row()
            .w_full()
            .gap(tk.r(0.75))
            .items_start()
            .child(col().grow(1.4).min_w(0.0).child(setup_field(
                tk,
                &un,
                url,
                err(app, "coreRpcUrl"),
                None,
            )))
            .child(col().grow(0.7).min_w(0.0).child(setup_field(
                tk,
                &nn,
                network_select,
                None,
                None,
            ))),
    );
    let an = t("connections:add.core.authMode");
    let auth = native_select(
        app,
        tk,
        layer,
        "connection-core-auth",
        &an,
        vec![
            (
                "cookiefile".into(),
                t("connections:add.core.authCookiefile"),
                true,
            ),
            ("basic".into(), t("connections:add.core.authBasic"), true),
        ],
        f.core_auth,
        true,
        |a: &mut App, v: &str| {
            stm(a).form.core_auth = if v == "basic" { "basic" } else { "cookiefile" };
            core_changed(a, "coreRpcAuthMode");
        },
    );
    out.push(setup_field(tk, &an, auth, None, None));
    if f.core_auth == "cookiefile" {
        let cn = t("connections:add.core.cookiefile");
        out.push(setup_field(
            tk,
            &cn,
            input(
                tk,
                "connection-core-cookiefile",
                &f.core_cookiefile,
                |a| &mut stm(a).form.core_cookiefile,
                &cn,
                None,
                false,
                |a: &mut App| core_changed(a, "coreRpcCookiefile"),
            ),
            err(app, "coreRpcCookiefile"),
            None,
        ));
    } else {
        let usn = t("connections:add.core.username");
        let pwn = t("connections:add.core.password");
        out.push(grid2(
            tk,
            setup_field(
                tk,
                &usn,
                input(
                    tk,
                    "connection-core-username",
                    &f.core_user,
                    |a| &mut stm(a).form.core_user,
                    &usn,
                    None,
                    false,
                    |a: &mut App| core_changed(a, "coreRpcUsername"),
                ),
                err(app, "coreRpcUsername"),
                None,
            ),
            setup_field(
                tk,
                &pwn,
                input(
                    tk,
                    "connection-core-password",
                    &f.core_pass,
                    |a| &mut stm(a).form.core_pass,
                    &pwn,
                    None,
                    true,
                    |a: &mut App| core_changed(a, "coreRpcPassword"),
                ),
                err(app, "coreRpcPassword"),
                None,
            ),
        ));
    }
    let testing = s.core_test_pending;
    let enabled = !testing && !f.core_url.text.trim().is_empty();
    let tb = button(
        tk,
        "add-core-test",
        Variant::Outline,
        true,
        if testing {
            Some((&I::LOADER_CIRCLE, tk.r(1.0), true))
        } else {
            None
        },
        &t("connections:add.core.test"),
        enabled,
    );
    let mut tr_row = row()
        .w_full()
        .gap(tk.r(0.5))
        .center_y()
        .flex_wrap()
        .child(if enabled {
            tb.on_press(submit::test_core)
        } else {
            tb
        });
    match &s.core_test {
        Some(CoreTest::Ok(p)) => {
            let tested_network = p
                .get("network")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| f.core_network.clone());
            let mut c = col().gap(tk.r(0.25)).child(xs(
                tk,
                tr(
                    "connections:add.core.testOkDetailed",
                    &[
                        ("network", &tested_network),
                        (
                            "status",
                            &core_status(p.get("status").and_then(Value::as_str)),
                        ),
                        ("blocks", &na(p, "blocks")),
                        ("headers", &na(p, "headers")),
                        ("peers", &na(p, "peers")),
                    ],
                ),
                emerald_ink(tk),
            ));
            if p.get("pruned").and_then(Value::as_bool) == Some(true)
                || p.get("ibd").and_then(Value::as_bool) == Some(true)
            {
                for m in core_readiness(
                    &serde_json::json!({"pruned": p.get("pruned"), "earliest_retained_at": p.get("earliest_retained_at"), "ibd": p.get("ibd")}),
                ) {
                    c = c.child(xs(tk, m, amber_ink(tk)));
                }
            }
            if p.pointer("/wallet_rpc/available").and_then(Value::as_bool) == Some(false) {
                c = c.child(xs(
                    tk,
                    p.pointer("/wallet_rpc/error/hint")
                        .or(p.pointer("/wallet_rpc/error/message"))
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .unwrap_or_else(|| t("connections:add.core.walletRpcUnavailable")),
                    tk.destructive,
                ));
            }
            if p.pointer("/block_filters/available")
                .and_then(Value::as_bool)
                == Some(false)
            {
                c = c.child(xs(
                    tk,
                    p.pointer("/block_filters/error/hint")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .unwrap_or_else(|| t("connections:add.core.blockFiltersUnavailable")),
                    tk.muted_fg,
                ));
            }
            tr_row = tr_row.child(c);
        }
        Some(CoreTest::Failed { message, hint }) => {
            tr_row = tr_row.child(
                text(hint.clone().unwrap_or_else(|| message.clone()))
                    .font(tk.txs(400.0))
                    .color(tk.destructive),
            )
        }
        None => {}
    }
    out.push(tr_row);
    out
}

/// `updateCoreForm`: a credential edit drops the detected credential and the last test.
fn core_changed(a: &mut App, key: &'static str) {
    let s = stm(a);
    s.form.core_credential_ref.clear();
    s.core_test = None;
    s.errors.remove(key);
}

fn backend_settings_fields(app: &App, tk: &Tk) -> Vec<El> {
    let mut c = col()
        .w_full()
        .padding(tk.r(0.75))
        .gap(tk.r(0.75))
        .radius(tk.r(0.375))
        .border(1.0, tk.border)
        .border_layout()
        .fill(tk.background)
        .child(
            text(t("connections:add.backendSettings.line1"))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .wrap(),
        )
        .child(
            text(t("connections:add.backendSettings.line2"))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .wrap(),
        );
    match app.read("ui.backends.settings.list") {
        None => {
            c = c.child(
                text(t("connections:add.backendSettings.loading"))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg),
            )
        }
        Some(d) => {
            let rows = backend_rows::regtest_backend_connections(d);
            if !rows.is_empty() {
                let mut list = col().w_full().gap(tk.r(0.375));
                for b in rows {
                    let hash = b.settings_hash.clone();
                    list = list.child(
                        row()
                            .key(format!("add-local-backend-{}", b.id))
                            .w_full()
                            .min_w(0.0)
                            .px(tk.r(0.75))
                            .py(tk.r(0.5))
                            .gap(tk.r(0.75))
                            .center_y()
                            .justify_between()
                            .radius(tk.r(0.375))
                            .border(1.0, tk.border)
                            .border_layout()
                            .fill(tk.muted.alpha(0.3))
                            .hover(tk.muted.alpha(0.55))
                            .role("button")
                            .label(b.label.clone())
                            .focusable(true)
                            .on_press(move |a: &mut App| {
                                submit::open_configured_backend(a, hash.clone())
                            })
                            .child(
                                col()
                                    .min_w(0.0)
                                    .grow(1.0)
                                    .child(
                                        text(b.label.clone())
                                            .font(tk.tsm(500.0))
                                            .color(tk.fg)
                                            .truncate(),
                                    )
                                    .child(
                                        text(endpoint_text(&b.endpoint))
                                            .font(tk.txs(400.0).mono())
                                            .color(tk.muted_fg)
                                            .truncate(),
                                    ),
                            )
                            .child(
                                text(b.sync_source.clone())
                                    .font(tk.txs(400.0))
                                    .color(tk.muted_fg),
                            ),
                    );
                }
                c = c.child(
                    col()
                        .w_full()
                        .gap(tk.r(0.5))
                        .child(
                            text(t("connections:add.backendSettings.localTitle"))
                                .font(tk.txs(500.0))
                                .color(tk.fg),
                        )
                        .child(list),
                );
            }
        }
    }
    vec![c]
}

/// `renderSetupFields`.
fn fields(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> Vec<El> {
    let src = selected(app);
    let m = meta(src.id);
    match m.kind() {
        SetupKind::Descriptor => descriptor_fields(app, tk, layer),
        SetupKind::AddressList => address_list_fields(app, tk, layer),
        SetupKind::SilentPayment => silent_payment_fields(app, tk, layer),
        SetupKind::Samourai => samourai_fields(app, tk, layer),
        SetupKind::FileWallet => file_wallet_fields(app, tk, layer),
        SetupKind::PriorReport => prior_report_fields(app, tk),
        SetupKind::BullbitcoinWallet => bull_wallet_fields(app, tk, layer),
        SetupKind::FileEnrichment => enrichment_fields(app, tk, layer),
        SetupKind::ExchangeApi if m.exchange.is_some() => {
            let ex = m.exchange.unwrap_or("");
            let provider = super::exchange::provider_name(ex);
            let footer = sync_after(
                app,
                tk,
                &tr("connections:exchange.syncAfter", &[("provider", provider)]),
            );
            vec![super::exchange::panel(
                app,
                cx,
                tk,
                layer,
                ex,
                label_field(app, tk),
                footer,
            )]
        }
        SetupKind::Btcpay => {
            let footer = sync_after(app, tk, &t("connections:add.btcpay.refreshAfter"));
            vec![super::btcpay::panel(
                app,
                cx,
                tk,
                layer,
                label_field(app, tk),
                footer,
            )]
        }
        SetupKind::Bip329 => bip329_fields(app, tk),
        SetupKind::BackendSettings if src.id == "bitcoin-core" => core_fields(app, tk, layer),
        SetupKind::BackendSettings => backend_settings_fields(app, tk),
        _ => vec![plain_box(
            tk,
            0.0,
            vec![
                text(t("connections:add.notWired"))
                    .font(tk.tsm(400.0))
                    .color(tk.muted_fg),
            ],
        )],
    }
}

/// `renderImportSummary`.
fn import_summary(tk: &Tk, r: &Value) -> El {
    let n = |k: &str| r.get(k).and_then(Value::as_u64);
    let imported = n("imported").unwrap_or(0);
    let skipped = n("skipped").unwrap_or(0);
    let rows_read = [
        "bullbitcoin_rows",
        "bullbitcoin_wallet_rows",
        "coinfinity_rows",
        "twentyonebitcoin_rows",
        "strike_rows",
        "binance_rows",
        "bisq_rows",
        "wasabi_transactions",
    ]
    .iter()
    .find_map(|k| n(k))
    .unwrap_or(imported + skipped);
    let book = r.get("scope").and_then(Value::as_str) == Some("book");
    let changed = imported + n("updated").unwrap_or(0);
    let mut metrics: Vec<(String, Option<u64>)> = vec![
        (
            t("connections:add.summary.metric.rowsRead"),
            Some(rows_read),
        ),
        (t("connections:add.summary.metric.inserted"), Some(imported)),
        (t("connections:add.summary.metric.updated"), n("updated")),
        (
            t("connections:add.summary.metric.unchanged"),
            n("unchanged"),
        ),
    ];
    for (k, key) in [
        ("matched", "matched"),
        ("wasabi_coins_observed", "coinsObserved"),
        ("wasabi_coins_active", "activeCoins"),
        ("wasabi_payments_in_coinjoin", "paymentsInCoinjoin"),
        ("skipped_unmatched", "unmatched"),
        ("unmatched", "unmatched"),
        ("skipped_ambiguous", "ambiguous"),
        ("ambiguous", "ambiguous"),
        ("excluded", "excluded"),
    ] {
        if r.get(k).is_some() {
            metrics.push((t(&format!("connections:add.summary.metric.{key}")), n(k)));
        }
    }
    let fc = |v: Option<u64>| crate::format::count(v.unwrap_or(0) as usize);
    let mut grid = col().w_full().gap(tk.r(0.5));
    for chunk in metrics.chunks(4) {
        let mut rr = row().w_full().gap(tk.r(0.5));
        for (label, v) in chunk {
            rr = rr.child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .padding(tk.r(0.5))
                    .radius(tk.r(0.375))
                    .border(1.0, tk.border.alpha(tk.border.a * 0.6))
                    .border_layout()
                    .child(text(label.clone()).font(tk.txs(400.0)).color(tk.muted_fg))
                    .child(
                        text(fc(*v))
                            .font(tk.tsm(500.0).features("'tnum' 1"))
                            .color(tk.fg),
                    ),
            );
        }
        for _ in chunk.len()..4 {
            rr = rr.child(col().grow(1.0).min_w(0.0));
        }
        grid = grid.child(rr);
    }
    let recs: Vec<Value> = ["inserted_records", "updated_records"]
        .iter()
        .flat_map(|k| {
            r.get(*k)
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .collect();
    let shown: Vec<&Value> = recs.iter().take(5).collect();
    let hidden = recs.len().saturating_sub(shown.len());
    let records = if shown.is_empty() {
        row()
            .w_full()
            .padding(tk.r(0.625))
            .radius(tk.r(0.375))
            .border(1.0, tk.border.alpha(tk.border.a * 0.6))
            .border_layout()
            .child(
                text(t("connections:add.summary.noChange"))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg)
                    .wrap(),
            )
    } else {
        let mut c = col()
            .w_full()
            .radius(tk.r(0.375))
            .border(1.0, tk.border.alpha(tk.border.a * 0.6))
            .border_layout();
        let total = shown.len();
        for (i, rec) in shown.into_iter().enumerate() {
            let s = |k: &str| {
                rec.get(k)
                    .and_then(Value::as_str)
                    .filter(|x| !x.is_empty())
                    .map(str::to_owned)
            };
            let head = format!(
                "{} · {}",
                s("wallet").unwrap_or_else(|| t("connections:add.summary.recordBook")),
                s("external_id")
                    .or_else(|| s("transaction_id"))
                    .unwrap_or_default()
            );
            let right = s("status")
                .or_else(|| s("pricing_external_ref"))
                .unwrap_or_else(|| t("connections:add.summary.recordBullOrder"));
            let fields: Vec<String> = rec
                .get("changed_fields")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            let detail = if let Some(w) = s("matched_wallet") {
                tr(
                    "connections:add.summary.matchedWalletRecord",
                    &[("wallet", &w)],
                )
            } else if !fields.is_empty() {
                tr(
                    "connections:add.summary.changedFields",
                    &[("fields", &fields.join(", "))],
                )
            } else {
                t("connections:add.summary.changedMetadata")
            };
            let mut item = col()
                .w_full()
                .padding(tk.r(0.625))
                .gap(tk.r(0.25))
                .child(
                    row()
                        .w_full()
                        .gap(tk.r(0.5))
                        .justify_between()
                        .child(
                            text(head)
                                .font(tk.txs(500.0))
                                .color(tk.fg)
                                .truncate()
                                .min_w(0.0),
                        )
                        .child(text(right).font(tk.txs(400.0)).color(tk.muted_fg)),
                )
                .child(text(detail).font(tk.txs(400.0)).color(tk.muted_fg).wrap());
            if i + 1 < total || hidden > 0 {
                item = item
                    .border_sides([0.0, 0.0, 1.0, 0.0], tk.border.alpha(tk.border.a * 0.4))
                    .border_layout();
            }
            c = c.child(item);
        }
        if hidden > 0 {
            c = c.child(
                row().w_full().padding(tk.r(0.625)).child(
                    text(tc("connections:add.summary.moreUpdated", hidden, &[]))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg),
                ),
            );
        }
        c
    };
    col()
        .w_full()
        .padding(tk.r(0.75))
        .gap(tk.r(0.75))
        .radius(tk.r(0.375))
        .border(1.0, tk.border)
        .border_layout()
        .fill(tk.background)
        .child(
            row()
                .w_full()
                .gap(tk.r(0.5))
                .items_start()
                .justify_between()
                .child(
                    col()
                        .child(
                            text(t("connections:add.summary.title"))
                                .font(tk.tsm(500.0))
                                .color(tk.fg),
                        )
                        .child(
                            text(t(if book {
                                "connections:add.summary.matchedBook"
                            } else {
                                "connections:add.summary.matchedWallet"
                            }))
                            .font(tk.txs(400.0))
                            .color(tk.muted_fg),
                        ),
                )
                .child(badge_secondary(
                    tk,
                    &tr(
                        "connections:add.summary.changed",
                        &[("value", &crate::format::count(changed as usize))],
                    ),
                )),
        )
        .child(grid)
        .children(bisq_held_block(tk, r))
        .child(records)
}

/// `border-amber-500/40`: the trades a Bisq import held for review instead of booking.
fn bisq_held_block(tk: &Tk, r: &Value) -> Option<El> {
    let lines = super::bisq::held_lines(r);
    if lines.is_empty() {
        return None;
    }
    let amber = rune::color::hex("#f59e0b").expect("hex");
    let mut c = col()
        .key("connection-bisq-held")
        .w_full()
        .gap(tk.r(0.25))
        .padding(tk.r(0.625))
        .radius(tk.r(0.375))
        .border(1.0, amber.alpha(0.4))
        .border_layout()
        .child(
            text(tr(
                "connections:add.summary.bisqHeld",
                &[("value", &crate::format::count(lines.len()))],
            ))
            .font(tk.txs(500.0))
            .color(tk.fg)
            .wrap(),
        );
    for line in lines {
        c = c.child(text(line).font(tk.txs(400.0)).color(tk.muted_fg).wrap());
    }
    Some(c)
}

/// The setup step's body: summary rail and form.
pub fn body(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    let src = selected(app);
    let wide = meta(src.id).kind() == SetupKind::Btcpay;
    let rail_w = if wide { 240.0 } else { 310.0 };
    let mut form = col()
        .key("connection-setup-form")
        .w_full()
        .max_w(tk.r(if wide { 48.0 } else { 36.0 }))
        .gap(tk.r(1.0))
        .role("form")
        .label(tr("connections:add.setupTitle", &[("title", &src.title())]));
    for f in fields(app, cx, tk, layer) {
        form = form.child(f);
    }
    let s = st(app);
    if let Some((wallet, processed, total)) = &s.progress {
        let pct = if *total > 0 {
            (*processed as f32 / *total as f32).min(1.0)
        } else {
            0.0
        };
        form = form.child(plain_box(
            tk,
            tk.r(0.25),
            vec![
                row()
                    .w_full()
                    .justify_between()
                    .child(
                        text(tr(
                            "connections:add.progress.importing",
                            &[("wallet", wallet)],
                        ))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg),
                    )
                    .child(
                        text(tr(
                            "connections:add.progress.rows",
                            &[
                                ("processed", &crate::format::count(*processed as usize)),
                                ("total", &crate::format::count(*total as usize)),
                            ],
                        ))
                        .font(tk.txs(500.0).features("'tnum' 1"))
                        .color(tk.fg),
                    ),
                row()
                    .w_full()
                    .h(tk.r(0.375))
                    .radius(tk.r(0.1875))
                    .fill(tk.muted)
                    .clip_rounded(tk.r(0.1875))
                    .child(el().grow(pct.max(0.0001)).h_full().fill(tk.primary))
                    .child(el().grow((1.0 - pct).max(0.0001)).h_full()),
            ],
        ));
    }
    if let Some(e) = &s.setup_error {
        form = form.child(
            row()
                .w_full()
                .padding(tk.r(0.75))
                .radius(tk.r(0.375))
                .border(1.0, tk.destructive.alpha(0.3))
                .border_layout()
                .fill(tk.destructive.alpha(0.1))
                .child(
                    text(e.clone())
                        .font(tk.tsm(400.0))
                        .color(tk.destructive)
                        .wrap(),
                )
                .role("alert"),
        );
    }
    if let Some(r) = &s.last_import {
        form = form.child(import_summary(tk, r));
    }
    let _ = notice(tk, Tone::Neutral, "");
    row()
        .w_full()
        .h_full()
        .child(
            col()
                .key("add-setup-summary")
                .w(rail_w)
                .h_full()
                .border_sides([0.0, 1.0, 0.0, 0.0], tk.border)
                .border_layout()
                .scroll(rune::ScrollSpec {
                    stick_bottom: false,
                })
                .child(
                    col()
                        .w_full()
                        .padding(tk.r(1.25))
                        .child(super::picker::summary_of(app, cx, tk, src)),
                ),
        )
        .child(
            col()
                .key("add-setup-scroll")
                .grow(1.0)
                .min_w(0.0)
                .h_full()
                .scroll(rune::ScrollSpec {
                    stick_bottom: false,
                })
                .child(
                    col()
                        .w_full()
                        .padding(tk.r(1.25))
                        .items_center()
                        .child(form),
                ),
        )
}

/// Selects mount their menus while the body is built; nothing else to add.
pub fn selects(_app: &App, _tk: &Tk, _layer: &mut Vec<El>) {}

#[allow(dead_code)]
fn _style(_: TextStyle) {}
