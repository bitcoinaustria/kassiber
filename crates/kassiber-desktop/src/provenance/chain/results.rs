//! `ResultPanels` (the Results card under the graph), `EntropyPanel` / `EntropyResult` and
//! `LabelsPanel`.

use rune::{El, Tx, col, row, text};
use serde_json::{Value, json};

use super::kit::{self, field, input, opt, select, val};
use super::model::{self, Node, Query, Sel, short_id};
use super::{report, st};
use crate::app::{App, Reply};
use crate::i18n::{t, tr};
use crate::theme::Tk;
use crate::txdetail::widgets::Variant;

/// `t(key, { defaultValue })`.
pub fn t_or(key: &str, default: &str) -> String {
    if crate::i18n::has(key) {
        t(key)
    } else {
        default.to_owned()
    }
}

fn tab_label(tab: &str) -> String {
    if tab == "labels" {
        t("chainAnalysis:labels.title")
    } else {
        t(&format!("chainAnalysis:{tab}"))
    }
}

fn empty(tk: &Tk) -> El {
    row().w_full().py(24.0).justify_center().child(
        text(t("chainAnalysis:noItems"))
            .font(tk.txs(400.0))
            .color(tk.muted_fg),
    )
}

/// `inspect`: highlight the ids and select the first.
fn inspect(app: &mut App, ids: Vec<String>) {
    let first = ids.first().cloned();
    st(app).highlighted = ids;
    if let Some(id) = first {
        super::select(app, Sel::node(id));
    }
}

fn card(tk: &Tk) -> El {
    col()
        .w_full()
        .min_w(0.0)
        .py(12.0)
        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
}

fn inspect_btn(tk: &Tk, k: String, ids: Vec<String>) -> El {
    let enabled = !ids.is_empty();
    let b = kit::btn(
        tk,
        k,
        Variant::Ghost,
        None,
        &t("chainAnalysis:inspect"),
        enabled,
    );
    if enabled {
        b.on_press(move |a: &mut App| inspect(a, ids.clone()))
    } else {
        b
    }
}

fn ids(v: &Value, k: &str) -> Vec<String> {
    v.get(k)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn finding_card(app: &App, tk: &Tk, i: usize, f: &Value) -> El {
    let code = f.get("code").and_then(Value::as_str).unwrap_or("");
    let title = t_or(
        &format!("privacyMirror:findingTitle.{code}"),
        f.get("title").and_then(Value::as_str).unwrap_or(""),
    );
    let id = f.get("id").and_then(Value::as_str).unwrap_or("").to_owned();
    card(tk)
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
                        .gap(4.0)
                        .child(text(title).font(tk.tsm(500.0)).color(tk.fg).wrap())
                        .child(
                            text(
                                f.get("detail")
                                    .and_then(Value::as_str)
                                    .unwrap_or("")
                                    .to_owned(),
                            )
                            .font(tk.txs(400.0).lh(tk.r(1.21875)))
                            .color(tk.muted_fg)
                            .wrap(),
                        ),
                )
                .child(inspect_btn(tk, format!("ca-f-{i}"), ids(f, "node_ids"))),
        )
        .child(kit::evidence(tk, app, &format!("ev-f-{id}-{i}"), f, None))
}

/// The Results card: a select of the result views and the chosen view.
pub fn view(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let s = &app.provenance.chain;
    let Some(r) = s.result.clone() else {
        return col();
    };
    let opts = model::TABS
        .iter()
        .map(|tab| {
            let count = r
                .tab_count(tab)
                .map(|n| format!(" ({n})"))
                .unwrap_or_default();
            opt(*tab, format!("{}{count}", tab_label(tab)))
        })
        .collect();
    let header = row()
        .w_full()
        .px(16.0)
        .py(12.0)
        .gap(12.0)
        .center_y()
        .justify_between()
        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
        .child(
            text(t("chainAnalysis:resultView"))
                .font(tk.tsm(500.0))
                .color(tk.fg),
        )
        .child(
            select(
                app,
                tk,
                layer,
                "ca-tab",
                &t("chainAnalysis:resultView"),
                s.tab,
                opts,
                true,
                32.0,
                |a: &mut App, v: &str| {
                    if let Some(tab) = model::TABS.iter().find(|x| **x == v) {
                        st(a).tab = tab;
                    }
                },
            )
            .w(158.0),
        );
    let mut body = col()
        .w_full()
        .padding(16.0)
        .gap(12.0)
        .role("region")
        .label(tab_label(s.tab));
    match s.tab {
        "findings" | "patterns" => {
            let list = r.list(s.tab);
            if list.is_empty() {
                body = body.child(empty(tk));
            }
            for (i, f) in list.iter().enumerate() {
                body = body.child(finding_card(app, tk, i, f));
            }
        }
        "clusters" => {
            let list = r.list("clusters");
            if list.is_empty() {
                body = body.child(empty(tk));
            }
            for (pair_i, pair) in list.chunks(2).enumerate() {
                let mut line = row().w_full().gap(12.0).items_start();
                for (j, c) in pair.iter().enumerate() {
                    let id = c.get("id").and_then(Value::as_str).unwrap_or("");
                    let label = c
                        .get("label")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                        .unwrap_or_else(|| short_id(id, 24));
                    line = line.child(
                        card(tk)
                            .w(0.0)
                            .grow(1.0)
                            .child(
                                row()
                                    .w_full()
                                    .gap(12.0)
                                    .center_y()
                                    .justify_between()
                                    .child(text(label).font(tk.tsm(500.0)).color(tk.fg))
                                    .child(inspect_btn(
                                        tk,
                                        format!("ca-c-{pair_i}-{j}"),
                                        ids(c, "node_ids"),
                                    )),
                            )
                            .child(kit::evidence(tk, app, &format!("ev-c-{id}"), c, None)),
                    );
                }
                if pair.len() == 1 {
                    line = line.child(col().w(0.0).grow(1.0));
                }
                body = body.child(line);
            }
        }
        "exposure" => {
            let list = r.list("exposure");
            if list.is_empty() {
                body = body.child(empty(tk));
            }
            for (i, e) in list.iter().enumerate() {
                let kind = e.get("kind").and_then(Value::as_str).unwrap_or("");
                let title = e
                    .pointer("/claim/label")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| {
                        t_or(
                            &format!("privacyMirror:findingTitle.{kind}"),
                            &kind.replace('_', " "),
                        )
                    });
                let n = ids(e, "node_ids");
                let id = e.get("id").and_then(Value::as_str).unwrap_or("");
                body = body.child(
                    card(tk)
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
                                        .gap(4.0)
                                        .child(text(title).font(tk.tsm(500.0)).color(tk.fg))
                                        .child(
                                            text(format!(
                                                "{} {}",
                                                n.len(),
                                                t("chainAnalysis:nodes")
                                            ))
                                            .font(tk.px(10.0, 400.0).mono())
                                            .color(tk.muted_fg),
                                        ),
                                )
                                .child(
                                    kit::btn(
                                        tk,
                                        format!("ca-x-{i}"),
                                        Variant::Ghost,
                                        None,
                                        &t("chainAnalysis:inspect"),
                                        true,
                                    )
                                    .on_press(move |a: &mut App| inspect(a, n.clone())),
                                ),
                        )
                        .child(kit::evidence(tk, app, &format!("ev-x-{id}"), e, None)),
                );
            }
        }
        "paths" => {
            let list = r.list("paths");
            if list.is_empty() {
                body = body.child(empty(tk));
            }
            for (i, p) in list.iter().enumerate() {
                let n = model::path_node_ids(p);
                let kind = if p.is_array() {
                    String::new()
                } else {
                    p.get("kind")
                        .map(|k| {
                            k.as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| k.to_string())
                        })
                        .unwrap_or_default()
                };
                body = body.child(
                    card(tk)
                        .child(
                            row()
                                .w_full()
                                .gap(12.0)
                                .center_y()
                                .justify_between()
                                .child(
                                    text(format!(
                                        "{} · {kind} · {} {}",
                                        i + 1,
                                        n.len(),
                                        t("chainAnalysis:nodes")
                                    ))
                                    .font(tk.txs(400.0).mono())
                                    .color(tk.fg),
                                )
                                .child(
                                    kit::btn(
                                        tk,
                                        format!("ca-p-{i}"),
                                        Variant::Ghost,
                                        None,
                                        &t("chainAnalysis:inspect"),
                                        true,
                                    )
                                    .on_press(move |a: &mut App| st(a).highlighted = n.clone()),
                                ),
                        )
                        .child(kit::evidence(tk, app, &format!("ev-p-{i}"), p, None)),
                );
            }
        }
        "frontier" => {
            let list = r.list("frontier");
            if list.is_empty() {
                body = body.child(empty(tk));
            } else {
                body = body.child(
                    row().w_full().justify_end().child(
                        kit::btn(
                            tk,
                            "ca-frontier-acquire",
                            Variant::Outline,
                            None,
                            &t("chainAnalysis:acquire.title"),
                            true,
                        )
                        .on_press(super::panels::show_acquire),
                    ),
                );
            }
            for (i, item) in list.iter().enumerate() {
                let reason = item.get("reason").and_then(Value::as_str).unwrap_or("");
                let node = item
                    .get("node_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let mut line = row()
                    .w_full()
                    .gap(12.0)
                    .items_start()
                    .justify_between()
                    .child(
                        col()
                            .min_w(0.0)
                            .grow(1.0)
                            .gap(4.0)
                            .child(
                                text(t_or(
                                    &format!("privacyMirror:reason.{reason}"),
                                    &reason.replace('_', " "),
                                ))
                                .font(tk.txs(500.0))
                                .color(tk.fg),
                            )
                            .child(
                                text(node.clone().unwrap_or_default())
                                    .font(tk.px(11.0, 400.0).mono())
                                    .color(tk.muted_fg)
                                    .wrap_anywhere(),
                            )
                            .child(kit::evidence(tk, app, &format!("ev-fr-{i}"), item, None)),
                    );
                if let Some(n) = node {
                    line = line.child(
                        kit::btn(
                            tk,
                            format!("ca-fr-{i}"),
                            Variant::Ghost,
                            None,
                            &t("chainAnalysis:inspect"),
                            true,
                        )
                        .on_press(move |a: &mut App| super::select(a, Sel::node(n.clone()))),
                    );
                }
                body = body.child(card(tk).child(line));
            }
        }
        "coverage" => body = body.child(coverage(app, tk, &r)),
        "entropy" => {
            let picked = picked_subject(app);
            body = body.child(entropy(
                app,
                tk,
                layer,
                Which::Graph,
                &picked,
                &r.query,
                &r.nodes,
                None,
            ));
        }
        "labels" => body = body.child(labels(app, tk, layer)),
        _ => {}
    }
    crate::ui::surface(tk)
        .w_full()
        .clip_rounded(tk.r(0.75))
        .child(header)
        .child(body)
}

