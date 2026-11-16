//! `DatasetsPanel` (Tools → Attribution datasets): validate a local attribution file as a
//! background job, import exactly the validated content hash, inspect, replace, revoke or clean
//! up versions, and look claims up. All local; nothing is uploaded.

use rune::{El, Tx, col, row, text};
use serde_json::{Map, Value, json};

use super::kit::{self, field, input, opt, select, val};
use super::model;
use super::{report, st};
use crate::app::{App, Reply};
use crate::i18n::{t, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::txdetail::widgets::Variant;
use crate::ui::ic;

const TEXT_FIELDS: [&str; 6] = [
    "dataset_key",
    "name",
    "version",
    "source",
    "license",
    "attribution_method",
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    Validate,
    Import,
    Discard,
}

impl JobKind {
    fn i(self) -> usize {
        self as usize
    }
    fn start_kind(self) -> &'static str {
        match self {
            JobKind::Validate => "ui.chain_analysis.datasets.preview.start",
            JobKind::Import => "ui.chain_analysis.datasets.import.start",
            JobKind::Discard => "ui.chain_analysis.datasets.discard.start",
        }
    }
    fn tag(self, op: &str) -> &'static str {
        match (self, op) {
            (JobKind::Validate, "start") => "provenance.chain.datasets.start.0",
            (JobKind::Import, "start") => "provenance.chain.datasets.start.1",
            (JobKind::Discard, "start") => "provenance.chain.datasets.start.2",
            (JobKind::Validate, "get") => "provenance.chain.datasets.get.0",
            (JobKind::Import, "get") => "provenance.chain.datasets.get.1",
            (JobKind::Discard, "get") => "provenance.chain.datasets.get.2",
            (JobKind::Validate, _) => "provenance.chain.datasets.cancel.0",
            (JobKind::Import, _) => "provenance.chain.datasets.cancel.1",
            (JobKind::Discard, _) => "provenance.chain.datasets.cancel.2",
        }
    }
    fn of(i: usize) -> JobKind {
        [JobKind::Validate, JobKind::Import, JobKind::Discard][i.min(2)]
    }
}

/// `useAnalysisJob`: one book-scoped job and its 500 ms status polls.
#[derive(Default, Clone)]
pub struct Job {
    pub job: Option<Value>,
    pub poll_error: bool,
    pub starting: bool,
    pub polling: bool,
    pub last_poll: f64,
    /// The completed job this panel already acted on.
    pub handled: String,
}

impl Job {
    fn busy(&self) -> bool {
        self.starting
            || self
                .job
                .as_ref()
                .and_then(|j| j.get("status"))
                .and_then(Value::as_str)
                == Some("running")
    }
}

#[derive(Default)]
pub struct Datasets {
    pub seeded: bool,
    pub source: Option<Value>,
    pub chain: String,
    pub network: String,
    pub visibility: String,
    pub format: String,
    pub adapter: String,
    pub preview: Option<(String, Value)>,
    pub submitted: String,
    pub notice: Option<String>,
    pub cursor: Option<String>,
    pub previous: Vec<Value>,
    pub page: Option<Value>,
    pub loading: bool,
    pub list_error: Option<String>,
    pub selected: Option<Value>,
    pub confirm_revoke: bool,
    pub inspecting: bool,
    pub revoking: bool,
    pub jobs: [Job; 3],
    pub q_for: Option<String>,
    pub q_mode: String,
    pub q_chain: String,
    pub q_network: String,
    pub q_observer: String,
    pub q_outcome: Option<(String, Value)>,
    pub q_busy: bool,
    pub q_pending_key: String,
}

fn ds(app: &mut App) -> &mut Datasets {
    &mut st(app).datasets
}

pub fn load_list(app: &mut App) {
    let mut args = json!({"limit": 50});
    if let Some(c) = &app.provenance.chain.datasets.cursor {
        args["cursor"] = json!(c);
    }
    if app
        .send(
            "ui.chain_analysis.datasets.list",
            args,
            "provenance.chain.datasets.list",
        )
        .is_some()
    {
        ds(app).loading = true;
    }
}

fn refresh(app: &mut App) {
    let d = ds(app);
    d.cursor = None;
    d.previous.clear();
    load_list(app);
}

fn manifest(app: &App) -> Map<String, Value> {
    let d = &app.provenance.chain.datasets;
    let mut m = Map::new();
    for f in ["dataset_key", "name", "version"] {
        m.insert(f.into(), json!(val(app, &format!("ds.{f}"))));
    }
    m.insert("chain".into(), json!(d.chain));
    m.insert("network".into(), json!(d.network));
    for f in ["source", "license", "attribution_method"] {
        m.insert(f.into(), json!(val(app, &format!("ds.{f}"))));
    }
    m.insert("visibility".into(), json!(d.visibility));
    let replaces = val(app, "ds.replaces");
    if !replaces.is_empty() {
        m.insert("expected_active_id".into(), json!(replaces));
    }
    m
}

fn preview_key(app: &App) -> String {
    let d = &app.provenance.chain.datasets;
    model::dataset_preview_key(
        d.source
            .as_ref()
            .and_then(|s| s.get("source_token"))
            .and_then(Value::as_str),
        &manifest(app),
        &d.format,
        &d.adapter,
    )
}

fn args(app: &App) -> Value {
    let d = &app.provenance.chain.datasets;
    json!({"source_token": d.source.as_ref().and_then(|s| s.get("source_token")), "manifest": Value::Object(manifest(app)), "format": d.format, "adapter": d.adapter})
}

