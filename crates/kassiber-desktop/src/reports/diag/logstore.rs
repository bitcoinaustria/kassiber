//! The Logs screen's RAM ring (lib/appLogs.ts) and the sources folded into it, as the web
//! folds them (lib/daemonLogBridge.ts, daemon/transport.ts):
//! - the daemon's own RAM ring, read with `ui.logs.snapshot` (a local read; the web polls it
//!   every 4 s app-wide, this app while the Logs screen is open, catching up on entry);
//! - the app's transport records ("Daemon invoke finished"), derived from `App::timings`.
//!
//! Nothing here touches the disk: records live in `app.reports.diag.logs` until cleared, and
//! leave the process only through an explicit export.

use std::collections::VecDeque;
use std::time::Duration;

use serde_json::{Value, json};

use super::ordjson::{self, J, obj, v};
use super::redact::{self, Mode, Opts};
use super::regex::Regex;

pub const MAX_RECORDS: usize = 10_000;
pub const MAX_BYTES: usize = 4 * 1024 * 1024;
pub const BRIDGE_INTERVAL: f64 = 4.0;
pub const BRIDGE_BACKOFF: f64 = 30.0;
pub const PAGE_LIMIT: usize = 500;

/// The transport wrapper's source (`withDaemonLogging(tauriDaemon, "daemon:tauri")`).
pub const TRANSPORT: &str = "daemon:tauri";

pub const LEVELS: [&str; 5] = ["error", "warning", "info", "debug", "trace"];

#[derive(Clone, Debug)]
pub struct Field {
    pub ty: String,
    pub value: Value,
}

#[derive(Clone, Debug)]
pub struct Record {
    pub id: String,
    pub ts: String,
    pub level: String,
    pub module: String,
    pub file: String,
    pub line: i64,
    pub msg: String,
    pub fields: Vec<(String, Field)>,
}

impl Record {
    fn bytes(&self) -> usize {
        ordjson::compact(&self.to_j()).len() + 1
    }

    /// The record as `JSON.stringify` writes it (keys in the web's order).
    pub fn to_j(&self) -> J {
        let fields = J::Obj(
            self.fields
                .iter()
                .map(|(k, f)| {
                    (
                        k.clone(),
                        obj(vec![
                            ("type", v(f.ty.clone())),
                            ("value", J::V(f.value.clone())),
                        ]),
                    )
                })
                .collect(),
        );
        obj(vec![
            ("id", v(self.id.clone())),
            ("ts", v(self.ts.clone())),
            ("level", v(self.level.clone())),
            ("module", v(self.module.clone())),
            ("file", v(self.file.clone())),
            ("line", v(self.line)),
            ("msg", v(self.msg.clone())),
            ("fields", fields),
        ])
    }

    /// `redactLogRecord`.
    pub fn redacted(&self, o: &Opts) -> Record {
        if !o.redacted {
            return self.clone();
        }
        let mut used: Vec<(String, usize)> = Vec::new();
        let fields = self
            .fields
            .iter()
            .map(|(name, f)| {
                let base = redact::field_name(name, &f.ty);
                let seen = match used.iter_mut().find(|(n, _)| *n == base) {
                    Some((_, c)) => {
                        *c += 1;
                        *c - 1
                    }
                    None => {
                        used.push((base.clone(), 1));
                        0
                    }
                };
                let rendered = if seen == 0 {
                    base
                } else {
                    format!("{base}_{}", seen + 1)
                };
                let (ty, value) = redact::field(&f.ty, &f.value, o);
                (rendered, Field { ty, value })
            })
            .collect();
        Record {
            msg: redact::text_for_mode(&self.msg, o),
            fields,
            ..self.clone()
        }
    }

    pub fn field_str(&self, name: &str) -> String {
        self.fields
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, f)| match &f.value {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => String::new(),
            })
            .unwrap_or_default()
    }
}

/// `formatFields`.
pub fn format_fields(fields: &[(String, Field)]) -> String {
    fields
        .iter()
        .map(|(k, f)| format!("{k}={}", redact::stringify(&f.value)))
        .collect::<Vec<_>>()
        .join(" ")
}