/// `pickedSubject`: the selected node, else the executed or drafted subject.
pub fn picked_subject(app: &App) -> String {
    let s = &app.provenance.chain;
    let Some(r) = &s.result else {
        return s.query.subject.clone().unwrap_or_default();
    };
    s.sel
        .as_ref()
        .filter(|x| !x.edge)
        .and_then(|x| r.node(&x.id))
        .map(|n| n.id.clone())
        .or_else(|| r.query.subject.clone().filter(|v| !v.is_empty()))
        .or_else(|| s.query.subject.clone())
        .unwrap_or_default()
}

fn coverage(app: &App, tk: &Tk, r: &model::AResult) -> El {
    let cov = r.coverage();
    let g = |k: &str| cov.get(k).cloned().unwrap_or(Value::Null);
    let pair = |a: Option<El>, b: Option<El>| {
        row()
            .w_full()
            .gap(12.0)
            .child(
                a.map(|e| e.w(0.0).grow(1.0))
                    .unwrap_or_else(|| col().w(0.0).grow(1.0)),
            )
            .child(
                b.map(|e| e.w(0.0).grow(1.0))
                    .unwrap_or_else(|| col().w(0.0).grow(1.0)),
            )
    };
    let left = col()
        .w(0.0)
        .grow(1.0)
        .min_w(0.0)
        .child(
            col()
                .w_full()
                .gap(12.0)
                .mb(12.0)
                .child(pair(
                    kit::fact(
                        tk,
                        &t("chainAnalysis:coverageMissing"),
                        &g("missing_node_count"),
                    ),
                    kit::fact(
                        tk,
                        &t("chainAnalysis:coverageCompleteTx"),
                        &g("complete_transaction_count"),
                    ),
                ))
                .child(pair(
                    kit::fact(
                        tk,
                        &t("chainAnalysis:coverageConflict"),
                        &g("conflicting_node_count"),
                    ),
                    kit::fact(tk, &t("chainAnalysis:coverageFresh"), &g("custody_fresh")),
                )),
        )
        .child(
            text(
                cov.get("unobserved_successors")
                    .map(|v| {
                        v.as_str().map(str::to_owned).unwrap_or_else(|| {
                            if v.is_null() {
                                String::new()
                            } else {
                                v.to_string()
                            }
                        })
                    })
                    .unwrap_or_default(),
            )
            .font(tk.txs(400.0))
            .color(tk.muted_fg)
            .wrap(),
        )
        .child(kit::evidence(
            tk,
            app,
            "cov-sources",
            &g("source_rows"),
            Some(t("chainAnalysis:coverageSources")),
        ))
        .child(kit::evidence(
            tk,
            app,
            "cov-pruning",
            &g("pruning"),
            Some(t("chainAnalysis:coveragePruning")),
        ))
        .child(kit::evidence(
            tk,
            app,
            "cov-all",
            cov,
            Some(t("chainAnalysis:coverage")),
        ));
    let right = col()
        .w(0.0)
        .grow(1.0)
        .min_w(0.0)
        .child(
            col()
                .w_full()
                .gap(12.0)
                .child(pair(
                    kit::fact(tk, &t("chainAnalysis:nodes"), &json!(r.nodes.len())),
                    kit::fact(tk, &t("chainAnalysis:edges"), &json!(r.edges.len())),
                ))
                .children(kit::fact(
                    tk,
                    &t("chainAnalysis:snapshot"),
                    &json!(r.snapshot_id),
                )),
        )
        .child(kit::evidence(
            tk,
            app,
            "cov-query",
            r.raw.get("query").unwrap_or(&Value::Null),
            Some(t("chainAnalysis:resultQuery")),
        ))
        .child(kit::evidence(
            tk,
            app,
            "cov-cap",
            r.raw.get("capabilities").unwrap_or(&Value::Null),
            Some(t("chainAnalysis:capabilities")),
        ));
    row()
        .w_full()
        .gap(16.0)
        .items_start()
        .child(left)
        .child(right)
}