/// `isCurrentDatasetPreview`.
fn ready(app: &App) -> bool {
    let key = preview_key(app);
    app.provenance
        .chain
        .datasets
        .preview
        .as_ref()
        .is_some_and(|(k, p)| {
            *k == key
                && p.get("validation").and_then(Value::as_str) == Some("complete")
                && p.get("sha256").and_then(Value::as_str).is_some_and(|h| {
                    h.len() == 64
                        && h.bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                })
        })
}

fn run_job(app: &mut App, kind: JobKind, a: Value) {
    let j = &mut ds(app).jobs[kind.i()];
    j.poll_error = false;
    j.job = None;
    if app.send(kind.start_kind(), a, kind.tag("start")).is_some() {
        ds(app).jobs[kind.i()].starting = true;
    }
}

fn pick(app: &mut App, tx: &mut Tx) {
    if app.read("ui.profiles.snapshot").is_none() {
        return;
    }
    let options = rune::FileDialogOptions::default().filter("Analysis source", &["csv", "jsonl"]);
    tx.open_file(options, |a: &mut App, result, _tx| match result {
        Ok(Some(path)) => {
            let args = json!({"source_file": path.to_string_lossy(), "purpose": "dataset", "expected_scope": a.expected_scope()});
            a.send("internal.chain_analysis.stage", args, "provenance.chain.stage.dataset");
        }
        Ok(None) => {}
        Err(e) => report(a, e.0),
    });
}

pub fn on_stage(app: &mut App, r: &Reply) {
    match &r.error {
        Some(e) => report(app, e.clone()),
        None if r.data.is_object() => {
            let d = ds(app);
            d.source = Some(r.data.clone());
            d.preview = None;
        }
        None => {}
    }
}

/// `JobProgress`.
fn job_progress(tk: &Tk, kind: JobKind, j: &Job) -> Option<El> {
    let job = j.job.as_ref()?;
    let small = tk.txs(400.0);
    let status = job.get("status").and_then(Value::as_str).unwrap_or("");
    let mut head = row()
        .w_full()
        .gap(12.0)
        .flex_wrap()
        .center_y()
        .justify_between()
        .child(
            text(format!(
                "{status} · {}",
                job.pointer("/progress/phase")
                    .and_then(Value::as_str)
                    .unwrap_or("")
            ))
            .font(small)
            .color(tk.fg)
            .role("status"),
        );
    if status == "running" {
        let requested = job
            .get("cancel_requested")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let id = job.get("job_id").cloned().unwrap_or(Value::Null);
        let b = kit::btn(
            tk,
            format!("ds-cancel-{}", kind.i()),
            Variant::Outline,
            None,
            &t(if requested {
                "chainAnalysis:workbench.cancelling"
            } else {
                "chainAnalysis:workbench.cancel"
            }),
            !requested,
        );
        head = head.child(if requested {
            b
        } else {
            b.on_press(move |a: &mut App| {
                a.send(
                    "ui.chain_analysis.jobs.cancel",
                    json!({"job_id": id}),
                    kind.tag("cancel"),
                );
            })
        });
    }
    let mut stats = row().gap(16.0).flex_wrap();
    for (k, key) in [
        ("row_count", "chainAnalysis:datasets.progressRows"),
        ("byte_count", "chainAnalysis:datasets.progressBytes"),
        ("rows_deleted", "chainAnalysis:datasets.progressDeleted"),
    ] {
        if let Some(n) = job
            .pointer(&format!("/progress/{k}"))
            .and_then(Value::as_u64)
        {
            stats = stats.child(
                text(crate::i18n::tc(key, n as usize, &[]))
                    .font(small.mono())
                    .color(tk.fg),
            );
        }
    }
    let ms = job
        .pointer("/progress/elapsed_ms")
        .filter(|v| !v.is_null())
        .or(job.get("elapsed_ms"))
        .map(Value::to_string)
        .unwrap_or_default();
    stats = stats.child(
        text(tr("chainAnalysis:workbench.elapsed", &[("ms", &ms)]))
            .font(small.mono())
            .color(tk.fg),
    );
    let mut c = col()
        .w_full()
        .padding(12.0)
        .gap(12.0)
        .radius(tk.r(0.5))
        .border(1.0, tk.border)
        .border_layout()
        .fill(tk.muted.alpha(0.15))
        .child(head)
        .child(stats);
    if j.poll_error {
        c = c.child(
            row().child(
                kit::btn(
                    tk,
                    format!("ds-resume-{}", kind.i()),
                    Variant::Outline,
                    None,
                    &t("chainAnalysis:workbench.resume"),
                    true,
                )
                .on_press(move |a: &mut App| ds(a).jobs[kind.i()].poll_error = false),
            ),
        );
    }
    if let Some(code) = job.get("error_code").and_then(Value::as_str) {
        c = c.child(kit::alert_text(tk, code.to_owned(), small));
    }
    Some(c)
}