/// `formatLogRecord`.
pub fn format_record(r: &Record, o: &Opts) -> String {
    let r = r.redacted(o);
    let fields = format_fields(&r.fields);
    let parts = [
        iso_time(&r.ts),
        format!("{:<7}", r.level.to_uppercase()),
        format!("{}:{}:{}", r.module, r.file, r.line),
        r.msg.clone(),
        fields,
    ];
    parts
        .into_iter()
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// `new Date(ts).toISOString()`: always milliseconds and `Z`.
pub fn iso_time(ts: &str) -> String {
    match crate::format::parse_ts(ts) {
        Some(secs) => iso_from_secs(secs),
        None => ts.to_owned(),
    }
}

pub fn iso_from_secs(secs: f64) -> String {
    let ms = (secs * 1000.0).round() as i64;
    let days = ms.div_euclid(86_400_000);
    let rem = ms.rem_euclid(86_400_000);
    let d = crate::format::Day::from_epoch_days(days);
    format!(
        "{}T{:02}:{:02}:{:02}.{:03}Z",
        d.iso(),
        rem / 3_600_000,
        rem / 60_000 % 60,
        rem / 1000 % 60,
        rem % 1000
    )
}

pub fn now_iso() -> String {
    iso_from_secs(crate::format::now_secs())
}

/// The ring and its feeders.
#[derive(Default)]
pub struct Store {
    pub records: VecDeque<Record>,
    pub bytes: usize,
    next_local: u64,
    /// `ui.logs.snapshot` cursor and the daemon run it belongs to.
    pub cursor: u64,
    started_at: Option<String>,
    pub in_flight: bool,
    pub failing: bool,
    /// App clock time of the next bridge poll.
    pub next_poll: f64,
    pages_this_tick: usize,
    /// The last `App::timings` entry already folded in.
    last_timing: Option<(String, Duration)>,
    /// Request ids already logged as started.
    started: std::collections::HashSet<String>,
    /// Bumped on every change, so cached renders know to rebuild.
    pub generation: u64,
}

impl Store {
    pub fn new() -> Store {
        Store {
            bytes: 2,
            ..Default::default()
        }
    }

    pub fn clear(&mut self) {
        self.records.clear();
        self.bytes = 2;
        self.generation += 1;
    }

    /// `emitAppLog`: secret floor at insert, then the ring bounds.
    pub fn emit(&mut self, mut r: Record) {
        if r.id.is_empty() {
            self.next_local += 1;
            r.id = format!("log-{}", self.next_local);
        }
        if r.ts.is_empty() {
            r.ts = now_iso();
        }
        r.msg = redact::secret_floor_text(&r.msg);
        for (name, f) in r.fields.iter_mut() {
            if name.to_lowercase().contains("blinding") {
                *f = Field {
                    ty: "text".into(),
                    value: Value::String("[redacted]".into()),
                };
                continue;
            }
            if let Value::String(s) = &f.value
                && !redact::SECRET_FLOOR_TYPES.contains(&f.ty.as_str())
            {
                f.value = Value::String(redact::secret_floor_text(s));
            }
        }
        self.bytes += r.bytes();
        self.records.push_back(r);
        while self.records.len() > MAX_RECORDS {
            if let Some(old) = self.records.pop_front() {
                self.bytes -= old.bytes();
            }
        }
        while !self.records.is_empty() && self.bytes > MAX_BYTES {
            let drop = (self.records.len() as f64 * 0.1).ceil().max(1.0) as usize;
            for _ in 0..drop {
                if let Some(old) = self.records.pop_front() {
                    self.bytes -= old.bytes();
                }
            }
        }
        self.bytes = self.bytes.max(2);
        self.generation += 1;
    }

    fn local(
        &mut self,
        level: &str,
        module: &str,
        file: &str,
        msg: &str,
        fields: Vec<(&str, Field)>,
    ) {
        self.emit(Record {
            id: String::new(),
            ts: String::new(),
            level: level.into(),
            module: module.into(),
            file: file.into(),
            line: 0,
            msg: msg.into(),
            fields: fields.into_iter().map(|(k, v)| (k.to_owned(), v)).collect(),
        });
    }

    /// Folds the app's finished requests (newest last) into transport records.
    pub fn fold_timings(&mut self, timings: &VecDeque<(String, Duration)>) {
        let start = match &self.last_timing {
            None => 0,
            Some(last) => match timings.iter().rposition(|t| t == last) {
                Some(i) => i + 1,
                None => 0,
            },
        };
        if start >= timings.len() {
            return;
        }
        let new: Vec<(String, Duration)> = timings.iter().skip(start).cloned().collect();
        self.last_timing = timings.back().cloned();
        for (kind, elapsed) in new {
            // The bridge's own poll is not logged (it would feed back into the ring).
            if kind == "ui.logs.snapshot" {
                continue;
            }
            self.local(
                "info",
                TRANSPORT,
                "daemon/transport.ts",
                "Daemon invoke finished",
                vec![
                    ("kind", text(&kind)),
                    ("request_kind", text(&kind)),
                    (
                        "duration_ms",
                        Field {
                            ty: "duration_ms".into(),
                            value: json!(elapsed.as_millis() as u64),
                        },
                    ),
                ],
            );
        }
    }

    /// "Daemon invoke started" for requests that appeared in `App::pending` since the last
    /// call (best effort: a request sent and answered between two polls only shows finished).
    pub fn fold_pending<'a>(&mut self, pending: impl Iterator<Item = (&'a String, &'a str)>) {
        let mut live = std::collections::HashSet::new();
        let mut new: Vec<(String, String)> = Vec::new();
        for (id, kind) in pending {
            live.insert(id.clone());
            if kind != "ui.logs.snapshot" && !self.started.contains(id) {
                new.push((id.clone(), kind.to_owned()));
            }
        }
        new.sort();
        for (id, kind) in new {
            self.started.insert(id.clone());
            self.local(
                "debug",
                TRANSPORT,
                "daemon/transport.ts",
                "Daemon invoke started",
                vec![
                    ("kind", text(&kind)),
                    ("request_id", text(&id)),
                    ("trace_id", text(&id)),
                ],
            );
        }
        self.started.retain(|id| live.contains(id));
    }

    pub fn poll_args(&self) -> Value {
        json!({"after_id": self.cursor, "limit": PAGE_LIMIT})
    }

    /// One `ui.logs.snapshot` reply; true when another page should be read right away.
    pub fn on_snapshot(&mut self, data: Option<&Value>, error: Option<&str>, clock: f64) -> bool {
        self.in_flight = false;
        let Some(d) = data.filter(|_| error.is_none()) else {
            if !self.failing {
                self.failing = true;
                self.local(
                    "warning",
                    "daemon:bridge-poll",
                    "lib/daemonLogBridge.ts",
                    "Daemon log poll failed; backing off",
                    vec![(
                        "error_message",
                        text(error.unwrap_or("ui.logs.snapshot returned no data")),
                    )],
                );
            }
            self.next_poll = clock + BRIDGE_BACKOFF;
            self.pages_this_tick = 0;
            return false;
        };
        self.failing = false;
        let started = d["started_at"].as_str().unwrap_or("").to_owned();
        if self.started_at.as_deref() != Some(started.as_str()) {
            let first = self.started_at.is_none();
            self.started_at = Some(started.clone());
            if !first {
                // Daemon restart: ring ids start over.
                self.cursor = 0;
                return true;
            }
        }
        if d["gap"].as_bool() == Some(true) {
            self.local(
                "info",
                "daemon:bridge",
                "lib/daemonLogBridge.ts",
                "Daemon log records were evicted before the bridge caught up",
                vec![
                    (
                        "after_id",
                        Field {
                            ty: "number".into(),
                            value: json!(self.cursor),
                        },
                    ),
                    (
                        "last_id",
                        Field {
                            ty: "number".into(),
                            value: d["last_id"].clone(),
                        },
                    ),
                ],
            );
        }
        let recs = d["records"].as_array().cloned().unwrap_or_default();
        for r in &recs {
            let fields = r["fields"]
                .as_object()
                .map(|m| {
                    m.iter()
                        .map(|(k, v)| {
                            (
                                k.clone(),
                                Field {
                                    ty: v["type"].as_str().unwrap_or("text").to_owned(),
                                    value: v["value"].clone(),
                                },
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            self.emit(Record {
                id: format!("daemon-{started}-{}", r["id"]),
                ts: r["ts"].as_str().unwrap_or("").to_owned(),
                level: r["level"].as_str().unwrap_or("info").to_owned(),
                module: r["module"].as_str().unwrap_or("").to_owned(),
                file: r["file"].as_str().unwrap_or("").to_owned(),
                line: r["line"].as_i64().unwrap_or(0),
                msg: r["msg"].as_str().unwrap_or("").to_owned(),
                fields,
            });
        }
        let last_id = d["last_id"].as_u64().unwrap_or(self.cursor);
        self.cursor = if recs.len() == PAGE_LIMIT {
            recs.last()
                .and_then(|r| r["id"].as_u64())
                .unwrap_or(last_id)
        } else {
            last_id
        };
        self.pages_this_tick += 1;
        if recs.len() == PAGE_LIMIT && self.pages_this_tick < 4 {
            return true;
        }
        self.pages_this_tick = 0;
        self.next_poll = clock + BRIDGE_INTERVAL;
        false
    }

    pub fn level_counts(&self) -> [usize; 5] {
        let mut c = [0; 5];
        for r in &self.records {
            if let Some(i) = LEVELS.iter().position(|l| *l == r.level) {
                c[i] += 1;
            }
        }
        c
    }

    pub fn module_counts(&self) -> Vec<(String, usize)> {
        let mut v: Vec<(String, usize)> = Vec::new();
        for r in &self.records {
            match v.iter_mut().find(|(m, _)| *m == r.module) {
                Some((_, n)) => *n += 1,
                None => v.push((r.module.clone(), 1)),
            }
        }
        // `localeCompare`: case-insensitive first.
        v.sort_by(|a, b| {
            a.0.to_lowercase()
                .cmp(&b.0.to_lowercase())
                .then(a.0.cmp(&b.0))
        });
        v
    }
}

pub fn text(s: &str) -> Field {
    Field {
        ty: "text".into(),
        value: Value::String(s.to_owned()),
    }
}

/// The table filter (`filterRecords`): indices into the ring.
pub fn filter(
    store: &Store,
    query: &str,
    regex: bool,
    level: Option<&str>,
    module: Option<&str>,
    o: &Opts,
) -> Vec<usize> {
    let needle = query.trim();
    enum M {
        None,
        Sub(String),
        Re(Regex),
        Never,
    }
    let m = if needle.is_empty() {
        M::None
    } else if !regex {
        M::Sub(needle.to_lowercase())
    } else {
        match Regex::new(needle, true) {
            Ok(r) => M::Re(r),
            Err(_) => M::Never,
        }
    };
    store
        .records
        .iter()
        .enumerate()
        .filter(|(_, r)| {
            if level.is_some_and(|l| r.level != l) || module.is_some_and(|m| r.module != m) {
                return false;
            }
            match &m {
                M::None => true,
                M::Never => false,
                M::Sub(s) => format_record(r, o).to_lowercase().contains(s),
                M::Re(re) => re.is_match(&format_record(r, o)),
            }
        })
        .map(|(i, _)| i)
        .collect()
}

pub struct Header {
    pub app_version: String,
    pub os: String,
    pub generated_at: String,
    pub time_range: String,
    pub active_filter: String,
    pub redaction: String,
}

/// `exportLogRecords`.
pub fn export(records: &[&Record], format: &str, o: &Opts, h: &Header) -> String {
    let rendered: Vec<Record> = records.iter().map(|r| r.redacted(o)).collect();
    let watermark = if o.redacted {
        ""
    } else {
        "RAW EXPORT - may contain wallet material"
    };
    if format == "jsonl" {
        let mut lines: Vec<String> = Vec::new();
        if !watermark.is_empty() {
            lines.push(ordjson::compact(&obj(vec![
                ("kind", v("kassiber.log_export_watermark")),
                ("ts", v(h.generated_at.clone())),
                ("msg", v(watermark)),
            ])));
        }
        lines.extend(rendered.iter().map(|r| ordjson::compact(&r.to_j())));
        return format!("{}\n", lines.join("\n"));
    }
    let lines: Vec<String> = records.iter().map(|r| format_record(r, o)).collect();
    if format == "log" {
        return if watermark.is_empty() {
            format!("{}\n", lines.join("\n"))
        } else {
            format!("{watermark}\n{}\n", lines.join("\n"))
        };
    }
    let mut out: Vec<String> = vec!["# Kassiber log snapshot".into(), String::new()];
    if !watermark.is_empty() {
        out.push(format!("> {watermark}"));
    }
    out.push(format!("- App version: {}", h.app_version));
    out.push(format!("- OS: {}", h.os));
    out.push(format!("- Generated: {}", h.generated_at));
    out.push(format!("- Time range: {}", h.time_range));
    out.push(format!(
        "- Active filter: {}",
        if o.redacted {
            redact::text_for_mode(&h.active_filter, o)
        } else {
            h.active_filter.clone()
        }
    ));
    out.push(format!("- Redaction: {}", h.redaction));
    out.push(String::new());
    out.push("```log".into());
    out.extend(lines);
    out.push("```".into());
    out.push(String::new());
    out.join("\n")
}

fn is_failure(r: &Record) -> bool {
    r.level == "error" || r.fields.iter().any(|(n, _)| n == "error_code") || {
        let m = r.msg.to_lowercase();
        m.contains("failed") || m.contains("threw") || m.contains("error")
    }
}

fn is_ai(r: &Record) -> bool {
    r.field_str("kind").starts_with("ai.chat")
        || r.module.contains("ai")
        || r.msg.contains("AI chat")
}

/// `exportSupportBundleRecords`.
pub fn support_bundle(records: &[&Record], issue: &str, mode: Mode, h: &Header) -> String {
    let max_events = 1000;
    let radius = 12usize;
    let exported: Vec<&Record> = records
        .iter()
        .skip(records.len().saturating_sub(max_events))
        .copied()
        .collect();
    let o = Opts {
        redacted: true,
        mask_amounts: mode == Mode::PublicSafe,
        mode,
    };
    let events: Vec<Record> = exported.iter().map(|r| r.redacted(&o)).collect();
    let failures: Vec<usize> = events
        .iter()
        .enumerate()
        .filter(|(_, r)| is_failure(r))
        .map(|(i, _)| i)
        .collect();
    let mut ctx = std::collections::BTreeSet::new();
    for &fi in &failures {
        let trace = events[fi].field_str("trace_id");
        let req = events[fi].field_str("request_id");
        for (i, r) in events.iter().enumerate() {
            if (!trace.is_empty() && r.field_str("trace_id") == trace)
                || (!req.is_empty() && r.field_str("request_id") == req)
            {
                ctx.insert(i);
            }
        }
        for i in fi.saturating_sub(radius)..=(fi + radius).min(events.len().saturating_sub(1)) {
            ctx.insert(i);
        }
    }
    let ai: Vec<usize> = events
        .iter()
        .enumerate()
        .filter(|(_, r)| is_ai(r))
        .map(|(i, _)| i)
        .collect();
    let omitted = records.len() - exported.len();
    let note = if mode == Mode::PublicSafe {
        "Each following JSONL row is independently redacted for public support: wallet/credential material is stripped, txids are replaced with stable pseudonyms (txid#…), amounts are replaced with salted runtime pseudonyms (amount#…), and operational fields such as addresses, paths, URLs, emails, IPs, labels, and onion hosts are masked."
    } else {
        "High-signal support bundles keep operational debugging data readable, including addresses, paths, URLs, labels, emails, IPs, onion hosts, and error text. Txids are replaced with stable pseudonyms (txid#…), and amounts are replaced with salted runtime pseudonyms (amount#… with a coarse ~magnitude) — never the raw value — so cross-line correlation survives without exposing exact amounts. Wallet/credential material is stripped. Intended for the maintainer or a trusted debugging session, not public posting."
    };
    let sections = obj(vec![
        ("issue", v(1)),
        ("redaction_report", v(1)),
        ("events", v(events.len())),
        ("last_failures", v(ctx.len())),
        ("ai_provenance", v(ai.len())),
        ("diagnostics", v(1)),
    ]);
    let mut lines: Vec<J> = vec![
        obj(vec![
            ("kind", v("kassiber.support_bundle.manifest")),
            ("schema_version", v(1)),
            ("generated_at", v(h.generated_at.clone())),
            ("app_version", v(h.app_version.clone())),
            ("os", v(h.os.clone())),
            ("time_range", v(h.time_range.clone())),
            (
                "active_filter",
                v(redact::text_for_mode(&h.active_filter, &o)),
            ),
            ("redaction", v(mode.id())),
            ("public_safe", v(mode == Mode::PublicSafe)),
            ("format_note", v(note)),
            ("sections", sections),
        ]),
        obj(vec![
            ("kind", v("kassiber.support_bundle.issue")),
            ("schema_version", v(1)),
            ("description", v(redact::text_for_mode(issue.trim(), &o))),
        ]),
        redaction_report(&exported, omitted, mode),
        obj(vec![
            ("kind", v("kassiber.support_bundle.diagnostics")),
            ("schema_version", v(1)),
            (
                "summary",
                obj(vec![
                    ("events_included", v(events.len())),
                    ("events_omitted_from_start", v(omitted)),
                    ("failures_detected", v(failures.len())),
                    ("ai_provenance_records", v(ai.len())),
                    ("buffer_time_range", v(h.time_range.clone())),
                ]),
            ),
        ]),
    ];
    let row = |kind: &str, i: usize| {
        obj(vec![
            ("kind", v(kind)),
            ("schema_version", v(1)),
            ("index", v(i)),
            ("record", events[i].to_j()),
        ])
    };
    lines.extend((0..events.len()).map(|i| row("kassiber.support_bundle.event", i)));
    lines.extend(
        ctx.iter()
            .map(|&i| row("kassiber.support_bundle.last_failure", i)),
    );
    lines.extend(
        ai.iter()
            .map(|&i| row("kassiber.support_bundle.ai_provenance", i)),
    );
    format!(
        "{}\n",
        lines
            .iter()
            .map(ordjson::compact)
            .collect::<Vec<_>>()
            .join("\n")
    )
}

fn redaction_report(records: &[&Record], omitted: usize, mode: Mode) -> J {
    let mut secret_counts: Vec<(String, J)> = Vec::new();
    let mut op_counts: Vec<(String, J)> = Vec::new();
    let (mut floor_hits, mut ps_hits) = (0, 0);
    let ps = Opts {
        redacted: true,
        mask_amounts: false,
        mode: Mode::PublicSafe,
    };
    let mut tally = |v: &str| {
        if redact::secret_floor_text(v) != v {
            floor_hits += 1;
        }
        if mode == Mode::PublicSafe && redact::text_for_mode(v, &ps) != v {
            ps_hits += 1;
        }
    };
    for r in records {
        tally(&r.msg);
        for (_, f) in &r.fields {
            let bump = |m: &mut Vec<(String, J)>| match m.iter_mut().find(|(k, _)| *k == f.ty) {
                Some((_, J::V(n))) => *n = Value::from(n.as_u64().unwrap_or(0) + 1),
                _ => m.push((f.ty.clone(), v(1))),
            };
            if redact::SECRET_FLOOR_TYPES.contains(&f.ty.as_str()) {
                bump(&mut secret_counts);
            }
            if redact::OPERATIONAL_TYPES.contains(&f.ty.as_str()) || f.ty == "amount" {
                bump(&mut op_counts);
            }
            tally(&redact::stringify(&f.value));
        }
    }
    let excluded = [
        "raw daemon arguments",
        "raw imported rows",
        "raw AI prompts",
        "database files",
        "descriptors",
        "xpubs",
        "private keys",
        "mnemonics",
        "backend URLs",
        "API keys",
        "local filesystem paths",
        "stack locals",
    ];
    obj(vec![
        ("kind", v("kassiber.support_bundle.redaction_report")),
        ("schema_version", v(1)),
        ("mode", v(mode.id())),
        ("txids", v("pseudonymized")),
        (
            "amounts",
            v(if mode == Mode::PublicSafe {
                "pseudonymized"
            } else {
                "pseudonymized-with-magnitude"
            }),
        ),
        ("omitted_events_from_start", v(omitted)),
        ("secret_floor_field_counts", J::Obj(secret_counts)),
        ("operational_field_counts", J::Obj(op_counts)),
        ("secret_floor_text_hits", v(floor_hits)),
        ("public_safe_text_hits", v(ps_hits)),
        (
            "excluded_material",
            J::Arr(excluded.iter().map(|x| v(*x)).collect()),
        ),
    ])
}

/// `logFilename` / `supportBundleFilename`.
pub fn filename(format: &str, redaction: &str) -> String {
    format!("kassiber-{}Z-{redaction}.{format}", stamp())
}

pub fn support_filename() -> String {
    format!("kassiber-support-{}Z.support.jsonl", stamp())
}

fn stamp() -> String {
    now_iso()[..16].replace(':', "-")
}

pub fn time_range(records: &[&Record]) -> String {
    match (records.first(), records.last()) {
        (Some(a), Some(b)) => format!("{} to {}", a.ts, b.ts),
        _ => "empty".into(),
    }
}

pub fn filter_label(level: Option<&str>, module: Option<&str>, query: &str, regex: bool) -> String {
    [
        level
            .map(|l| format!("level={l}"))
            .unwrap_or_else(|| "level=all".into()),
        module
            .map(|m| format!("module={m}"))
            .unwrap_or_else(|| "module=all".into()),
        if query.is_empty() {
            "search=none".into()
        } else {
            format!("search={}{query}", if regex { "regex:" } else { "" })
        },
    ]
    .join(", ")
}

/// `formatBytes`.
pub fn format_bytes(b: usize) -> String {
    if b < 1024 {
        format!("{b} B")
    } else if b < 1024 * 1024 {
        format!("{:.1} KiB", b as f64 / 1024.0)
    } else {
        format!("{:.1} MiB", b as f64 / (1024.0 * 1024.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(level: &str, module: &str, msg: &str) -> Record {
        Record {
            id: String::new(),
            ts: "2026-11-15T10:00:00.000Z".into(),
            level: level.into(),
            module: module.into(),
            file: "f.rs".into(),
            line: 3,
            msg: msg.into(),
            fields: vec![("kind".into(), text("ui.x"))],
        }
    }

    #[test]
    fn ring_filter_and_export() {
        let mut s = Store::new();
        s.emit(rec("info", "daemon", "request started"));
        s.emit(rec("error", "kassiber.daemon.request", "request failed"));
        assert_eq!(s.level_counts(), [1, 0, 1, 0, 0]);
        let o = Opts::screen(true, false);
        assert_eq!(filter(&s, "", false, Some("error"), None, &o), vec![1]);
        assert_eq!(filter(&s, "STARTED", false, None, None, &o), vec![0]);
        assert_eq!(filter(&s, "fail(ed|s)", true, None, None, &o), vec![1]);
        assert_eq!(filter(&s, "(", true, None, None, &o), Vec::<usize>::new());
        assert_eq!(
            format_record(&s.records[0], &o),
            "2026-11-15T10:00:00.000Z INFO    daemon:f.rs:3 request started kind=ui.x"
        );
        let h = Header {
            app_version: "v".into(),
            os: "macOS".into(),
            generated_at: "g".into(),
            time_range: "r".into(),
            active_filter: "level=all".into(),
            redaction: "redacted".into(),
        };
        let all: Vec<&Record> = s.records.iter().collect();
        let md = export(&all, "md", &o, &h);
        assert!(md.starts_with("# Kassiber log snapshot\n\n- App version: v\n"));
        let raw = export(&all, "log", &Opts::screen(false, false), &h);
        assert!(raw.starts_with("RAW EXPORT"));
        let b = support_bundle(&all, "it broke", Mode::HighSignal, &h);
        assert_eq!(b.lines().count(), 4 + 2 + 2);
        assert!(b.contains("\"last_failures\":2"));
        s.clear();
        assert!(s.records.is_empty());
        assert_eq!(format_bytes(2048), "2.0 KiB");
    }
}