// ---- Entropy ----------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Which {
    Graph,
    Psbt,
}

impl Which {
    fn p(self) -> &'static str {
        match self {
            Which::Graph => "ent",
            Which::Psbt => "pent",
        }
    }
    fn tag(self, op: &str) -> &'static str {
        match (self, op) {
            (Which::Graph, "start") => "provenance.chain.entropy.start",
            (Which::Graph, "get") => "provenance.chain.entropy.get",
            (Which::Graph, _) => "provenance.chain.entropy.cancel",
            (Which::Psbt, "start") => "provenance.chain.psbt_entropy.start",
            (Which::Psbt, "get") => "provenance.chain.psbt_entropy.get",
            (Which::Psbt, _) => "provenance.chain.psbt_entropy.cancel",
        }
    }
}

#[derive(Default)]
pub struct Entropy {
    /// The `subject` prop the field was last reset to.
    pub synced_subject: Option<String>,
    pub domain_edit: Option<(String, String, String)>,
    pub observer: Option<String>,
    pub kind: Option<String>,
    pub protocol: Option<String>,
    pub job: Option<Value>,
    pub baseline: Option<Value>,
    pub poll_error: bool,
    pub starting: bool,
    pub polling: bool,
    pub last_poll: f64,
    pub cancelling: bool,
    pub page: usize,
}

pub fn ent(app: &mut App, w: Which) -> &mut Entropy {
    match w {
        Which::Graph => &mut app.provenance.chain.entropy,
        Which::Psbt => &mut app.provenance.chain.psbt.entropy,
    }
}

fn ent_ref(app: &App, w: Which) -> &Entropy {
    match w {
        Which::Graph => &app.provenance.chain.entropy,
        Which::Psbt => &app.provenance.chain.psbt.entropy,
    }
}

/// Resets the subject field when the `subject` prop changes (`useEffect([subject])`).
pub fn sync_entropy(app: &mut App, w: Which, subject: &str) {
    if ent_ref(app, w).synced_subject.as_deref() == Some(subject) {
        return;
    }
    let p = w.p();
    for (k, v) in [
        ("subject", subject),
        ("budget", "200000"),
        ("duration", "1000"),
        ("received", "0"),
        ("paid", "0"),
    ] {
        let key = format!("{p}.{k}");
        if k == "subject" || !app.provenance.chain.edits.contains_key(&key) {
            kit::set_val(app, &key, v);
        }
    }
    let e = ent(app, w);
    e.synced_subject = Some(subject.to_owned());
    e.domain_edit = None;
}

struct Form {
    value: String,
    chain: String,
    network: String,
    domain_ready: bool,
    request: Value,
    scenario: Option<Value>,
    budget: i64,
    duration: i64,
}

fn form(app: &App, w: Which, query: &Query, nodes: &[Node]) -> Form {
    let p = w.p();
    let e = ent_ref(app, w);
    let value = val(app, &format!("{p}.subject"));
    let inferred = model::entropy_subject_domain(
        &value,
        nodes,
        query.chain.as_deref(),
        query.network.as_deref(),
    );
    let (chain, network) = match (&e.domain_edit, &inferred) {
        (Some((s, c, n)), _) if *s == value.trim() => (c.clone(), n.clone()),
        (_, model::Domain::Resolved { chain, network, .. }) => (chain.clone(), network.clone()),
        _ => (
            query.chain.clone().unwrap_or_default(),
            query
                .network
                .as_deref()
                .map(|n| model::network_input(Some(n)))
                .unwrap_or_default(),
        ),
    };
    let resolved = model::entropy_subject_domain(
        &value,
        nodes,
        (!chain.is_empty()).then_some(chain.as_str()),
        (!network.is_empty()).then_some(network.as_str()),
    );
    let domain_ready = !chain.is_empty()
        && !network.is_empty()
        && !(chain == "liquid" && network == "signet")
        && resolved != model::Domain::Ambiguous;
    let observer = e.observer.clone().unwrap_or_else(|| {
        if query.observer == "public" {
            "public".into()
        } else {
            "owner".into()
        }
    });
    let num = |k: &str| {
        val(app, &format!("{p}.{k}"))
            .trim()
            .parse::<f64>()
            .map(|f| f as i64)
            .unwrap_or(0)
    };
    let (budget, duration) = (num("budget"), num("duration"));
    let scenario = model::entropy_scenario(
        e.kind.as_deref().unwrap_or("independent"),
        e.protocol.as_deref().unwrap_or("generic"),
        &val(app, &format!("{p}.received")),
        &val(app, &format!("{p}.paid")),
    );
    let subject = match &resolved {
        model::Domain::Resolved { subject, .. } => subject.clone(),
        _ => value.trim().to_owned(),
    };
    let mut request = json!({"subject": subject, "network": network, "observer": observer, "max_states": budget, "max_duration_ms": duration});
    if !chain.is_empty() {
        request["chain"] = json!(chain);
    }
    if let Some(s) = &scenario {
        request["scenario"] = s.clone();
    }
    Form {
        value,
        chain,
        network,
        domain_ready,
        request,
        scenario,
        budget,
        duration,
    }
}