/// `ClaimTable`.
fn claim_table(app: &App, tk: &Tk, prefix: &str, claims: &[Value]) -> El {
    let small = tk.txs(400.0);
    let head = |s: &str| {
        text(s.to_owned())
            .font(tk.txs(600.0))
            .color(tk.fg)
            .w(0.0)
            .grow(1.0)
    };
    let mut table = col().w_full().child(
        row()
            .w_full()
            .padding(8.0)
            .gap(8.0)
            .child(head(&t("chainAnalysis:datasets.subject")))
            .child(head(&t("chainAnalysis:datasets.label")))
            .child(head(&t("chainAnalysis:labels.category")))
            .child(head(&t("chainAnalysis:showEvidence"))),
    );
    for (i, c) in claims.iter().enumerate() {
        let g = |k: &str| c.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
        let k = format!("{prefix}-claim-{i}");
        let summary = c
            .get("dataset_status")
            .or(c.get("confidence"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| t("chainAnalysis:showEvidence"));
        let mut ev = col().w(0.0).grow(1.0).child(kit::summary_plain(
            tk,
            app,
            &k,
            text(summary).font(small).color(tk.fg),
        ));
        if kit::is_open(app, &k) {
            ev = ev.child(col().mt(4.0).child(kit::structured(tk, c, 0)));
        }
        table = table.child(
            row()
                .w_full()
                .padding(8.0)
                .gap(8.0)
                .items_start()
                .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
                .child(
                    text(g("subject"))
                        .font(small.mono())
                        .color(tk.fg)
                        .wrap_anywhere()
                        .w(0.0)
                        .grow(1.0),
                )
                .child(
                    text(g("label"))
                        .font(small)
                        .color(tk.fg)
                        .wrap()
                        .w(0.0)
                        .grow(1.0),
                )
                .child(
                    text(g("category"))
                        .font(small)
                        .color(tk.fg)
                        .w(0.0)
                        .grow(1.0),
                )
                .child(ev),
        );
    }
    col()
        .w_full()
        .max_h(256.0)
        .child(table.scroll(rune::ScrollSpec {
            stick_bottom: false,
        }))
}

pub fn view(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let d = &app.provenance.chain.datasets;
    let small = tk.txs(400.0);
    let busy = d.jobs.iter().any(Job::busy) || d.revoking;
    let can_ask = app.prefs.ai_features
        && !app.assistant.streaming
        && (d.selected.is_some() || d.source.is_some());
    let ask = kit::btn(
        tk,
        "ds-ask",
        Variant::Outline,
        Some(&I::SPARKLES),
        &t("chainAnalysis:ask"),
        can_ask,
    );
    let ask = if can_ask {
        ask.on_press(|a: &mut App| {
            let d = &a.provenance.chain.datasets;
            // UUID/token only: never the imported labels, source text, filename or identities.
            let prompt = match (&d.selected, &d.source) {
                (Some(s), _) => tr(
                    "chainAnalysis:datasets.assistantDataset",
                    &[("id", s.get("id").and_then(Value::as_str).unwrap_or(""))],
                ),
                (None, Some(src)) => tr(
                    "chainAnalysis:datasets.assistantSource",
                    &[(
                        "token",
                        src.get("source_token")
                            .and_then(Value::as_str)
                            .unwrap_or(""),
                    )],
                ),
                _ => return,
            };
            crate::assistant::hand_off(a, &prompt);
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
                                .child(ic(&I::DATABASE, 20.0, tk.fg))
                                .child(
                                    text(t("chainAnalysis:datasets.title"))
                                        .font(tk.tlg(600.0))
                                        .color(tk.fg)
                                        .role("heading")
                                        .level(2),
                                ),
                        )
                        .child(
                            text(t("chainAnalysis:datasets.help"))
                                .font(tk.tsm(400.0))
                                .color(tk.muted_fg)
                                .wrap()
                                .max_w(768.0),
                        ),
                )
                .child(ask),
        );
    for k in [JobKind::Validate, JobKind::Import, JobKind::Discard] {
        if let Some(p) = job_progress(tk, k, &d.jobs[k.i()]) {
            c = c.child(p);
        }
    }
    if let Some(n) = &d.notice {
        c = c.child(
            row()
                .w_full()
                .padding(12.0)
                .gap(8.0)
                .items_start()
                .radius(tk.r(0.5))
                .border(1.0, tk.border)
                .border_layout()
                .fill(tk.muted.alpha(0.2))
                .role("status")
                .child(
                    text(n.clone())
                        .font(small)
                        .color(tk.fg)
                        .wrap()
                        .grow(1.0)
                        .min_w(0.0),
                )
                .child(
                    crate::shell::transient::dismiss(
                        tk,
                        "ds-notice-dismiss",
                        tk.muted_fg,
                        |a: &mut App| ds(a).notice = None,
                    )
                    .mt(-6.0)
                    .mr(-6.0),
                ),
        );
    }
    // The form.
    let src = d
        .source
        .as_ref()
        .map(|s| {
            format!(
                "{} · {} B",
                s.get("filename").and_then(Value::as_str).unwrap_or(""),
                s.get("size_bytes")
                    .map(Value::to_string)
                    .unwrap_or_default()
            )
        })
        .unwrap_or_else(|| t("chainAnalysis:datasets.noFile"));
    let choose = kit::btn(
        tk,
        "ds-pick",
        Variant::Outline,
        None,
        &t("chainAnalysis:workbench.chooseFile"),
        !busy,
    );
    let mut form = col().w(0.0).grow(1.2).min_w(0.0).gap(16.0).child(
        col()
            .w_full()
            .padding(16.0)
            .radius(tk.r(0.5))
            .border(1.0, tk.border)
            .border_layout()
            .child(text(src).font(small).color(tk.muted_fg).wrap().mb(12.0))
            .child(row().child(if busy {
                choose
            } else {
                choose.on_press_tx(|a: &mut App, tx: &mut Tx| pick(a, tx))
            })),
    );
    let mut fields: Vec<El> = TEXT_FIELDS
        .iter()
        .map(|f| {
            field(
                tk,
                &t(&format!("chainAnalysis:datasets.manifest.{f}")),
                input(
                    tk,
                    app,
                    &format!("ds.{f}"),
                    &t(&format!("chainAnalysis:datasets.manifest.{f}")),
                    "",
                    false,
                    32.0,
                    !busy,
                    None,
                ),
            )
        })
        .collect();
    fields.push(field(
        tk,
        &t("chainAnalysis:chain"),
        select(
            app,
            tk,
            layer,
            "ds-chain",
            &t("chainAnalysis:chain"),
            &d.chain,
            vec![opt("bitcoin", "Bitcoin"), opt("liquid", "Liquid")],
            !busy,
            32.0,
            |a: &mut App, v: &str| ds(a).chain = v.to_owned(),
        ),
    ));
    let mut nets = vec![
        opt("main", "main"),
        opt("test", "test"),
        opt("regtest", "regtest"),
    ];
    if d.chain == "bitcoin" {
        nets.push(opt("signet", "signet"));
    }
    fields.push(field(
        tk,
        &t("chainAnalysis:network"),
        select(
            app,
            tk,
            layer,
            "ds-network",
            &t("chainAnalysis:network"),
            &d.network,
            nets,
            !busy,
            32.0,
            |a: &mut App, v: &str| ds(a).network = v.to_owned(),
        ),
    ));
    fields.push(field(
        tk,
        &t("chainAnalysis:datasets.visibility"),
        select(
            app,
            tk,
            layer,
            "ds-visibility",
            &t("chainAnalysis:datasets.visibility"),
            &d.visibility,
            vec![
                opt("private", t("chainAnalysis:datasets.private")),
                opt("public", t("chainAnalysis:datasets.public")),
            ],
            !busy,
            32.0,
            |a: &mut App, v: &str| ds(a).visibility = v.to_owned(),
        ),
    ));
    fields.push(field(
        tk,
        &t("chainAnalysis:datasets.format"),
        select(
            app,
            tk,
            layer,
            "ds-format",
            &t("chainAnalysis:datasets.format"),
            &d.format,
            vec![opt("csv", "CSV"), opt("jsonl", "JSONL")],
            !busy,
            32.0,
            |a: &mut App, v: &str| ds(a).format = v.to_owned(),
        ),
    ));
    fields.push(field(
        tk,
        &t("chainAnalysis:datasets.adapter"),
        select(
            app,
            tk,
            layer,
            "ds-adapter",
            &t("chainAnalysis:datasets.adapter"),
            &d.adapter,
            ["generic", "am_i_exposed", "maru92"]
                .iter()
                .map(|x| opt(*x, *x))
                .collect(),
            !busy,
            32.0,
            |a: &mut App, v: &str| ds(a).adapter = v.to_owned(),
        ),
    ));
    fields.push(field(
        tk,
        &t("chainAnalysis:datasets.replaces"),
        input(
            tk,
            app,
            "ds.replaces",
            &t("chainAnalysis:datasets.replaces"),
            "",
            true,
            32.0,
            !busy,
            None,
        ),
    ));
    let mut grid = col().w_full().gap(12.0);
    let mut it = fields.into_iter();
    while let Some(a) = it.next() {
        let b = it.next().unwrap_or_else(col);
        grid = grid.child(
            row()
                .w_full()
                .gap(12.0)
                .child(a.w(0.0).grow(1.0))
                .child(b.w(0.0).grow(1.0)),
        );
    }
    let required_ok = TEXT_FIELDS
        .iter()
        .all(|f| !val(app, &format!("ds.{f}")).is_empty());
    let can_validate = d.source.is_some() && !busy && required_ok;
    let validate = kit::btn(
        tk,
        "ds-validate",
        Variant::Default,
        None,
        &t(if d.jobs[0].busy() {
            "chainAnalysis:datasets.validating"
        } else {
            "chainAnalysis:datasets.preview"
        }),
        d.source.is_some() && !busy,
    );
    form = form
        .child(grid)
        .child(
            text(t("chainAnalysis:datasets.visibilityHelp"))
                .font(small)
                .color(tk.muted_fg)
                .wrap(),
        )
        .child(
            text(t("chainAnalysis:datasets.formatHelp"))
                .font(small)
                .color(tk.muted_fg)
                .wrap(),
        )
        .child(row().child(if can_validate {
            validate.on_press(|a: &mut App| {
                let key = preview_key(a);
                let args = args(a);
                let d = ds(a);
                d.preview = None;
                d.notice = None;
                d.submitted = key;
                run_job(a, JobKind::Validate, args);
            })
        } else {
            validate
        }));
    let is_ready = ready(app);
    if d.preview.is_some() && !is_ready {
        form = form.child(
            text(t("chainAnalysis:datasets.previewChanged"))
                .font(small)
                .color(tk.muted_fg)
                .role("status"),
        );
    }
    if let Some((_, p)) = d.preview.as_ref().filter(|_| is_ready) {
        let f = |label: &str, v: &Value| {
            kit::fact(tk, label, v)
                .map(|e| e.w(0.0).grow(1.0))
                .unwrap_or_else(|| col().w(0.0).grow(1.0))
        };
        let hash = p.get("sha256").cloned().unwrap_or(Value::Null);
        let importing = d.jobs[1].busy();
        let imp = kit::btn(
            tk,
            "ds-import",
            Variant::Default,
            None,
            &t(if importing {
                "chainAnalysis:datasets.importing"
            } else {
                "chainAnalysis:datasets.confirmImport"
            }),
            !busy,
        );
        let h = hash.clone();
        form = form.child(
            col()
                .w_full()
                .padding(16.0)
                .gap(12.0)
                .radius(tk.r(0.5))
                .border(1.0, tk.border)
                .border_layout()
                .fill(tk.muted.alpha(0.15))
                .child(
                    text(t("chainAnalysis:datasets.validated"))
                        .font(tk.tsm(500.0))
                        .color(tk.fg),
                )
                .child(
                    row()
                        .w_full()
                        .gap(12.0)
                        .child(f(
                            &t("chainAnalysis:datasets.rows"),
                            p.get("row_count").unwrap_or(&Value::Null),
                        ))
                        .child(f(
                            &t("chainAnalysis:datasets.bytes"),
                            p.get("byte_count").unwrap_or(&Value::Null),
                        )),
                )
                .children(kit::fact(tk, "SHA-256", &hash))
                .child(claim_table(
                    app,
                    tk,
                    "prev",
                    p.get("sample_claims")
                        .and_then(Value::as_array)
                        .map(Vec::as_slice)
                        .unwrap_or(&[]),
                ))
                .child(
                    text(t("chainAnalysis:datasets.confirmHelp"))
                        .font(small)
                        .color(tk.muted_fg)
                        .wrap(),
                )
                .child(row().child(if busy {
                    imp
                } else {
                    imp.on_press(move |a: &mut App| {
                        let mut args = args(a);
                        args["expected_sha256"] = h.clone();
                        ds(a).preview = None;
                        run_job(a, JobKind::Import, args);
                    })
                })),
        );
    }
    // Installed versions.
    let mut items = d.previous.clone();
    items.extend(
        d.page
            .as_ref()
            .and_then(|p| p.get("items"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
    );
    let mut installed = col().w(0.0).grow(1.0).min_w(0.0).gap(12.0).child(
        row()
            .w_full()
            .gap(12.0)
            .center_y()
            .justify_between()
            .child(
                text(t("chainAnalysis:datasets.installed"))
                    .font(tk.tsm(500.0))
                    .color(tk.fg),
            )
            .child({
                let b = kit::btn(
                    tk,
                    "ds-refresh",
                    Variant::Ghost,
                    None,
                    &t("chainAnalysis:datasets.refresh"),
                    !d.loading && !busy,
                );
                if !d.loading && !busy {
                    b.on_press(refresh)
                } else {
                    b
                }
            }),
    );
    if let Some(e) = &d.list_error {
        installed = installed.child(kit::alert_text(tk, e.clone(), small));
    }
    let mut list = col()
        .w_full()
        .radius(tk.r(0.5))
        .border(1.0, tk.border)
        .border_layout();
    if items.is_empty() {
        list = list.child(
            text(t("chainAnalysis:datasets.empty"))
                .font(small)
                .color(tk.muted_fg)
                .padding(16.0),
        );
    }
    for (i, pack) in items.iter().enumerate() {
        let id = pack
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let g = |k: &str| {
            pack.get(k)
                .map(|v| {
                    v.as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| v.to_string())
                })
                .unwrap_or_default()
        };
        let mut r = col()
            .key(format!("ds-pack-{id}"))
            .w_full()
            .padding(12.0)
            .hover(tk.muted.alpha(0.3))
            .role("button")
            .label(
                pack.pointer("/manifest/name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
            )
            .focusable(!d.inspecting)
            .child(
                row()
                    .gap(4.0)
                    .child(
                        text(
                            pack.pointer("/manifest/name")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_owned(),
                        )
                        .font(tk.tsm(500.0))
                        .color(tk.fg),
                    )
                    .child(text(g("version")).font(tk.tsm(500.0)).color(tk.muted_fg)),
            )
            .child(
                text(format!(
                    "{} · {} · {}",
                    g("status"),
                    g("visibility"),
                    g("row_count")
                ))
                .font(tk.px(10.0, 400.0))
                .color(tk.muted_fg)
                .mt(4.0),
            );
        if i > 0 {
            r = r.border_sides([1.0, 0.0, 0.0, 0.0], tk.border);
        }
        if !d.inspecting {
            r = r.on_press(move |a: &mut App| {
                if a.send(
                    "ui.chain_analysis.datasets.get",
                    json!({"id": id}),
                    "provenance.chain.datasets.inspect",
                )
                .is_some()
                {
                    ds(a).inspecting = true;
                }
            });
        }
        list = list.child(r);
    }
    installed = installed.child(
        col()
            .w_full()
            .max_h(320.0)
            .child(list.scroll(rune::ScrollSpec {
                stick_bottom: false,
            })),
    );
    if let Some(next) = d
        .page
        .as_ref()
        .and_then(|p| p.get("next_cursor"))
        .and_then(Value::as_str)
        .map(str::to_owned)
    {
        installed = installed.child(
            row().child(
                kit::btn(
                    tk,
                    "ds-next",
                    Variant::Ghost,
                    None,
                    &t("chainAnalysis:workbench.next"),
                    !d.loading,
                )
                .on_press(move |a: &mut App| {
                    let d = ds(a);
                    let page: Vec<Value> = d
                        .page
                        .as_ref()
                        .and_then(|p| p.get("items"))
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    d.previous.extend(page);
                    d.cursor = Some(next.clone());
                    load_list(a);
                }),
            ),
        );
    }
    if let Some(sel) = &d.selected {
        installed = installed.child(selected_view(app, tk, sel, busy));
    }
    c = c.child(
        row()
            .w_full()
            .gap(20.0)
            .items_start()
            .child(form)
            .child(installed),
    );
    c.child(query_view(app, tk, layer))
}

fn selected_view(_app: &App, tk: &Tk, sel: &Value, busy: bool) -> El {
    let d_confirm = _app.provenance.chain.datasets.confirm_revoke;
    let status = sel
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let mut c = col()
        .w_full()
        .padding(16.0)
        .gap(12.0)
        .radius(tk.r(0.5))
        .border(1.0, tk.border)
        .border_layout()
        .child(
            text(format!(
                "{} · {}",
                sel.pointer("/manifest/name")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
                sel.get("version").and_then(Value::as_str).unwrap_or("")
            ))
            .font(tk.tsm(500.0))
            .color(tk.fg),
        )
        .child(
            col()
                .gap(12.0)
                .children(kit::fact(
                    tk,
                    &t("chainAnalysis:id"),
                    sel.get("id").unwrap_or(&Value::Null),
                ))
                .children(kit::fact(
                    tk,
                    &t("chainAnalysis:status"),
                    sel.get("status").unwrap_or(&Value::Null),
                ))
                .children(kit::fact(
                    tk,
                    "SHA-256",
                    sel.get("content_sha256").unwrap_or(&Value::Null),
                )),
        )
        .child(kit::structured(
            tk,
            sel.get("manifest").unwrap_or(&Value::Null),
            0,
        ));
    let id = sel.get("id").cloned().unwrap_or(Value::Null);
    let dismiss = || {
        kit::btn(
            tk,
            "ds-dismiss",
            Variant::Ghost,
            None,
            &t("chainAnalysis:dismiss"),
            true,
        )
        .on_press(|a: &mut App| ds(a).confirm_revoke = false)
    };
    if status == "failed" || status == "staging" {
        let label = t(if d_confirm {
            "chainAnalysis:datasets.confirmDiscard"
        } else {
            "chainAnalysis:datasets.discard"
        });
        let b = if d_confirm {
            kit::btn_destructive(tk, "ds-discard", &label, !busy)
        } else {
            kit::btn(tk, "ds-discard", Variant::Outline, None, &label, !busy)
        };
        let mut r = row().gap(8.0).child(if busy {
            b
        } else {
            b.on_press(move |a: &mut App| {
                if !a.provenance.chain.datasets.confirm_revoke {
                    ds(a).confirm_revoke = true;
                    return;
                }
                run_job(a, JobKind::Discard, json!({"id": id}));
            })
        });
        if d_confirm {
            r = r.child(dismiss());
        }
        c = c.child(
            col()
                .gap(8.0)
                .child(
                    text(t("chainAnalysis:datasets.discardHelp"))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .wrap(),
                )
                .child(r),
        );
    } else if status == "active" || status == "superseded" {
        let replace = kit::btn(
            tk,
            "ds-replace",
            Variant::Outline,
            None,
            &t("chainAnalysis:datasets.replace"),
            !busy && status == "active",
        );
        let s2 = sel.clone();
        let label = t(if d_confirm {
            "chainAnalysis:datasets.confirmRevoke"
        } else {
            "chainAnalysis:datasets.revoke"
        });
        let b = if d_confirm {
            kit::btn_destructive(tk, "ds-revoke", &label, !busy)
        } else {
            kit::btn(tk, "ds-revoke", Variant::Outline, None, &label, !busy)
        };
        let rev = sel.get("revision").cloned().unwrap_or(json!(0));
        let mut r = row()
            .gap(8.0)
            .flex_wrap()
            .child(if !busy && status == "active" {
                replace.on_press(move |a: &mut App| {
                    let m = model::dataset_replacement_manifest(&s2);
                    for f in TEXT_FIELDS {
                        kit::set_val(
                            a,
                            &format!("ds.{f}"),
                            m.get(f).and_then(Value::as_str).unwrap_or(""),
                        );
                    }
                    kit::set_val(
                        a,
                        "ds.replaces",
                        m.get("expected_active_id")
                            .and_then(Value::as_str)
                            .unwrap_or(""),
                    );
                    let d = ds(a);
                    d.chain = m
                        .get("chain")
                        .and_then(Value::as_str)
                        .unwrap_or("bitcoin")
                        .to_owned();
                    d.network = m
                        .get("network")
                        .and_then(Value::as_str)
                        .unwrap_or("main")
                        .to_owned();
                    d.visibility = m
                        .get("visibility")
                        .and_then(Value::as_str)
                        .unwrap_or("private")
                        .to_owned();
                    d.preview = None;
                })
            } else {
                replace
            })
            .child(if busy {
                b
            } else {
                b.on_press(move |a: &mut App| {
                    if !a.provenance.chain.datasets.confirm_revoke {
                        ds(a).confirm_revoke = true;
                        return;
                    }
                    if a.send(
                        "ui.chain_analysis.datasets.revoke",
                        json!({"id": id, "expected_revision": rev}),
                        "provenance.chain.datasets.revoke",
                    )
                    .is_some()
                    {
                        ds(a).revoking = true;
                    }
                })
            });
        if d_confirm {
            r = r.child(dismiss());
        }
        c = c.child(r);
    }
    c
}

/// `DatasetQuery`: exact claim lookup, scoped to the selected dataset when there is one.
fn query_args(app: &App) -> Value {
    let d = &app.provenance.chain.datasets;
    let v = val(app, "dsq.value").trim().to_owned();
    let mut a = if d.q_mode == "label" {
        json!({"label": v})
    } else {
        json!({"subject": v, "chain": d.q_chain, "network": d.q_network})
    };
    if let Some(id) = d.selected.as_ref().and_then(|s| s.get("id")) {
        a["dataset_id"] = id.clone();
    }
    a["observer"] = json!(d.q_observer);
    a["limit"] = json!(50);
    a
}

fn query_view(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let d = &app.provenance.chain.datasets;
    let small = tk.txs(400.0);
    let has_dataset = d.selected.is_some();
    let value = val(app, "dsq.value");
    let mut form = row().w_full().gap(12.0).flex_wrap().items_end().child(
        field(
            tk,
            &t("chainAnalysis:datasets.matchBy"),
            select(
                app,
                tk,
                layer,
                "dsq-mode",
                &t("chainAnalysis:datasets.matchBy"),
                &d.q_mode,
                vec![
                    opt("subject", t("chainAnalysis:datasets.subject")),
                    kit::Opt {
                        value: "label".into(),
                        label: t("chainAnalysis:datasets.label"),
                        disabled: !has_dataset,
                    },
                ],
                true,
                32.0,
                |a: &mut App, v: &str| ds(a).q_mode = v.to_owned(),
            ),
        )
        .w(140.0),
    );
    let label = t(if d.q_mode == "subject" {
        "chainAnalysis:datasets.subject"
    } else {
        "chainAnalysis:datasets.label"
    });
    form = form.child(
        field(
            tk,
            &label,
            input(tk, app, "dsq.value", &label, "", false, 32.0, true, None),
        )
        .min_w(240.0)
        .grow(1.0),
    );
    if d.q_mode == "subject" {
        let mut nets = vec![
            opt("main", "main"),
            opt("test", "test"),
            opt("regtest", "regtest"),
        ];
        if d.q_chain == "bitcoin" {
            nets.push(opt("signet", "signet"));
        }
        form = form
            .child(
                field(
                    tk,
                    &t("chainAnalysis:chain"),
                    select(
                        app,
                        tk,
                        layer,
                        "dsq-chain",
                        &t("chainAnalysis:chain"),
                        &d.q_chain,
                        vec![opt("bitcoin", "Bitcoin"), opt("liquid", "Liquid")],
                        true,
                        32.0,
                        |a: &mut App, v: &str| ds(a).q_chain = v.to_owned(),
                    ),
                )
                .w(120.0),
            )
            .child(
                field(
                    tk,
                    &t("chainAnalysis:network"),
                    select(
                        app,
                        tk,
                        layer,
                        "dsq-network",
                        &t("chainAnalysis:network"),
                        &d.q_network,
                        nets,
                        true,
                        32.0,
                        |a: &mut App, v: &str| ds(a).q_network = v.to_owned(),
                    ),
                )
                .w(110.0),
            );
    }
    form = form.child(
        field(
            tk,
            &t("chainAnalysis:workbench.observer"),
            select(
                app,
                tk,
                layer,
                "dsq-observer",
                &t("chainAnalysis:workbench.observer"),
                &d.q_observer,
                vec![
                    opt("owner", t("chainAnalysis:workbench.owner")),
                    opt("public", t("chainAnalysis:workbench.public")),
                ],
                true,
                32.0,
                |a: &mut App, v: &str| ds(a).q_observer = v.to_owned(),
            ),
        )
        .w(190.0),
    );
    let enabled = !d.q_busy && !value.trim().is_empty();
    let search = kit::btn(
        tk,
        "dsq-search",
        Variant::Default,
        None,
        &t("chainAnalysis:datasets.search"),
        enabled,
    );
    form = form.child(if enabled {
        search.on_press(|a: &mut App| run_query(a, None))
    } else {
        search
    });
    let mut c = col()
        .w_full()
        .pt(20.0)
        .gap(12.0)
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
        .child(
            text(t("chainAnalysis:datasets.query"))
                .font(tk.tsm(500.0))
                .color(tk.fg),
        )
        .child(
            text(t("chainAnalysis:datasets.queryHelp"))
                .font(small)
                .color(tk.muted_fg)
                .wrap(),
        )
        .child(form);
    let key = query_args(app).to_string();
    if let Some((k, out)) = &d.q_outcome
        && *k == key
    {
        let items = out
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        c = c.child(claim_table(app, tk, "q", &items));
        if items.is_empty() {
            c = c.child(
                text(t("chainAnalysis:datasets.noMatch"))
                    .font(small)
                    .color(tk.muted_fg),
            );
        }
        if let Some(next) = out
            .get("next_cursor")
            .and_then(Value::as_str)
            .map(str::to_owned)
        {
            let b = kit::btn(
                tk,
                "dsq-next",
                Variant::Outline,
                None,
                &t("chainAnalysis:workbench.next"),
                !d.q_busy,
            );
            c = c.child(row().child(if d.q_busy {
                b
            } else {
                b.on_press(move |a: &mut App| run_query(a, Some(next.clone())))
            }));
        }
    }
    c
}

fn run_query(a: &mut App, cursor: Option<String>) {
    let d = &a.provenance.chain.datasets;
    if val(a, "dsq.value").trim().is_empty()
        || d.q_busy
        || (d.q_mode == "label" && d.selected.is_none())
    {
        return;
    }
    let base = query_args(a);
    let key = base.to_string();
    let mut args = base;
    if let Some(c) = cursor {
        args["cursor"] = json!(c);
    }
    if a.send(
        "ui.chain_analysis.datasets.query",
        args,
        "provenance.chain.datasets.query",
    )
    .is_some()
    {
        let d = ds(a);
        d.q_busy = true;
        d.q_pending_key = key;
    }
}

/// Seeds defaults, resets the claim lookup when the selected dataset changes, and polls jobs.
pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let mut changed = super::psbt::poll(app);
    if !app.provenance.chain.datasets.seeded {
        for f in TEXT_FIELDS {
            kit::set_val(app, &format!("ds.{f}"), "");
        }
        kit::set_val(app, "ds.replaces", "");
        kit::set_val(app, "dsq.value", "");
        let d = ds(app);
        d.seeded = true;
        d.chain = "bitcoin".into();
        d.network = "main".into();
        d.visibility = "private".into();
        d.format = "csv".into();
        d.adapter = "generic".into();
        d.q_mode = "subject".into();
        d.q_chain = "bitcoin".into();
        d.q_network = "main".into();
        d.q_observer = "owner".into();
        changed = true;
    }
    let sel_id = app
        .provenance
        .chain
        .datasets
        .selected
        .as_ref()
        .and_then(|s| s.get("id"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| "all".into());
    if app.provenance.chain.datasets.q_for.as_deref() != Some(sel_id.as_str()) {
        kit::set_val(app, "dsq.value", "");
        let d = ds(app);
        d.q_for = Some(sel_id);
        d.q_mode = "subject".into();
        d.q_chain = "bitcoin".into();
        d.q_network = "main".into();
        d.q_observer = "owner".into();
        d.q_outcome = None;
        changed = true;
    }
    for i in 0..3 {
        let kind = JobKind::of(i);
        let j = &app.provenance.chain.datasets.jobs[i];
        let Some(job) = &j.job else { continue };
        if job.get("status").and_then(Value::as_str) != Some("running") || j.poll_error || j.polling
        {
            continue;
        }
        if app.clock - j.last_poll < 0.5 {
            tx.wake_after(std::time::Duration::from_millis(100));
            continue;
        }
        let id = job.get("job_id").cloned().unwrap_or(Value::Null);
        if app
            .send(
                "ui.chain_analysis.jobs.get",
                json!({"job_id": id}),
                kind.tag("get"),
            )
            .is_some()
        {
            let clock = app.clock;
            let j = &mut ds(app).jobs[i];
            j.polling = true;
            j.last_poll = clock;
            changed = true;
        }
    }
    changed
}

/// A job's newest state: a completed validation becomes the preview, a completed import the
/// selection, a completed discard clears it.
fn on_job(app: &mut App, i: usize, job: Value) {
    let done = job.get("status").and_then(Value::as_str) == Some("completed");
    let id = job
        .get("job_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let result = job.get("result").cloned().filter(|r| r.is_object());
    ds(app).jobs[i].job = Some(job);
    if !done || ds(app).jobs[i].handled == id {
        return;
    }
    match (JobKind::of(i), result) {
        (JobKind::Validate, Some(r)) => {
            ds(app).jobs[i].handled = id;
            let key = app.provenance.chain.datasets.submitted.clone();
            ds(app).preview = Some((key, r));
        }
        (JobKind::Import, Some(r)) => {
            ds(app).jobs[i].handled = id;
            let d = ds(app);
            d.selected = Some(r);
            d.notice = Some(t("chainAnalysis:datasets.imported"));
            refresh(app);
        }
        (JobKind::Discard, Some(r))
            if r.get("discarded").and_then(Value::as_bool) == Some(true) =>
        {
            ds(app).jobs[i].handled = id;
            let d = ds(app);
            d.selected = None;
            d.confirm_revoke = false;
            refresh(app);
        }
        _ => {}
    }
}

pub fn on_reply(app: &mut App, sub: &str, r: &Reply) {
    let rest = sub.strip_prefix("datasets.").unwrap_or(sub);
    match rest {
        "list" => {
            let d = ds(app);
            d.loading = false;
            match &r.error {
                Some(e) => d.list_error = Some(e.clone()),
                None => {
                    d.page = Some(r.data.clone());
                    d.list_error = None;
                }
            }
        }
        "inspect" => {
            ds(app).inspecting = false;
            match &r.error {
                Some(e) => report(app, e.clone()),
                None if r.data.is_object() => {
                    let d = ds(app);
                    d.selected = Some(r.data.clone());
                    d.confirm_revoke = false;
                }
                None => {}
            }
        }
        "revoke" => {
            ds(app).revoking = false;
            match &r.error {
                Some(e) => report(app, e.clone()),
                None => {
                    let d = ds(app);
                    if r.data.is_object() {
                        d.selected = Some(r.data.clone());
                    }
                    d.confirm_revoke = false;
                    refresh(app);
                }
            }
        }
        "query" => {
            ds(app).q_busy = false;
            match &r.error {
                Some(e) => report(app, e.clone()),
                None if r.data.is_object() => {
                    let d = ds(app);
                    let key = d.q_pending_key.clone();
                    d.q_outcome = Some((key, r.data.clone()));
                }
                None => {}
            }
        }
        other => {
            let Some((op, i)) = other.split_once('.') else {
                return;
            };
            let Ok(i) = i.parse::<usize>() else { return };
            if i > 2 {
                return;
            }
            match op {
                "start" => ds(app).jobs[i].starting = false,
                "get" => ds(app).jobs[i].polling = false,
                _ => {}
            }
            match &r.error {
                Some(e) => {
                    if op == "get" {
                        ds(app).jobs[i].poll_error = true;
                    }
                    if op == "start" && i == 1 {
                        ds(app).preview = None;
                    }
                    report(app, e.clone());
                }
                None if r.data.is_object() => on_job(app, i, r.data.clone()),
                None => {}
            }
        }
    }
}
