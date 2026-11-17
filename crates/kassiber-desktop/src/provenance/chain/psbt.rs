//! `PsbtPanel` (Tools → Inspect a PSBT file): local inspection of a staged PSBT, an optional
//! proposal comparison with explicit Payjoin constraints, and entropy over either side.
//!
//! The file is chosen in the native dialog and staged by the daemon
//! (`internal.chain_analysis.stage`, as the Tauri command does); the UI only holds its opaque
//! token. Nothing is signed or broadcast.

use rune::{El, Tx, col, row, text};
use serde_json::{Value, json};

use super::kit::{self, field, input, val};
use super::model::{self, Query};
use super::results::{self, Entropy, Which};
use super::{Bind, report, st};
use crate::app::{App, Reply};
use crate::i18n::{t, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::txdetail::widgets::Variant;
use crate::ui::ic;

#[derive(Default)]
pub struct Psbt {
    pub before: Option<Value>,
    pub after: Option<Value>,
    pub network: Option<String>,
    pub payjoin: bool,
    pub substitute: bool,
    /// (input key, analysis or comparison, is a comparison)
    pub outcome: Option<(String, Value, bool)>,
    pub busy: bool,
    pub entropy_side: Option<&'static str>,
    pub entropy_job: Option<(String, String, String)>,
    pub entropy: Entropy,
    pub entropy_for: Option<String>,
    pub seeded: bool,
}

/// The book's bound Bitcoin network, if it is bound.
pub fn book_network(binding: &Value) -> Option<String> {
    binding
        .get("domains")
        .and_then(Value::as_array)
        .and_then(|d| {
            d.iter()
                .find(|x| x.get("chain").and_then(Value::as_str) == Some("bitcoin"))
        })
        .and_then(|d| d.get("network"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// `bookEvidenceNetwork`: the one environment the wallets show, else mainnet.
fn evidence_network(inventory: Option<&Value>) -> String {
    let mut seen: Vec<String> = vec![];
    for w in inventory
        .and_then(|i| i.get("wallets"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        for e in w
            .get("environments")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
        {
            if let Some(s) = e.as_str()
                && !seen.iter().any(|x| x == s)
            {
                seen.push(s.to_owned());
            }
        }
    }
    if seen.len() == 1 && ["main", "test", "signet", "regtest"].contains(&seen[0].as_str()) {
        seen.remove(0)
    } else {
        "main".into()
    }
}

fn input_key(app: &App) -> String {
    let p = &app.provenance.chain.psbt;
    let tok = |s: &Option<Value>| s.as_ref().and_then(|v| v.get("source_token")).cloned();
    json!([
        tok(&p.before),
        tok(&p.after),
        p.network,
        p.payjoin,
        val(app, "pj.payment"),
        val(app, "pj.fee"),
        val(app, "pj.max"),
        val(app, "pj.rate"),
        p.substitute
    ])
    .to_string()
}

fn constraints(app: &App) -> Option<Value> {
    model::payjoin_options(
        &val(app, "pj.payment"),
        &val(app, "pj.fee"),
        &val(app, "pj.max"),
        &val(app, "pj.rate"),
        app.provenance.chain.psbt.substitute,
    )
}

/// `psbtAssistantContext`: opaque grants and categorical context only.
fn assistant_input(app: &App) -> Option<Value> {
    let p = &app.provenance.chain.psbt;
    let before = p.before.as_ref()?.get("source_token")?.as_str()?.to_owned();
    let after = p
        .after
        .as_ref()
        .and_then(|a| a.get("source_token"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let c = constraints(app);
    if p.payjoin && c.is_none() {
        return None;
    }
    let network = p.network.clone()?;
    let key = input_key(app);
    let mut v = json!({"before_token": before, "network": network});
    if let Some(a) = &after {
        v["after_token"] = json!(a);
    }
    if p.payjoin {
        v["payjoin"] = c.unwrap_or(Value::Null);
    }
    v["current_comparison_available"] = json!(
        p.outcome
            .as_ref()
            .is_some_and(|(k, _, cmp)| *cmp && *k == key)
    );
    if let Some((job, token, job_network)) = &p.entropy_job
        && *job_network == network
        && (Some(token) == Some(&before) || after.as_ref() == Some(token))
    {
        v["entropy_job_id"] = json!(job);
        v["entropy_source_token"] = json!(token);
    }
    Some(v)
}

fn pick(app: &mut App, tx: &mut Tx, side: &'static str) {
    if app.read("ui.profiles.snapshot").is_none() {
        return;
    }
    let options = rune::FileDialogOptions::default().filter("Analysis source", &["psbt", "txt"]);
    tx.open_file(options, move |a: &mut App, result, _tx| match result {
        Ok(Some(path)) => {
            let args = json!({"source_file": path.to_string_lossy(), "purpose": "psbt", "expected_scope": a.expected_scope()});
            let tag = if side == "before" { "provenance.chain.stage.before" } else { "provenance.chain.stage.after" };
            a.send("internal.chain_analysis.stage", args, tag);
        }
        Ok(None) => {}
        Err(e) => report(a, e.0),
    });
}

pub fn view(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let s = &app.provenance.chain;
    let small = tk.tsm(400.0);
    let book = s.binding.ready().and_then(book_network);
    match &s.binding {
        Bind::Idle | Bind::Loading => {
            return text(t("common:state.loading")).font(small).color(tk.fg);
        }
        Bind::Failed(_) => {
            return row().child(
                kit::btn(
                    tk,
                    "psbt-retry",
                    Variant::Default,
                    None,
                    &t("common:actions.retry"),
                    true,
                )
                .on_press(|a: &mut App| {
                    st(a).binding = Bind::Idle;
                    super::load_binding(a);
                }),
            );
        }
        Bind::Ready(_) => {}
    }
    if book.is_none() && s.inventory.is_none() {
        return text(t("common:state.loading")).font(small).color(tk.fg);
    }
    if let (Some(b), Some(i)) = (&book, &s.initial_network)
        && b != i
    {
        return text(t("settings:bookNetwork.inspectionMismatch"))
            .font(small)
            .color(tk.fg)
            .role("alert");
    }
    let p = &s.psbt;
    let network = p.network.clone().unwrap_or_else(|| "main".into());
    let busy = p.busy;
    let ai = assistant_input(app);
    let can_ask = ai.is_some() && app.prefs.ai_features && !app.assistant.streaming;
    let ask = kit::btn(
        tk,
        "psbt-ask",
        Variant::Outline,
        Some(&I::SPARKLES),
        &t("chainAnalysis:ask"),
        can_ask,
    );
    let ask = if can_ask {
        ask.on_press(|a: &mut App| {
            if let Some(v) = assistant_input(a) {
                let prompt = tr(
                    "chainAnalysis:psbt.assistantPrompt",
                    &[("selection", &v.to_string())],
                );
                crate::assistant::hand_off(a, &prompt);
            }
        })
    } else {
        ask
    };
    let mut c = crate::ui::surface(tk)
        .w_full()
        .padding(tk.card_pad())
        .gap(20.0)
        .child(
            row()
                .w_full()
                .gap(12.0)
                .items_start()
                .justify_between()
                .child(
                    col()
                        .min_w(0.0)
                        .grow(1.0)
                        .gap(8.0)
                        .child(
                            row()
                                .gap(8.0)
                                .center_y()
                                .child(ic(&I::FILE_SEARCH, 20.0, tk.fg))
                                .child(
                                    text(t("chainAnalysis:psbt.title"))
                                        .font(tk.tlg(600.0))
                                        .color(tk.fg)
                                        .role("heading")
                                        .level(2),
                                ),
                        )
                        .child(
                            text(t("chainAnalysis:psbt.help"))
                                .font(small)
                                .color(tk.muted_fg)
                                .wrap(),
                        ),
                )
                .child(ask),
        );
    let mut sides = row().w_full().gap(16.0);
    for side in ["before", "after"] {
        let source = if side == "before" {
            &p.before
        } else {
            &p.after
        };
        let info = source
            .as_ref()
            .map(|v| {
                format!(
                    "{} · {} B",
                    v.get("filename").and_then(Value::as_str).unwrap_or(""),
                    v.get("size_bytes")
                        .map(Value::to_string)
                        .unwrap_or_default()
                )
            })
            .unwrap_or_else(|| t("chainAnalysis:psbt.noFile"));
        let choose = kit::btn(
            tk,
            format!("psbt-pick-{side}"),
            Variant::Outline,
            None,
            &t("chainAnalysis:workbench.chooseFile"),
            !busy,
        );
        let mut buttons = row().gap(8.0).child(if busy {
            choose
        } else {
            choose.on_press_tx(move |a: &mut App, tx: &mut Tx| pick(a, tx, side))
        });
        if source.is_some() {
            let clear = kit::btn(
                tk,
                format!("psbt-clear-{side}"),
                Variant::Ghost,
                None,
                &t("chainAnalysis:workbench.clearSelection"),
                !busy,
            );
            buttons = buttons.child(if busy {
                clear
            } else {
                clear.on_press(move |a: &mut App| {
                    let p = &mut st(a).psbt;
                    if side == "before" {
                        p.before = None
                    } else {
                        p.after = None
                    }
                    p.entropy_side = None;
                    p.entropy_job = None;
                })
            });
        }
        sides = sides.child(
            dashed_box(tk)
                .w(0.0)
                .grow(1.0)
                .child(
                    text(t(if side == "before" {
                        "chainAnalysis:psbt.original"
                    } else {
                        "chainAnalysis:psbt.proposal"
                    }))
                    .font(tk.tsm(500.0))
                    .color(tk.fg),
                )
                .child(
                    text(info)
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .wrap()
                        .my(12.0),
                )
                .child(buttons),
        );
    }
    c = c.child(sides);
    c = c.child(
        text(tr(
            "settings:analysisNetwork.effective",
            &[("network", &network)],
        ))
        .font(tk.txs(400.0))
        .color(tk.muted_fg)
        .help(t("chainAnalysis:psbt.networkHelp")),
    );
    // Payjoin checks.
    let pj_open = kit::is_open(app, "psbt-payjoin");
    let mut pj = col()
        .w_full()
        .radius(tk.r(0.5))
        .border(1.0, tk.border)
        .border_layout()
        .child(kit::summary(
            tk,
            app,
            "psbt-payjoin",
            text(t("chainAnalysis:psbt.payjoin"))
                .font(tk.tsm(500.0))
                .color(tk.fg),
            [16.0, 16.0],
        ));
    if pj_open {
        let mut body = col().w_full().px(16.0).pb(16.0).gap(12.0).child(
            row()
                .gap(8.0)
                .center_y()
                .child(crate::settings::controls::checkbox(
                    tk,
                    "psbt-pj-on",
                    p.payjoin,
                    &t("chainAnalysis:psbt.enablePayjoin"),
                    |a: &mut App| {
                        let p = &mut st(a).psbt;
                        p.payjoin = !p.payjoin;
                    },
                ))
                .child(
                    text(t("chainAnalysis:psbt.enablePayjoin"))
                        .font(tk.txs(400.0))
                        .color(tk.fg),
                ),
        );
        if p.payjoin {
            let f = |k: &str, key: &str| {
                field(
                    tk,
                    &t(&format!("chainAnalysis:psbt.{k}")),
                    input(
                        tk,
                        app,
                        key,
                        &t(&format!("chainAnalysis:psbt.{k}")),
                        "",
                        false,
                        32.0,
                        true,
                        None,
                    ),
                )
                .w(0.0)
                .grow(1.0)
            };
            body = body
                .child(
                    row()
                        .w_full()
                        .gap(12.0)
                        .child(f("paymentIndex", "pj.payment"))
                        .child(f("feeIndex", "pj.fee"))
                        .child(f("maximumFee", "pj.max"))
                        .child(f("minimumRate", "pj.rate")),
                )
                .child(
                    row()
                        .gap(8.0)
                        .center_y()
                        .child(crate::settings::controls::checkbox(
                            tk,
                            "psbt-pj-sub",
                            p.substitute,
                            &t("chainAnalysis:psbt.substitution"),
                            |a: &mut App| {
                                let p = &mut st(a).psbt;
                                p.substitute = !p.substitute;
                            },
                        ))
                        .child(
                            text(t("chainAnalysis:psbt.substitution"))
                                .font(tk.txs(400.0))
                                .color(tk.fg),
                        ),
                );
        }
        pj = pj.child(body);
    }
    c = c.child(pj);
    let key = input_key(app);
    let valid_pj = !p.payjoin || constraints(app).is_some();
    let can_analyze = p.before.is_some() && !busy;
    let can_compare = p.before.is_some() && p.after.is_some() && !busy && valid_pj;
    let analyze = kit::btn(
        tk,
        "psbt-analyze",
        Variant::Default,
        None,
        &t("chainAnalysis:psbt.analyze"),
        can_analyze,
    );
    let compare = kit::btn(
        tk,
        "psbt-compare",
        Variant::Outline,
        None,
        &t("chainAnalysis:psbt.compare"),
        can_compare,
    );
    let k1 = key.clone();
    let k2 = key.clone();
    c = c.child(
        row()
            .gap(8.0)
            .child(if can_analyze {
                analyze.on_press(move |a: &mut App| {
                    let p = &a.provenance.chain.psbt;
                    let token = p.before.as_ref().and_then(|b| b.get("source_token")).cloned();
                    let args = json!({"psbt_token": token, "network": p.network});
                    if a.send("ui.chain_analysis.psbt.analyze", args, "provenance.chain.psbt.analyze").is_some() {
                        let p = &mut st(a).psbt;
                        p.busy = true;
                        p.outcome = Some((k1.clone(), Value::Null, false));
                        let mount = st(a).mounts;
                        st(a).psbt_asked.push_back((mount, k1.clone(), false));
                    }
                })
            } else {
                analyze
            })
            .child(if can_compare {
                compare.on_press(move |a: &mut App| {
                    let c = constraints(a);
                    let p = &a.provenance.chain.psbt;
                    let tok = |s: &Option<Value>| s.as_ref().and_then(|v| v.get("source_token")).cloned();
                    let mut args = json!({"before_token": tok(&p.before), "after_token": tok(&p.after), "network": p.network});
                    if p.payjoin {
                        args["payjoin"] = c.unwrap_or(Value::Null);
                    }
                    if a.send("ui.chain_analysis.psbt.compare", args, "provenance.chain.psbt.compare").is_some() {
                        let p = &mut st(a).psbt;
                        p.busy = true;
                        p.outcome = Some((k2.clone(), Value::Null, true));
                        let mount = st(a).mounts;
                        st(a).psbt_asked.push_back((mount, k2.clone(), true));
                    }
                })
            } else {
                compare
            }),
    );
    let current = p
        .outcome
        .as_ref()
        .filter(|(k, v, _)| *k == key && !v.is_null());
    if p.outcome.as_ref().is_some_and(|(_, v, _)| !v.is_null()) && current.is_none() {
        c = c.child(
            text(t("chainAnalysis:workbench.inputsChanged"))
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .role("status"),
        );
    }
    let hide = app.prefs.hide_sensitive();
    if let Some((_, v, cmp)) = current {
        if !cmp {
            c = c.child(result_view(app, tk, v, None));
        } else {
            let pj = v.get("payjoin").filter(|p| p.is_object());
            let delta = v.get("delta").cloned().unwrap_or(Value::Null);
            let n = |k: &str| {
                json!(
                    delta
                        .get(k)
                        .and_then(Value::as_array)
                        .map(Vec::len)
                        .unwrap_or(0)
                )
            };
            let fee = model::format_amount(delta.get("fee_msat").and_then(Value::as_str), None);
            let f = |label: &str, v: Value| {
                kit::fact(tk, label, &v)
                    .map(|e| e.w(0.0).grow(1.0))
                    .unwrap_or_else(|| col().w(0.0).grow(1.0))
            };
            let mut cc = col()
                .w_full()
                .gap(20.0)
                .child(
                    row()
                        .w_full()
                        .gap(16.0)
                        .items_start()
                        .child(
                            result_view(
                                app,
                                tk,
                                v.get("before").unwrap_or(&Value::Null),
                                Some(t("chainAnalysis:psbt.original")),
                            )
                            .w(0.0)
                            .grow(1.0),
                        )
                        .child(
                            result_view(
                                app,
                                tk,
                                v.get("after").unwrap_or(&Value::Null),
                                Some(t("chainAnalysis:psbt.proposal")),
                            )
                            .w(0.0)
                            .grow(1.0),
                        ),
                )
                .child(
                    text(t("chainAnalysis:psbt.changes"))
                        .font(tk.tsm(500.0))
                        .color(tk.fg),
                )
                .child(
                    row()
                        .w_full()
                        .gap(12.0)
                        .child(f(&t("chainAnalysis:psbt.inputsAdded"), n("inputs_added")))
                        .child(f(
                            &t("chainAnalysis:psbt.inputsRemoved"),
                            n("inputs_removed"),
                        ))
                        .child(f(
                            &t("chainAnalysis:psbt.feeDelta"),
                            json!(if hide {
                                crate::theme::MASK.to_owned()
                            } else {
                                fee
                            }),
                        )),
                );
            // The raw delta repeats the fee in msat.
            if !hide {
                cc = cc.child(kit::structured(tk, &delta, 0));
            }
            if let Some(pj) = pj {
                let mut b = col()
                    .w_full()
                    .padding(16.0)
                    .gap(12.0)
                    .radius(tk.r(0.5))
                    .border(1.0, tk.border)
                    .border_layout()
                    .child(
                        text(format!(
                            "{} · {}",
                            t("chainAnalysis:psbt.payjoin"),
                            pj.get("status").and_then(Value::as_str).unwrap_or("")
                        ))
                        .font(tk.tsm(500.0))
                        .color(tk.fg),
                    );
                for (i, check) in pj
                    .get("checks")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
                    .iter()
                    .enumerate()
                {
                    let code = check.get("code").and_then(Value::as_str).unwrap_or("");
                    let k = format!("pj-check-{code}");
                    let mut d = col().w_full().py(8.0).child(kit::summary_plain(
                        tk,
                        app,
                        &k,
                        row()
                            .child(
                                text(format!("{} · ", code.replace('_', " ")))
                                    .font(tk.txs(400.0))
                                    .color(tk.fg),
                            )
                            .child(
                                text(
                                    check
                                        .get("status")
                                        .and_then(Value::as_str)
                                        .unwrap_or("")
                                        .to_owned(),
                                )
                                .font(tk.txs(700.0))
                                .color(tk.fg),
                            ),
                    ));
                    if i > 0 {
                        d = d.border_sides([1.0, 0.0, 0.0, 0.0], tk.border);
                    }
                    if kit::is_open(app, &k) {
                        d = d.child(col().mt(8.0).child(kit::structured(
                            tk,
                            check.get("details").unwrap_or(&Value::Null),
                            0,
                        )));
                    }
                    b = b.child(d);
                }
                if let Some(l) = kit::code_list(
                    tk,
                    &t("chainAnalysis:entropyPanel.limitations"),
                    pj.get("limitations"),
                ) {
                    b = b.child(l);
                }
                cc = cc.child(b);
            }
            c = c.child(cc);
        }
        let mut eb = row().gap(8.0).child(
            kit::btn(
                tk,
                "psbt-ent-before",
                Variant::Outline,
                None,
                &t("chainAnalysis:psbt.entropyOriginal"),
                true,
            )
            .on_press(|a: &mut App| st(a).psbt.entropy_side = Some("before")),
        );
        if *cmp {
            let has_pj = v.get("payjoin").is_some_and(Value::is_object);
            let b = kit::btn(
                tk,
                "psbt-ent-after",
                Variant::Outline,
                None,
                &t("chainAnalysis:psbt.entropyProposal"),
                !has_pj,
            );
            eb = eb.child(if has_pj {
                b
            } else {
                b.on_press(|a: &mut App| st(a).psbt.entropy_side = Some("after"))
            });
        }
        c = c.child(eb);
        if *cmp && v.get("payjoin").is_some_and(Value::is_object) {
            c = c.child(
                text(t("chainAnalysis:entropyPanel.unsupported"))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg)
                    .wrap(),
            );
        }
    }
    if let Some(side) = p.entropy_side {
        let source = if side == "after" { &p.after } else { &p.before };
        if let Some(src) = source.as_ref().filter(|_| !(side == "after" && p.payjoin)) {
            let token = src
                .get("source_token")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let q = Query {
                network: Some(network.clone()),
                ..Query::default()
            };
            c = c.child(
                kit::bordered(tk, 16.0)
                    .child(
                        text(t(if side == "before" {
                            "chainAnalysis:psbt.entropyOriginal"
                        } else {
                            "chainAnalysis:psbt.entropyProposal"
                        }))
                        .font(tk.tsm(500.0))
                        .color(tk.fg)
                        .mb(12.0),
                    )
                    .child(results::entropy(
                        app,
                        tk,
                        layer,
                        Which::Psbt,
                        "psbt:transaction",
                        &q,
                        &[],
                        Some((token, network.clone())),
                    )),
            );
        }
    }
    c
}

fn dashed_box(tk: &Tk) -> El {
    col()
        .min_w(0.0)
        .padding(16.0)
        .radius(tk.r(0.5))
        .border(1.0, tk.border)
        .border_layout()
}

/// `PsbtResult`.
pub fn result_view(app: &App, tk: &Tk, r: &Value, title: Option<String>) -> El {
    let hide = app.prefs.hide_sensitive();
    let amount = |v: Option<&Value>| match v.and_then(Value::as_str) {
        None => t("chainAnalysis:workbench.unknown"),
        Some(_) if hide => crate::theme::MASK.to_owned(),
        Some(s) => model::format_amount(Some(s), None),
    };
    let g = |p: &str| r.pointer(p);
    let f = |label: &str, v: String| kit::fact_text(tk, label, v).w(0.0).grow(1.0);
    let rate = g("/totals/final_fee_rate_sat_vb")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| t("chainAnalysis:workbench.unknown"));
    let known = format!(
        "{}/{}",
        g("/coverage/known_input_amounts")
            .map(Value::to_string)
            .unwrap_or_default(),
        g("/coverage/input_count")
            .map(Value::to_string)
            .unwrap_or_default()
    );
    let mut c = kit::bordered(tk, 16.0)
        .gap(16.0)
        .child(
            text(format!(
                "{} · PSBT v{}",
                title.unwrap_or_else(|| t("chainAnalysis:psbt.result")),
                g("/psbt_version").map(Value::to_string).unwrap_or_default()
            ))
            .font(tk.tsm(500.0))
            .color(tk.fg),
        )
        .child(
            row()
                .w_full()
                .gap(16.0)
                .child(f(
                    &t("chainAnalysis:psbt.totalIn"),
                    amount(g("/totals/input_msat")),
                ))
                .child(f(
                    &t("chainAnalysis:psbt.totalOut"),
                    amount(g("/totals/output_msat")),
                ))
                .child(f(
                    &t("chainAnalysis:psbt.fee"),
                    amount(g("/totals/fee_msat")),
                )),
        )
        .child(
            row()
                .w_full()
                .gap(16.0)
                .child(f(&t("chainAnalysis:psbt.knownInputs"), known))
                .child(f(
                    &t("chainAnalysis:psbt.verifiedPrevious"),
                    g("/coverage/previous_transaction_hashes_verified")
                        .map(Value::to_string)
                        .unwrap_or_default(),
                ))
                .child(f(&t("chainAnalysis:psbt.finalRate"), rate)),
        )
        .child(
            text(t("chainAnalysis:psbt.validationHelp"))
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .wrap(),
        );
    let small = tk.txs(400.0);
    let mut table = col().w_full().child(
        row()
            .w_full()
            .gap(8.0)
            .child(
                text(t("chainAnalysis:psbt.input"))
                    .font(tk.txs(600.0))
                    .color(tk.fg)
                    .w(0.0)
                    .grow(1.0),
            )
            .child(
                text(t("chainAnalysis:amount"))
                    .font(tk.txs(600.0))
                    .color(tk.fg)
                    .w(0.0)
                    .grow(1.0),
            )
            .child(
                text(t("chainAnalysis:psbt.evidence"))
                    .font(tk.txs(600.0))
                    .color(tk.fg)
                    .w(0.0)
                    .grow(1.0),
            ),
    );
    for i in g("/transaction_facts/inputs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        table = table.child(
            row()
                .w_full()
                .padding(8.0)
                .gap(8.0)
                .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
                .child(
                    text(
                        i.get("input_index")
                            .map(Value::to_string)
                            .unwrap_or_default(),
                    )
                    .font(small)
                    .color(tk.fg)
                    .w(0.0)
                    .grow(1.0),
                )
                .child(
                    text(amount(i.get("amount_msat")))
                        .font(small.mono())
                        .color(tk.fg)
                        .w(0.0)
                        .grow(1.0),
                )
                .child(
                    text(
                        i.get("utxo_evidence")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned(),
                    )
                    .font(small)
                    .color(tk.fg)
                    .w(0.0)
                    .grow(1.0),
                ),
        );
    }
    c = c.child(
        col()
            .w_full()
            .max_h(256.0)
            .child(table.scroll(rune::ScrollSpec {
                stick_bottom: false,
            })),
    );
    if let Some(l) = kit::code_list(
        tk,
        &t("chainAnalysis:entropyPanel.limitations"),
        g("/validation/limitations"),
    ) {
        c = c.child(l);
    }
    let codes: Vec<Value> = g("/findings")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|f| f.get("code").cloned()).collect())
        .unwrap_or_default();
    if let Some(l) = kit::code_list(tk, &t("chainAnalysis:findings"), Some(&Value::Array(codes))) {
        c = c.child(l);
    }
    if let Some(f) = g("/features").filter(|v| v.is_object()) {
        c = c.child(super::inspector::features(
            app,
            tk,
            &format!(
                "psbtf-{}",
                g("/subject_id").and_then(Value::as_str).unwrap_or("")
            ),
            f,
        ));
    }
    c
}

/// Pins the inspection network once the binding (and, unbound, the inventory) is known, and
/// re-keys the entropy panel by source and network.
pub fn poll(app: &mut App) -> bool {
    let mut changed = false;
    if !app.provenance.chain.psbt.seeded {
        for (k, v) in [
            ("pj.payment", "0"),
            ("pj.fee", ""),
            ("pj.max", "0"),
            ("pj.rate", ""),
        ] {
            kit::set_val(app, k, v);
        }
        st(app).psbt.seeded = true;
        changed = true;
    }
    if app.provenance.chain.psbt.network.is_none() {
        let s = &app.provenance.chain;
        let book = s.binding.ready().and_then(book_network);
        let ready = book.is_some() || (s.binding.ready().is_some() && s.inventory.is_some());
        if ready {
            let initial = book.or(s.initial_network.clone());
            let n = initial
                .filter(|n| ["main", "test", "signet", "regtest"].contains(&n.as_str()))
                .unwrap_or_else(|| evidence_network(s.inventory.as_ref()));
            st(app).psbt.network = Some(n);
            changed = true;
        }
    }
    let p = &app.provenance.chain.psbt;
    if let Some(side) = p.entropy_side {
        let src = if side == "after" { &p.after } else { &p.before };
        let key = src
            .as_ref()
            .and_then(|s| s.get("source_token"))
            .and_then(Value::as_str)
            .map(|t| format!("{t}:{}", p.network.clone().unwrap_or_default()));
        if key.is_some() && key != p.entropy_for {
            st(app).psbt.entropy = Entropy::default();
            st(app).psbt.entropy_for = key;
            results::sync_entropy(app, Which::Psbt, "psbt:transaction");
            changed = true;
        }
    }
    changed
}

/// An entropy job started on a PSBT side (`onJob`).
pub fn on_entropy_job(app: &mut App, job: &Value) {
    let p = &app.provenance.chain.psbt;
    let side = p.entropy_side.unwrap_or("before");
    let token = (if side == "after" { &p.after } else { &p.before })
        .as_ref()
        .and_then(|s| s.get("source_token"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let network = p.network.clone().unwrap_or_default();
    if let Some(id) = job.get("job_id").and_then(Value::as_str) {
        st(app).psbt.entropy_job = Some((id.to_owned(), token, network));
    }
}

pub fn on_reply(app: &mut App, sub: &str, r: &Reply) {
    match sub {
        "stage.before" | "stage.after" => match &r.error {
            Some(e) => report(app, e.clone()),
            None if r.data.is_object() => {
                let p = &mut st(app).psbt;
                if sub == "stage.before" {
                    p.before = Some(r.data.clone())
                } else {
                    p.after = Some(r.data.clone())
                }
                p.entropy_side = None;
            }
            None => {}
        },
        "stage.dataset" => super::datasets::on_stage(app, r),
        "psbt.analyze" | "psbt.compare" => {
            let Some((mount, key, compare)) = st(app).psbt_asked.pop_front() else {
                return;
            };
            // Only the answer for the inputs (and the mount) the shown request was sent for: an
            // earlier mount's or earlier inputs' result never fills this slot.
            let current = mount == st(app).mounts
                && st(app)
                    .psbt
                    .outcome
                    .as_ref()
                    .is_some_and(|(k, _, c)| *k == key && *c == compare);
            if !current {
                return;
            }
            st(app).psbt.busy = false;
            match &r.error {
                Some(e) => {
                    st(app).psbt.outcome = None;
                    report(app, e.clone());
                }
                None => {
                    if let Some((_, v, _)) = st(app).psbt.outcome.as_mut() {
                        *v = r.data.clone();
                    }
                }
            }
        }
        "psbt_entropy.start" => results::entropy_reply(app, Which::Psbt, "start", r),
        "psbt_entropy.get" => results::entropy_reply(app, Which::Psbt, "get", r),
        "psbt_entropy.cancel" => results::entropy_reply(app, Which::Psbt, "cancel", r),
        _ => {}
    }
}

#[cfg(test)]
mod answer_tests {
    use super::*;

    /// Comparing one pair, leaving, coming back and comparing another before the first answer:
    /// the first answer does not fill the second comparison's result.
    #[test]
    fn an_earlier_comparison_does_not_fill_the_current_one() {
        let mut app = crate::app::tests::quiet_app();
        app.provenance.chain.psbt_asked.push_back((0, "pair-1".into(), true));
        app.provenance.chain.mounts = 1;
        app.provenance.chain.psbt_asked.push_back((1, "pair-2".into(), true));
        app.provenance.chain.psbt.outcome = Some(("pair-2".into(), Value::Null, true));
        let r = Reply {
            kind: "ui.chain_analysis.psbt.compare".into(),
            data: serde_json::json!({"findings": ["first"]}),
            error: None,
            error_detail: Value::Null,
            elapsed: Default::default(),
        };
        on_reply(&mut app, "psbt.compare", &r);
        assert!(app.provenance.chain.psbt.outcome.as_ref().unwrap().1.is_null());
        on_reply(&mut app, "psbt.compare", &r);
        assert!(!app.provenance.chain.psbt.outcome.as_ref().unwrap().1.is_null());
    }
}