/// `EntropyPanel`: the conditional interpretation count of one transaction (or a staged PSBT).
#[allow(clippy::too_many_arguments)]
pub fn entropy(
    app: &App,
    tk: &Tk,
    layer: &mut Vec<El>,
    w: Which,
    _subject: &str,
    query: &Query,
    nodes: &[Node],
    psbt: Option<(String, String)>,
) -> El {
    let p = w.p();
    let e = ent_ref(app, w);
    let f = form(app, w, query, nodes);
    let running = e
        .job
        .as_ref()
        .and_then(|j| j.get("status"))
        .and_then(Value::as_str)
        == Some("running");
    let busy = e.starting || running;
    let small = tk.txs(400.0);
    let kind = e.kind.clone().unwrap_or_else(|| "independent".into());
    let protocol = e.protocol.clone().unwrap_or_else(|| "generic".into());
    let observer = e.observer.clone().unwrap_or_else(|| {
        if query.observer == "public" {
            "public".into()
        } else {
            "owner".into()
        }
    });
    let mut c = col().w_full().gap(16.0).child(
        text(t("chainAnalysis:entropyPanel.description"))
            .font(tk.tsm(400.0).lh(tk.r(1.421875)))
            .color(tk.muted_fg)
            .wrap()
            .max_w(768.0),
    );
    let mut fields: Vec<El> = vec![];
    if psbt.is_none() {
        fields.push(
            field(
                tk,
                &t("chainAnalysis:entropyPanel.subject"),
                input(
                    tk,
                    app,
                    &format!("{p}.subject"),
                    &t("chainAnalysis:entropyPanel.subject"),
                    "",
                    true,
                    32.0,
                    !busy,
                    Some(match w {
                        Which::Graph => |a: &mut App| ent(a, Which::Graph).domain_edit = None,
                        Which::Psbt => |a: &mut App| ent(a, Which::Psbt).domain_edit = None,
                    }),
                ),
            )
            .w(0.0)
            .grow(2.0),
        );
        let value = f.value.trim().to_owned();
        let (v1, n1) = (value.clone(), f.network.clone());
        fields.push(
            field(
                tk,
                &t("chainAnalysis:chain"),
                select(
                    app,
                    tk,
                    layer,
                    &format!("{p}-chain"),
                    &t("chainAnalysis:chain"),
                    &f.chain,
                    vec![
                        opt("", t("chainAnalysis:entropyPanel.chooseDomain")),
                        opt("bitcoin", "Bitcoin"),
                        opt("liquid", "Liquid"),
                    ],
                    !busy,
                    32.0,
                    move |a: &mut App, v: &str| {
                        ent(a, w).domain_edit = Some((v1.clone(), v.to_owned(), n1.clone()))
                    },
                ),
            )
            .w(0.0)
            .grow(1.0),
        );
        let c1 = f.chain.clone();
        let mut nets = vec![opt("", t("chainAnalysis:entropyPanel.chooseDomain"))];
        nets.extend(
            ["main", "test", "signet", "regtest"]
                .iter()
                .filter(|n| f.chain != "liquid" || **n != "signet")
                .map(|n| opt(*n, *n)),
        );
        fields.push(
            field(
                tk,
                &t("chainAnalysis:network"),
                select(
                    app,
                    tk,
                    layer,
                    &format!("{p}-network"),
                    &t("chainAnalysis:network"),
                    &f.network,
                    nets,
                    !busy,
                    32.0,
                    move |a: &mut App, v: &str| {
                        ent(a, w).domain_edit = Some((value.clone(), c1.clone(), v.to_owned()))
                    },
                ),
            )
            .w(0.0)
            .grow(1.0),
        );
        fields.push(
            field(
                tk,
                &t("chainAnalysis:workbench.observer"),
                select(
                    app,
                    tk,
                    layer,
                    &format!("{p}-observer"),
                    &t("chainAnalysis:workbench.observer"),
                    &observer,
                    vec![
                        opt("owner", t("chainAnalysis:workbench.owner")),
                        opt("public", t("chainAnalysis:workbench.public")),
                    ],
                    !busy,
                    32.0,
                    move |a: &mut App, v: &str| ent(a, w).observer = Some(v.to_owned()),
                ),
            )
            .w(0.0)
            .grow(1.0),
        );
    }
    fields.push(
        field(
            tk,
            &t("chainAnalysis:workbench.scenario"),
            select(
                app,
                tk,
                layer,
                &format!("{p}-kind"),
                &t("chainAnalysis:workbench.scenario"),
                &kind,
                vec![
                    opt("independent", t("chainAnalysis:workbench.independent")),
                    opt("coinjoin_intrafees", t("chainAnalysis:workbench.intrafees")),
                ],
                !busy,
                32.0,
                move |a: &mut App, v: &str| ent(a, w).kind = Some(v.to_owned()),
            ),
        )
        .w(0.0)
        .grow(1.0),
    );
    fields.push(
        field(
            tk,
            &t("chainAnalysis:workbench.protocol"),
            select(
                app,
                tk,
                layer,
                &format!("{p}-protocol"),
                &t("chainAnalysis:workbench.protocol"),
                &protocol,
                ["generic", "whirlpool", "joinmarket", "wabisabi"]
                    .iter()
                    .map(|x| opt(*x, *x))
                    .collect(),
                !busy,
                32.0,
                move |a: &mut App, v: &str| ent(a, w).protocol = Some(v.to_owned()),
            ),
        )
        .w(0.0)
        .grow(1.0),
    );
    fields.push(
        field(
            tk,
            &t("chainAnalysis:entropyPanel.budget"),
            input(
                tk,
                app,
                &format!("{p}.budget"),
                &t("chainAnalysis:entropyPanel.budget"),
                "",
                false,
                32.0,
                !busy,
                None,
            ),
        )
        .w(0.0)
        .grow(1.0),
    );
    fields.push(
        field(
            tk,
            &t("chainAnalysis:workbench.duration"),
            input(
                tk,
                app,
                &format!("{p}.duration"),
                &t("chainAnalysis:workbench.duration"),
                "",
                false,
                32.0,
                !busy,
                None,
            ),
        )
        .w(0.0)
        .grow(1.0),
    );
    if kind == "coinjoin_intrafees" {
        fields.push(
            field(
                tk,
                &t("chainAnalysis:workbench.received"),
                input(
                    tk,
                    app,
                    &format!("{p}.received"),
                    &t("chainAnalysis:workbench.received"),
                    "",
                    true,
                    32.0,
                    !busy,
                    None,
                ),
            )
            .w(0.0)
            .grow(1.0),
        );
        fields.push(
            field(
                tk,
                &t("chainAnalysis:workbench.paid"),
                input(
                    tk,
                    app,
                    &format!("{p}.paid"),
                    &t("chainAnalysis:workbench.paid"),
                    "",
                    true,
                    32.0,
                    !busy,
                    None,
                ),
            )
            .w(0.0)
            .grow(1.0),
        );
    }
    // `lg:grid-cols-4`: rows of four (the subject spans two).
    let mut grid = col().w_full().gap(12.0);
    let mut line = row().w_full().gap(12.0);
    let mut used = 0;
    for (i, fe) in fields.into_iter().enumerate() {
        let span = if psbt.is_none() && i == 0 { 2 } else { 1 };
        if used + span > 4 {
            grid = grid.child(line.child(col().w(0.0).grow((4 - used) as f32)));
            line = row().w_full().gap(12.0);
            used = 0;
        }
        line = line.child(fe);
        used += span;
    }
    if used > 0 {
        if used < 4 {
            line = line.child(col().w(0.0).grow((4 - used) as f32));
        }
        grid = grid.child(line);
    }
    grid = grid.child(
        text(t("chainAnalysis:workbench.scenarioHelp"))
            .font(small)
            .color(tk.muted_fg)
            .wrap(),
    );
    if f.scenario.is_none() {
        grid = grid.child(kit::alert_text(
            tk,
            t("chainAnalysis:workbench.exactMsat"),
            small,
        ));
    }
    if psbt.is_none() && !f.domain_ready {
        grid = grid.child(
            text(t("chainAnalysis:entropyPanel.domainRequired"))
                .font(small)
                .color(tk.muted_fg)
                .wrap(),
        );
    }
    let enabled = !busy
        && f.scenario.is_some()
        && (psbt.is_some() || (!f.value.trim().is_empty() && f.domain_ready))
        && f.budget >= 1
        && f.budget <= 2_000_000
        && f.duration >= 1
        && f.duration <= 30_000;
    let (req, scenario, budget, duration, src) = (
        f.request.clone(),
        f.scenario.clone(),
        f.budget,
        f.duration,
        psbt.clone(),
    );
    let run = kit::btn(
        tk,
        format!("{p}-run"),
        Variant::Default,
        None,
        &t("chainAnalysis:entropyPanel.run"),
        enabled,
    );
    grid = grid.child(row().child(if enabled {
        run.on_press(move |a: &mut App| {
            let args = match &src {
                Some((token, network)) => json!({"psbt_token": token, "network": network, "scenario": scenario, "max_states": budget, "max_duration_ms": duration}),
                None => req.clone(),
            };
            let kind = if src.is_some() { "ui.chain_analysis.psbt.entropy.start" } else { "ui.chain_analysis.entropy.start" };
            ent(a, w).poll_error = false;
            if a.send(kind, args, w.tag("start")).is_some() {
                ent(a, w).starting = true;
            }
        })
    } else {
        run
    }));
    c = c.child(grid);
    if let Some(job) = &e.job {
        c = c.child(job_view(app, tk, w, job, e));
    }
    c
}

fn job_view(app: &App, tk: &Tk, w: Which, job: &Value, e: &Entropy) -> El {
    let p = w.p();
    let small = tk.txs(400.0);
    let status = job.get("status").and_then(Value::as_str).unwrap_or("");
    let result = job.get("result").filter(|r| r.is_object());
    let snapshot = job
        .pointer("/request/snapshot_id")
        .and_then(Value::as_str)
        .or(result
            .and_then(|r| r.get("snapshot_id"))
            .and_then(Value::as_str))
        .unwrap_or("—")
        .to_owned();
    let mut head = row()
        .w_full()
        .gap(12.0)
        .flex_wrap()
        .items_start()
        .justify_between()
        .child(
            col()
                .gap(4.0)
                .child(
                    text(t("chainAnalysis:workbench.submitted"))
                        .font(tk.tsm(500.0))
                        .color(tk.fg),
                )
                .child(
                    text(snapshot)
                        .font(tk.px(10.0, 400.0).mono())
                        .color(tk.muted_fg)
                        .wrap_anywhere(),
                ),
        );
    if status == "running" {
        let requested = job
            .get("cancel_requested")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let enabled = !e.cancelling && !requested;
        let id = job.get("job_id").cloned().unwrap_or(Value::Null);
        let b = kit::btn(
            tk,
            format!("{p}-cancel"),
            Variant::Outline,
            None,
            &t(if requested {
                "chainAnalysis:workbench.cancelling"
            } else {
                "chainAnalysis:workbench.cancel"
            }),
            enabled,
        );
        head = head.child(if enabled {
            b.on_press(move |a: &mut App| {
                if a.send(
                    "ui.chain_analysis.jobs.cancel",
                    json!({"job_id": id}),
                    w.tag("cancel"),
                )
                .is_some()
                {
                    ent(a, w).cancelling = true;
                }
            })
        } else {
            b
        });
    }
    let n = |p: &str| {
        job.pointer(p)
            .map(|v| {
                v.as_u64()
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| v.to_string())
            })
            .unwrap_or_default()
    };
    let elapsed = job
        .pointer("/progress/elapsed_ms")
        .filter(|v| !v.is_null())
        .or(job.get("elapsed_ms"))
        .map(Value::to_string)
        .unwrap_or_default();
    let mut c = col()
        .w_full()
        .padding(16.0)
        .gap(16.0)
        .radius(tk.r(0.5))
        .border(1.0, tk.border)
        .border_layout()
        .fill(tk.muted.alpha(0.15))
        .child(head)
        .child(
            row()
                .w_full()
                .gap(20.0)
                .flex_wrap()
                .role("status")
                .child(
                    text(format!(
                        "{status} · {}",
                        job.pointer("/progress/phase")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                    ))
                    .font(small)
                    .color(tk.fg),
                )
                .child(
                    text(tr(
                        "chainAnalysis:workbench.work",
                        &[("count", &n("/progress/states_explored"))],
                    ))
                    .font(small)
                    .color(tk.fg),
                )
                .child(
                    text(tr("chainAnalysis:workbench.elapsed", &[("ms", &elapsed)]))
                        .font(small)
                        .color(tk.fg),
                ),
        );
    if e.poll_error {
        c = c.child(
            row().child(
                kit::btn(
                    tk,
                    format!("{p}-resume"),
                    Variant::Outline,
                    None,
                    &t("chainAnalysis:workbench.resume"),
                    true,
                )
                .on_press(move |a: &mut App| ent(a, w).poll_error = false),
            ),
        );
    }
    if let Some(code) = job.get("error_code").and_then(Value::as_str) {
        c = c.child(kit::alert_text(tk, code.to_owned(), tk.tsm(400.0)));
    }
    if let Some(r) = result {
        c = c.child(entropy_result(app, tk, w, r, job.get("request")));
        if r.get("status").and_then(Value::as_str) == Some("exact") {
            let rv = r.clone();
            c = c.child(
                row().child(
                    kit::btn(
                        tk,
                        format!("{p}-pin"),
                        Variant::Outline,
                        None,
                        &t("chainAnalysis:workbench.pinBaseline"),
                        true,
                    )
                    .on_press(move |a: &mut App| ent(a, w).baseline = Some(rv.clone())),
                ),
            );
        }
        if let Some(b) = &e.baseline {
            let changes = model::entropy_link_changes(b, r);
            let mut cmp = col()
                .w_full()
                .pt(12.0)
                .gap(8.0)
                .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
                .child(
                    text(t("chainAnalysis:workbench.comparison"))
                        .font(tk.tsm(500.0))
                        .color(tk.fg),
                )
                .child(kit::structured(
                    tk,
                    b.get("scenario").unwrap_or(&Value::Null),
                    0,
                ))
                .child(
                    text(t("chainAnalysis:workbench.comparisonHelp"))
                        .font(small)
                        .color(tk.muted_fg)
                        .wrap(),
                );
            match changes {
                None => {
                    cmp = cmp.child(
                        text(t("chainAnalysis:workbench.incomparable"))
                            .font(small)
                            .color(tk.fg)
                            .wrap(),
                    )
                }
                Some(ch) => {
                    let f = |label: &str, v: Value| {
                        kit::fact(tk, label, &v)
                            .map(|e| e.w(0.0).grow(1.0))
                            .unwrap_or_else(|| col().w(0.0).grow(1.0))
                    };
                    cmp = cmp.child(
                        row()
                            .w_full()
                            .gap(12.0)
                            .child(f(
                                &t("chainAnalysis:workbench.before"),
                                b.get("interpretation_count")
                                    .cloned()
                                    .unwrap_or(Value::Null),
                            ))
                            .child(f(
                                &t("chainAnalysis:workbench.after"),
                                r.get("interpretation_count")
                                    .cloned()
                                    .unwrap_or(Value::Null),
                            ))
                            .child(f(
                                &t("chainAnalysis:workbench.changedLinks"),
                                json!(ch.len()),
                            )),
                    );
                    let mut table = col().w_full().child(
                        row()
                            .w_full()
                            .gap(8.0)
                            .child(
                                text(t("chainAnalysis:workbench.link"))
                                    .font(tk.txs(600.0))
                                    .color(tk.fg)
                                    .w(0.0)
                                    .grow(2.0),
                            )
                            .child(
                                text(t("chainAnalysis:workbench.before"))
                                    .font(tk.txs(600.0))
                                    .color(tk.fg)
                                    .w(0.0)
                                    .grow(1.0),
                            )
                            .child(
                                text(t("chainAnalysis:workbench.after"))
                                    .font(tk.txs(600.0))
                                    .color(tk.fg)
                                    .w(0.0)
                                    .grow(1.0),
                            ),
                    );
                    for l in ch.iter().take(100) {
                        table = table.child(
                            row()
                                .w_full()
                                .py(8.0)
                                .gap(8.0)
                                .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
                                .child(
                                    text(format!(
                                        "{} → {}",
                                        short_id(&l.input_id, 24),
                                        short_id(&l.output_id, 24)
                                    ))
                                    .font(small.mono())
                                    .color(tk.fg)
                                    .w(0.0)
                                    .grow(2.0)
                                    .wrap(),
                                )
                                .child(
                                    text(l.before.clone())
                                        .font(small.mono())
                                        .color(tk.fg)
                                        .w(0.0)
                                        .grow(1.0)
                                        .wrap(),
                                )
                                .child(
                                    text(l.after.clone())
                                        .font(small.mono())
                                        .color(tk.fg)
                                        .w(0.0)
                                        .grow(1.0)
                                        .wrap(),
                                ),
                        );
                    }
                    cmp = cmp.child(col().w_full().max_h(256.0).child(table.scroll(
                        rune::ScrollSpec {
                            stick_bottom: false,
                        },
                    )));
                    if ch.len() > 100 {
                        cmp = cmp.child(
                            text(tr(
                                "chainAnalysis:workbench.firstRows",
                                &[("count", "100"), ("total", &ch.len().to_string())],
                            ))
                            .font(small)
                            .color(tk.fg),
                        );
                    }
                }
            }
            c = c.child(cmp);
        }
    }
    c
}

/// `EntropyResult`.
pub fn entropy_result(app: &App, tk: &Tk, w: Which, r: &Value, request: Option<&Value>) -> El {
    let p = w.p();
    let status = r.get("status").and_then(Value::as_str).unwrap_or("");
    let exact = status == "exact";
    let bounded = ["model_bounded", "timeout", "cancelled"].contains(&status);
    let links = if exact {
        model::entropy_links(r.get("link_counts"))
    } else {
        vec![]
    };
    let f = |label: &str, v: Value| {
        kit::fact(tk, label, &v)
            .map(|e| e.w(0.0).grow(1.0))
            .unwrap_or_else(|| col().w(0.0).grow(1.0))
    };
    let mut first = row().w_full().gap(16.0).child(f(
        &t("chainAnalysis:entropyPanel.resultSubject"),
        r.get("subject")
            .filter(|v| !v.is_null())
            .cloned()
            .or(request.and_then(|q| q.get("subject")).cloned())
            .unwrap_or(Value::Null),
    ));
    if let Some(q) = request {
        let chain = q.get("chain").and_then(Value::as_str).unwrap_or("bitcoin");
        let request_network = q
            .get("network")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| t("chainAnalysis:all"));
        first = first
            .child(f(
                &t("chainAnalysis:entropyPanel.requestDomain"),
                json!(format!("{chain} / {request_network}")),
            ))
            .child(f(
                &t("chainAnalysis:entropyPanel.budget"),
                q.get("max_states").cloned().unwrap_or(Value::Null),
            ));
    }
    let mut second = row()
        .w_full()
        .gap(16.0)
        .child(f(
            &t("chainAnalysis:entropyPanel.status"),
            r.get("status").cloned().unwrap_or(Value::Null),
        ))
        .child(f(
            &t("chainAnalysis:entropyPanel.model"),
            r.get("model").cloned().unwrap_or(Value::Null),
        ));
    if exact || bounded {
        second = second.child(f(
            &t(if exact {
                "chainAnalysis:entropyPanel.partitions"
            } else {
                "chainAnalysis:entropyPanel.lowerBound"
            }),
            r.get(if exact {
                "interpretation_count"
            } else {
                "interpretation_count_lower_bound"
            })
            .cloned()
            .unwrap_or(Value::Null),
        ));
    }
    if exact {
        second = second.child(f(
            &t("chainAnalysis:entropyPanel.bits"),
            r.get("entropy_bits").cloned().unwrap_or(Value::Null),
        ));
    }
    let mut c = col()
        .w_full()
        .gap(16.0)
        .child(first)
        .child(second)
        .children(kit::fact(
            tk,
            &t("chainAnalysis:entropyPanel.reason"),
            r.get("reason").unwrap_or(&Value::Null),
        ));
    if r.get("scenario").is_some_and(|s| !s.is_null()) {
        c = c.child(
            col()
                .w_full()
                .padding(12.0)
                .gap(8.0)
                .radius(tk.r(0.5))
                .border(1.0, tk.border)
                .border_layout()
                .child(
                    text(t("chainAnalysis:workbench.scenario"))
                        .font(tk.txs(500.0))
                        .color(tk.fg),
                )
                .child(kit::structured(tk, &r["scenario"], 0)),
        );
    }
    let lists = [
        kit::code_list(
            tk,
            &t("chainAnalysis:entropyPanel.assumptions"),
            r.get("assumptions"),
        ),
        kit::code_list(
            tk,
            &t("chainAnalysis:entropyPanel.limitations"),
            r.get("limitations"),
        ),
    ];
    c = c.child(
        row()
            .w_full()
            .gap(16.0)
            .items_start()
            .children(lists.into_iter().map(|l| {
                l.map(|e| e.w(0.0).grow(1.0))
                    .unwrap_or_else(|| col().w(0.0).grow(1.0))
            })),
    );
    if !links.is_empty() {
        let k = format!("{p}-links");
        let mut d = col()
            .w_full()
            .padding(12.0)
            .radius(tk.r(0.5))
            .border(1.0, tk.border)
            .border_layout()
            .child(kit::summary_plain(
                tk,
                app,
                &k,
                text(format!(
                    "{} · {}",
                    t("chainAnalysis:entropyPanel.details"),
                    links.len()
                ))
                .font(tk.txs(500.0))
                .color(tk.fg),
            ));
        if kit::is_open(app, &k) {
            let pages = links.len().div_ceil(50).max(1);
            let shown = ent_ref(app, w).page.min(pages - 1);
            let total = r
                .get("interpretation_count")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let small = tk.txs(400.0);
            let mut table = col().w_full().child(
                row()
                    .w_full()
                    .gap(8.0)
                    .child(
                        text(t("chainAnalysis:from"))
                            .font(tk.txs(600.0))
                            .color(tk.fg)
                            .w(0.0)
                            .grow(1.0),
                    )
                    .child(
                        text(t("chainAnalysis:to"))
                            .font(tk.txs(600.0))
                            .color(tk.fg)
                            .w(0.0)
                            .grow(1.0),
                    )
                    .child(
                        text(t("chainAnalysis:entropyPanel.partitions"))
                            .font(tk.txs(600.0))
                            .color(tk.fg)
                            .w(0.0)
                            .grow(1.0),
                    ),
            );
            for (i, o, n) in links.iter().skip(shown * 50).take(50) {
                table = table.child(
                    row()
                        .w_full()
                        .padding(8.0)
                        .gap(8.0)
                        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
                        .child(
                            text(i.clone())
                                .font(small.mono())
                                .color(tk.fg)
                                .wrap_anywhere()
                                .w(0.0)
                                .grow(1.0),
                        )
                        .child(
                            text(o.clone())
                                .font(small.mono())
                                .color(tk.fg)
                                .wrap_anywhere()
                                .w(0.0)
                                .grow(1.0),
                        )
                        .child(
                            text(format!("{n}/{total}"))
                                .font(small.mono())
                                .color(tk.fg)
                                .wrap_anywhere()
                                .w(0.0)
                                .grow(1.0),
                        ),
                );
            }
            let prev = kit::btn(
                tk,
                format!("{p}-prev"),
                Variant::Ghost,
                None,
                &t("chainAnalysis:workbench.previous"),
                shown > 0,
            );
            let next_ok = (shown + 1) * 50 < links.len();
            let next = kit::btn(
                tk,
                format!("{p}-next"),
                Variant::Ghost,
                None,
                &t("chainAnalysis:workbench.next"),
                next_ok,
            );
            d = d
                .child(
                    text(t("chainAnalysis:workbench.conditionalLinks"))
                        .font(small)
                        .color(tk.muted_fg)
                        .wrap()
                        .my(12.0),
                )
                .child(
                    col()
                        .w_full()
                        .max_h(288.0)
                        .child(table.scroll(rune::ScrollSpec {
                            stick_bottom: false,
                        })),
                )
                .child(
                    row()
                        .mt(8.0)
                        .gap(8.0)
                        .child(if shown > 0 {
                            prev.on_press(move |a: &mut App| ent(a, w).page = shown - 1)
                        } else {
                            prev
                        })
                        .child(if next_ok {
                            next.on_press(move |a: &mut App| ent(a, w).page = shown + 1)
                        } else {
                            next
                        }),
                );
        }
        c = c.child(d);
    }
    c
}

/// Polls a running entropy job every 500 ms.
fn poll_entropy(app: &mut App, tx: &mut Tx, w: Which) -> bool {
    let e = ent_ref(app, w);
    let Some(job) = &e.job else { return false };
    if job.get("status").and_then(Value::as_str) != Some("running") || e.poll_error || e.polling {
        return false;
    }
    if app.clock - e.last_poll < 0.5 {
        tx.wake_after(std::time::Duration::from_millis(100));
        return false;
    }
    let id = job.get("job_id").cloned().unwrap_or(Value::Null);
    if app
        .send(
            "ui.chain_analysis.jobs.get",
            json!({"job_id": id}),
            w.tag("get"),
        )
        .is_some()
    {
        let clock = app.clock;
        let e = ent(app, w);
        e.polling = true;
        e.last_poll = clock;
    }
    true
}

pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let mut changed = poll_entropy(app, tx, Which::Graph) | poll_entropy(app, tx, Which::Psbt);
    if app.provenance.chain.tab == "entropy"
        && app.provenance.chain.result.is_some()
        && app.provenance.chain.workspace == model::Workspace::Graph
    {
        let picked = picked_subject(app);
        if app.provenance.chain.entropy.synced_subject.as_deref() != Some(picked.as_str()) {
            sync_entropy(app, Which::Graph, &picked);
            changed = true;
        }
    }
    if app.provenance.chain.tab == "labels" && app.provenance.chain.result.is_some() {
        changed |= seed_labels(app);
    }
    changed
}

/// An entropy reply for instance `w` (`op`: start, get or cancel).
pub fn entropy_reply(app: &mut App, w: Which, op: &str, r: &Reply) {
    let e = ent(app, w);
    match op {
        "start" => e.starting = false,
        "get" => e.polling = false,
        _ => e.cancelling = false,
    }
    match &r.error {
        Some(err) => {
            if op == "get" {
                ent(app, w).poll_error = true;
            }
            report(app, err.clone());
        }
        None if r.data.is_object() => {
            let e = ent(app, w);
            e.job = Some(r.data.clone());
            e.page = 0;
            if w == Which::Psbt && op == "start" {
                super::psbt::on_entropy_job(app, &r.data);
            }
        }
        None => {}
    }
}

// ---- Labels -----------------------------------------------------------------------------

#[derive(Default)]
pub struct Labels {
    pub for_subject: Option<String>,
    pub list: Option<Value>,
    pub loading: bool,
    pub error: Option<String>,
    pub chain: String,
    pub network: String,
    pub category: String,
    pub confidence: String,
    pub confirm_delete: Option<String>,
    pub busy: bool,
}

fn load_labels(app: &mut App) {
    if app
        .send(
            "ui.chain_analysis.labels.list",
            Value::Null,
            "provenance.chain.labels.list",
        )
        .is_some()
    {
        st(app).labels.loading = true;
    }
}

/// Re-keys the form by the picked subject (`key={pickedSubject}`), reading the list again.
fn seed_labels(app: &mut App) -> bool {
    let picked = picked_subject(app);
    if app.provenance.chain.labels.for_subject.as_deref() == Some(picked.as_str()) {
        return false;
    }
    let s = &app.provenance.chain;
    let r = s.result.clone();
    let node = s
        .sel
        .as_ref()
        .filter(|x| !x.edge)
        .and_then(|x| r.as_ref()?.node(&x.id).cloned());
    let subject = node
        .as_ref()
        .map(|n| n.identity().to_owned())
        .unwrap_or_else(|| picked.clone());
    let chain = node
        .as_ref()
        .map(|n| n.chain.clone())
        .or_else(|| r.as_ref().and_then(|r| r.query.chain.clone()))
        .unwrap_or_else(|| "bitcoin".into());
    let network = model::network_input(
        node.as_ref()
            .map(|n| n.network.as_str())
            .or_else(|| r.as_ref().and_then(|r| r.query.network.as_deref())),
    );
    let list = app.provenance.chain.labels.list.take();
    app.provenance.chain.labels = Labels {
        for_subject: Some(picked),
        chain,
        network,
        category: "other".into(),
        confidence: "unverified".into(),
        list,
        ..Labels::default()
    };
    kit::set_val(app, "lbl.subject", &subject);
    kit::set_val(app, "lbl.label", "");
    kit::set_val(app, "lbl.source", "");
    load_labels(app);
    true
}

fn labels(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let l = &app.provenance.chain.labels;
    let busy = l.busy;
    let small = tk.txs(400.0);
    let (subject, label, source) = (
        val(app, "lbl.subject"),
        val(app, "lbl.label"),
        val(app, "lbl.source"),
    );
    let row1 = row()
        .w_full()
        .gap(12.0)
        .child(
            field(
                tk,
                &t("chainAnalysis:labels.subject"),
                input(
                    tk,
                    app,
                    "lbl.subject",
                    &t("chainAnalysis:labels.subject"),
                    "",
                    true,
                    32.0,
                    !busy,
                    None,
                ),
            )
            .w(0.0)
            .grow(2.0),
        )
        .child(
            row()
                .w(0.0)
                .grow(1.0)
                .gap(8.0)
                .child(
                    field(
                        tk,
                        &t("chainAnalysis:chain"),
                        select(
                            app,
                            tk,
                            layer,
                            "lbl-chain",
                            &t("chainAnalysis:chain"),
                            &l.chain,
                            vec![opt("bitcoin", "Bitcoin"), opt("liquid", "Liquid")],
                            !busy,
                            32.0,
                            |a: &mut App, v: &str| st(a).labels.chain = v.to_owned(),
                        ),
                    )
                    .w(0.0)
                    .grow(1.0),
                )
                .child(
                    field(
                        tk,
                        &t("chainAnalysis:network"),
                        select(
                            app,
                            tk,
                            layer,
                            "lbl-network",
                            &t("chainAnalysis:network"),
                            &l.network,
                            ["main", "test", "signet", "regtest"]
                                .iter()
                                .map(|n| opt(*n, *n))
                                .collect(),
                            !busy,
                            32.0,
                            |a: &mut App, v: &str| st(a).labels.network = v.to_owned(),
                        ),
                    )
                    .w(0.0)
                    .grow(1.0),
                ),
        );
    let row2 = row()
        .w_full()
        .gap(12.0)
        .child(
            field(
                tk,
                &t("chainAnalysis:labels.label"),
                input(
                    tk,
                    app,
                    "lbl.label",
                    &t("chainAnalysis:labels.label"),
                    "",
                    false,
                    32.0,
                    !busy,
                    None,
                ),
            )
            .w(0.0)
            .grow(1.0),
        )
        .child(
            field(
                tk,
                &t("chainAnalysis:labels.source"),
                input(
                    tk,
                    app,
                    "lbl.source",
                    &t("chainAnalysis:labels.source"),
                    "",
                    false,
                    32.0,
                    !busy,
                    None,
                ),
            )
            .w(0.0)
            .grow(2.0),
        );
    let enabled = !busy
        && !subject.trim().is_empty()
        && !label.trim().is_empty()
        && !source.trim().is_empty();
    let save = kit::btn(
        tk,
        "lbl-save",
        Variant::Outline,
        None,
        &t("chainAnalysis:labels.save"),
        enabled,
    );
    let row3 = row()
        .w_full()
        .gap(12.0)
        .items_end()
        .child(
            field(
                tk,
                &t("chainAnalysis:labels.category"),
                select(
                    app,
                    tk,
                    layer,
                    "lbl-category",
                    &t("chainAnalysis:labels.category"),
                    &l.category,
                    ["exchange", "merchant", "mixer", "service", "self", "other"]
                        .iter()
                        .map(|c| opt(*c, t(&format!("chainAnalysis:labels.{c}"))))
                        .collect(),
                    !busy,
                    32.0,
                    |a: &mut App, v: &str| st(a).labels.category = v.to_owned(),
                ),
            )
            .w(0.0)
            .grow(1.0),
        )
        .child(
            field(
                tk,
                &t("chainAnalysis:labels.confidence"),
                select(
                    app,
                    tk,
                    layer,
                    "lbl-confidence",
                    &t("chainAnalysis:labels.confidence"),
                    &l.confidence,
                    ["user_confirmed", "imported", "unverified"]
                        .iter()
                        .map(|c| opt(*c, t(&format!("chainAnalysis:labels.{c}"))))
                        .collect(),
                    !busy,
                    32.0,
                    |a: &mut App, v: &str| st(a).labels.confidence = v.to_owned(),
                ),
            )
            .w(0.0)
            .grow(1.0),
        )
        .child(row().w(0.0).grow(1.0).child(if enabled {
            save.on_press(save_label)
        } else {
            save
        }));
    let mut c = col()
        .w_full()
        .gap(12.0)
        .child(
            text(t("chainAnalysis:labels.scope"))
                .font(small)
                .color(tk.muted_fg)
                .wrap(),
        )
        .child(col().w_full().gap(12.0).child(row1).child(row2).child(row3));
    if let Some(e) = &l.error {
        c = c.child(kit::alert_text(tk, e.clone(), tk.tsm(400.0)));
    }
    let items: Vec<Value> = l
        .list
        .as_ref()
        .and_then(|v| v.get("items"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if items.is_empty() && !l.loading {
        c = c.child(
            text(t("chainAnalysis:labels.empty"))
                .font(small)
                .color(tk.muted_fg),
        );
    }
    let mut list = col().w_full().gap(8.0);
    for item in &items {
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let confirm = l.confirm_delete.as_deref() == Some(id.as_str());
        let label = t(if confirm {
            "chainAnalysis:labels.deleteConfirm"
        } else {
            "chainAnalysis:labels.delete"
        });
        let del = if confirm {
            kit::btn_destructive(tk, format!("lbl-del-{id}"), &label, !busy)
        } else {
            kit::btn(
                tk,
                format!("lbl-del-{id}"),
                Variant::Ghost,
                None,
                &label,
                !busy,
            )
        };
        let (did, rev) = (
            id.clone(),
            item.get("revision").cloned().unwrap_or(json!(0)),
        );
        let del = if busy {
            del
        } else {
            del.on_press(move |a: &mut App| {
                if a.provenance.chain.labels.confirm_delete.as_deref() != Some(did.as_str()) {
                    st(a).labels.confirm_delete = Some(did.clone());
                    return;
                }
                if a.send(
                    "ui.chain_analysis.labels.delete",
                    json!({"id": did, "expected_revision": rev}),
                    "provenance.chain.labels.delete",
                )
                .is_some()
                {
                    st(a).labels.busy = true;
                }
            })
        };
        list = list.child(
            card(tk)
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
                                .gap(4.0)
                                .child(
                                    text(
                                        item.get("label")
                                            .and_then(Value::as_str)
                                            .unwrap_or("")
                                            .to_owned(),
                                    )
                                    .font(tk.tsm(500.0))
                                    .color(tk.fg),
                                )
                                .child(
                                    text(
                                        item.get("subject")
                                            .and_then(Value::as_str)
                                            .unwrap_or("")
                                            .to_owned(),
                                    )
                                    .font(small.mono())
                                    .color(tk.muted_fg)
                                    .wrap_anywhere()
                                    .selectable(true),
                                ),
                        )
                        .child(del),
                )
                .child(kit::evidence(tk, app, &format!("ev-lbl-{id}"), item, None)),
        );
    }
    c.child(
        col()
            .w_full()
            .max_h(320.0)
            .child(list.scroll(rune::ScrollSpec {
                stick_bottom: false,
            })),
    )
}

fn save_label(a: &mut App) {
    let l = &a.provenance.chain.labels;
    if l.busy {
        return;
    }
    let args = json!({"subject": val(a, "lbl.subject").trim(), "chain": l.chain, "network": l.network, "label": val(a, "lbl.label").trim(), "source": val(a, "lbl.source").trim(), "category": l.category, "confidence": l.confidence, "cluster_defining": false});
    if a.send(
        "ui.chain_analysis.labels.upsert",
        args,
        "provenance.chain.labels.save",
    )
    .is_some()
    {
        st(a).labels.busy = true;
    }
}

pub fn on_reply(app: &mut App, sub: &str, r: &Reply) {
    match sub {
        "entropy.start" => entropy_reply(app, Which::Graph, "start", r),
        "entropy.get" => entropy_reply(app, Which::Graph, "get", r),
        "entropy.cancel" => entropy_reply(app, Which::Graph, "cancel", r),
        "labels.list" => {
            let l = &mut st(app).labels;
            l.loading = false;
            match &r.error {
                Some(e) => l.error = Some(e.clone()),
                None => {
                    l.list = Some(r.data.clone());
                    l.error = None;
                }
            }
        }
        "labels.save" | "labels.delete" => {
            st(app).labels.busy = false;
            match &r.error {
                Some(e) => report(app, e.clone()),
                None => {
                    if sub == "labels.save" {
                        kit::set_val(app, "lbl.label", "");
                        kit::set_val(app, "lbl.source", "");
                    } else {
                        st(app).labels.confirm_delete = None;
                    }
                    load_labels(app);
                }
            }
        }
        _ => {}
    }
}
