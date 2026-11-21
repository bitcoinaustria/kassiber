//! Pieces "Link manually" and a move's details share: one transaction read as a leg
//! (`useMoveLeg.ts`), the review core's dry run and its apply (`reviewPreview.ts`), the leg card
//! (`MoveLeg.tsx`), the fee line, labelled sections and what a previewed change does
//! (`MoveEffects.tsx`), and the panel/dialog frames both use.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};

use rune::{Color, Cx, El, Shadow, Tx, col, el, row, text};
use serde_json::{Value, json};

use super::super::kit::{self, Var};
use super::super::link_model::{
    self, FeeCheck, LinkLeg, Parts, bps_text, check_fee, date_time, fee_share_bps, short_id,
};
use super::super::model::{self, Rail};
use crate::app::{App, Currency, Reply};
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::{MASK, Tk};
use crate::ui::{ic, sensitive};

/// Who a read or a run is for: its replies come back under that part's tags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    Link,
    Unlink,
}

impl Owner {
    fn tag(self, stage: &str) -> &'static str {
        match (self, stage) {
            (Owner::Link, "leg") => "custody.link.leg",
            (Owner::Link, "snap") => "custody.link.pv.snap",
            (Owner::Link, "cases") => "custody.link.pv.cases",
            (Owner::Link, "plan") => "custody.link.pv.plan",
            (Owner::Link, _) => "custody.link.pv.apply",
            (Owner::Unlink, "leg") => "custody.unlink.leg",
            (Owner::Unlink, "snap") => "custody.unlink.pv.snap",
            (Owner::Unlink, "cases") => "custody.unlink.pv.cases",
            (Owner::Unlink, "plan") => "custody.unlink.pv.plan",
            (Owner::Unlink, _) => "custody.unlink.pv.apply",
        }
    }
}

// ---- Legs (`useMoveLeg`) ----------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct LegRead {
    pub loading: bool,
    pub failed: bool,
    pub tx: Option<Value>,
}

/// Transactions read as legs, by id (`ui.transactions.resolve`, a local read).
#[derive(Clone, Debug, Default)]
pub struct Legs {
    pub map: HashMap<String, LegRead>,
}

impl Legs {
    /// The leg (with what its links leave open when `parts` is given), loading, failed.
    pub fn leg(&self, id: Option<&str>, parts: Option<&Parts>) -> (Option<LinkLeg>, bool, bool) {
        let Some(id) = id else {
            return (None, false, false);
        };
        match self.map.get(id) {
            None => (None, true, false),
            Some(r) => {
                let leg =
                    r.tx.as_ref()
                        .and_then(|tx| link_model::leg_from_tx(tx, None))
                        .map(|l| link_model::with_open_part(l, parts));
                (leg, r.loading, r.failed)
            }
        }
    }
}

fn legs_mut(app: &mut App, owner: Owner) -> &mut Legs {
    match owner {
        Owner::Link => &mut app.custody.link.legs,
        Owner::Unlink => &mut app.custody.unlink.legs,
    }
}

/// Reads a transaction once (again after `reload`).
pub fn ensure_leg(app: &mut App, owner: Owner, id: &str) {
    if id.is_empty() || legs_mut(app, owner).map.contains_key(id) {
        return;
    }
    legs_mut(app, owner).map.insert(
        id.to_owned(),
        LegRead {
            loading: true,
            ..LegRead::default()
        },
    );
    app.send(
        "ui.transactions.resolve",
        json!({"query": id}),
        owner.tag("leg"),
    );
}

/// A leg read came back.
pub fn on_leg(app: &mut App, owner: Owner, r: &Reply) {
    let id = r.data["query"].as_str().map(str::to_owned);
    let legs = legs_mut(app, owner);
    // A failed read has no query to name: mark whichever read is still out.
    let key = id.or_else(|| {
        legs.map
            .iter()
            .find(|(_, v)| v.loading)
            .map(|(k, _)| k.clone())
    });
    let Some(key) = key else { return };
    let tx = r.data.get("transaction").filter(|v| v.is_object()).cloned();
    legs.map.insert(
        key,
        LegRead {
            loading: false,
            failed: r.error.is_some(),
            tx,
        },
    );
}

/// Every leg read again (the book changed).
pub fn reload_legs(app: &mut App, owner: Owner) {
    let ids: Vec<String> = legs_mut(app, owner).map.keys().cloned().collect();
    for id in ids {
        app.send(
            "ui.transactions.resolve",
            json!({"query": id}),
            owner.tag("leg"),
        );
        if let Some(r) = legs_mut(app, owner).map.get_mut(&id) {
            r.loading = false;
        }
    }
}

// ---- The review core's dry run (`useReviewPreview`) -------------------------------------------

#[derive(Clone, Debug, Default)]
pub enum PStatus {
    #[default]
    Idle,
    /// Started at (app clock).
    Checking(f64),
    Ready {
        artifact: Value,
        idem: String,
    },
    Failed {
        message: String,
        code: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Snap,
    Cases,
    Plan,
}

#[derive(Clone, Debug, Default)]
pub struct Preview {
    /// The operations as planned (their JSON), None plans nothing.
    pub key: Option<String>,
    pub status: PStatus,
    settle_at: Option<f64>,
    token: u64,
    /// Requests out, oldest first: (stage, token, cache key).
    queue: VecDeque<(Stage, u64, String)>,
    /// The plan this preview waits on (a run already under way joins it).
    cache_key: Option<String>,
    attempts: u32,
    pub applying: bool,
    /// The artifact of the apply in flight.
    applying_artifact: Option<Value>,
    retries: u64,
}

impl Preview {
    /// A fresh preview for a dialog that closed or moved on. The requests still out stay
    /// listed (their replies settle the shared run table) and the token keeps counting, so a
    /// late reply is never taken for the new preview's.
    pub fn reset(&mut self) {
        let old = std::mem::take(self);
        self.queue = old.queue;
        self.token = old.token;
    }
}

const PREVIEW_CANCELLED: &str = "review_plan_cancelled";
const FINISHED_LIMIT: usize = 20;

thread_local! {
    /// Finished plans by book, input version and operations (`finished`).
    static FINISHED: RefCell<Vec<(String, Value)>> = const { RefCell::new(Vec::new()) };
    /// Plans still running, by the same key, with when they started (`running`).
    static RUNNING: RefCell<HashMap<String, f64>> = RefCell::new(HashMap::new());
}

/// The book state starts over (lock, book switch): plans still running belong to requests that
/// will not be answered here, so a later identical plan must not join them.
pub fn forget_running() {
    RUNNING.with(|r| r.borrow_mut().clear());
}

fn finished(key: &str) -> Option<Value> {
    FINISHED.with(|f| {
        f.borrow()
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    })
}

fn remember(key: &str, artifact: &Value) {
    FINISHED.with(|f| {
        let mut f = f.borrow_mut();
        f.retain(|(k, _)| k != key);
        f.push((key.to_owned(), artifact.clone()));
        while f.len() > FINISHED_LIMIT {
            f.remove(0);
        }
    });
}

fn forget(artifact: &Value) {
    FINISHED.with(|f| f.borrow_mut().retain(|(_, v)| v != artifact));
}

pub fn preview_mut(app: &mut App, owner: Owner) -> &mut Preview {
    match owner {
        Owner::Link => &mut app.custody.link.preview,
        Owner::Unlink => &mut app.custody.unlink.preview,
    }
}

/// The daemon's error code behind a failed request.
pub fn error_code(r: &Reply) -> Option<String> {
    r.error_detail
        .get("code")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// `isStalePlan`.
pub fn is_stale(code: Option<&str>) -> bool {
    matches!(
        code,
        Some("review_plan_stale" | "custody_review_plan_stale")
    )
}

/// Plans `ops` whenever they change (None plans nothing). Called every poll.
pub fn sync_preview(app: &mut App, owner: Owner, ops: Option<Value>) {
    let key = ops.map(|o| o.to_string());
    let clock = app.clock;
    let p = preview_mut(app, owner);
    if p.key == key {
        return;
    }
    p.key = key.clone();
    p.token += 1;
    p.queue.retain(|_| true);
    p.cache_key = None;
    p.attempts = 0;
    match key {
        None => {
            p.status = PStatus::Idle;
            p.settle_at = None;
        }
        Some(_) => {
            p.status = PStatus::Checking(clock);
            // Wait 500 ms after the last change before planning.
            p.settle_at = Some(clock + 0.5);
        }
    }
}

/// `retry`: plan the same operations again.
pub fn retry_preview(app: &mut App, owner: Owner) {
    let clock = app.clock;
    let p = preview_mut(app, owner);
    if p.key.is_none() {
        return;
    }
    p.token += 1;
    p.retries += 1;
    p.cache_key = None;
    p.attempts = 0;
    p.status = PStatus::Checking(clock);
    p.settle_at = Some(clock + 0.5);
}

/// Starts a due plan; keeps the elapsed-seconds line ticking. True when anything changed.
pub fn poll_preview(app: &mut App, owner: Owner, tx: &mut Tx) -> bool {
    let (changed, wake) = advance_preview(app, owner);
    if let Some(w) = wake {
        tx.wake_at(w);
    }
    changed
}

/// `poll_preview` without a frame: (changed, when to wake).
pub fn advance_preview(app: &mut App, owner: Owner) -> (bool, Option<f64>) {
    let clock = app.clock;
    let p = preview_mut(app, owner);
    let mut wake = None;
    if let PStatus::Checking(_) = p.status {
        wake = Some(clock.floor() + 1.0);
    }
    let Some(at) = p.settle_at else {
        return (false, wake);
    };
    if clock < at {
        return (false, Some(wake.map_or(at, |w: f64| w.min(at))));
    }
    p.settle_at = None;
    let token = p.token;
    let scope = app.expected_scope();
    if scope["workspace_id"].is_null() || scope["profile_id"].is_null() {
        return (false, wake);
    }
    // The plan binds to the book's input version, read straight from the journal snapshot.
    preview_mut(app, owner)
        .queue
        .push_back((Stage::Snap, token, String::new()));
    app.send(
        "ui.journals.snapshot",
        json!({"expected_scope": scope}),
        owner.tag("snap"),
    );
    (true, wake)
}

/// Headless runs: start a due plan now instead of after the settle delay.
pub fn start_now(app: &mut App, owner: Owner) {
    let p = preview_mut(app, owner);
    if p.settle_at.is_some() {
        p.settle_at = Some(f64::MIN);
    }
    advance_preview(app, owner);
}

fn send_plan(app: &mut App, owner: Owner, version: i64, token: u64) {
    let scope = app.expected_scope();
    let p = preview_mut(app, owner);
    let Some(key) = p.key.clone() else { return };
    let cache_key = json!([scope["workspace_id"], scope["profile_id"], version, key]).to_string();
    p.cache_key = Some(cache_key.clone());
    if let Some(artifact) = finished(&cache_key) {
        p.status = PStatus::Ready {
            artifact,
            idem: idempotency_key(),
        };
        return;
    }
    if let Some(started) = RUNNING.with(|r| r.borrow().get(&cache_key).copied()) {
        // Joined a run that began earlier: count its time.
        p.status = PStatus::Checking(started);
        return;
    }
    let ops: Value = serde_json::from_str(&key).unwrap_or(Value::Null);
    let clock = app.clock;
    RUNNING.with(|r| r.borrow_mut().insert(cache_key.clone(), clock));
    preview_mut(app, owner)
        .queue
        .push_back((Stage::Plan, token, cache_key));
    app.send("ui.review.plan", json!({"operations": ops, "expected_input_version": version, "expected_scope": scope, "preview": true}), owner.tag("plan"));
}

fn idempotency_key() -> String {
    // A random v4-style id (`crypto.randomUUID()`).
    let mut bytes = [0u8; 16];
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut x = seed ^ 0x9E37_79B9_7F4A_7C15_F39C_C060_5CED_C834;
    for b in bytes.iter_mut() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *b = (x & 0xff) as u8;
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let h: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// `reviewArtifact`: a plan the core returned, checked for the shape the apply needs.
fn valid_artifact(v: &Value) -> bool {
    let effects = |e: &Value| {
        e["entries_count"].is_u64()
            && e["quarantine_count"].is_u64()
            && e["report_ready"].is_boolean()
            && e["quarantines"].is_array()
    };
    v["schema_version"] == 1
        && v["workspace_id"].is_string()
        && v["profile_id"].is_string()
        && v["base_input_version"].is_u64()
        && v["digest"].as_str().is_some_and(|d| {
            d.len() == 64
                && d.chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        })
        && v["operations"]
            .as_array()
            .is_some_and(|a| !a.is_empty() && a.len() <= 50)
        && effects(&v["before"])
        && effects(&v["after"])
}

/// What an apply came back with.
pub enum Applied {
    Ok(Value),
    Err {
        message: String,
        code: Option<String>,
    },
}

/// A preview reply (`stage` is the tag's last segment); an apply's outcome is handed back.
pub fn on_preview_reply(
    app: &mut App,
    owner: Owner,
    stage: &str,
    r: &Reply,
    failed_text: &str,
) -> Option<Applied> {
    if stage == "apply" {
        let p = preview_mut(app, owner);
        p.applying = false;
        let artifact = p.applying_artifact.take();
        return Some(match &r.error {
            None => Applied::Ok(r.data.clone()),
            Some(e) => {
                let code = error_code(r);
                // The book changed in a way its version does not show: forget this plan.
                if is_stale(code.as_deref())
                    && let Some(a) = artifact
                {
                    forget(&a);
                }
                Applied::Err {
                    message: e.clone(),
                    code,
                }
            }
        });
    }
    let p = preview_mut(app, owner);
    let (st, token, cache_key) = p.queue.pop_front()?;
    let current = token == p.token;
    match st {
        Stage::Snap => {
            if !current {
                return None;
            }
            let v = r.data["status"]["journalInputVersion"]
                .as_i64()
                .filter(|_| r.error.is_none());
            match v {
                Some(v) => send_plan(app, owner, v, token),
                None => {
                    let scope = app.expected_scope();
                    preview_mut(app, owner)
                        .queue
                        .push_back((Stage::Cases, token, String::new()));
                    app.send(
                        "ui.review.cases",
                        json!({"limit": 1, "expected_scope": scope}),
                        owner.tag("cases"),
                    );
                }
            }
        }
        Stage::Cases => {
            if !current {
                return None;
            }
            if let Some(e) = &r.error {
                preview_mut(app, owner).status = PStatus::Failed {
                    message: if e.is_empty() {
                        failed_text.to_owned()
                    } else {
                        e.clone()
                    },
                    code: error_code(r),
                };
            } else {
                let v = r.data["input_version"].as_i64().unwrap_or(0);
                send_plan(app, owner, v, token);
            }
        }
        Stage::Plan => {
            RUNNING.with(|run| run.borrow_mut().remove(&cache_key));
            let code = error_code(r);
            if r.error.is_none() && valid_artifact(&r.data) {
                remember(&cache_key, &r.data);
            }
            // Whichever preview waits on this run (it may have joined it) takes its outcome.
            let p = preview_mut(app, owner);
            if p.cache_key.as_deref() != Some(cache_key.as_str())
                || !matches!(p.status, PStatus::Checking(_))
            {
                return None;
            }
            match (&r.error, code.as_deref()) {
                (None, _) if valid_artifact(&r.data) => {
                    p.status = PStatus::Ready {
                        artifact: r.data.clone(),
                        idem: idempotency_key(),
                    }
                }
                (None, _) => {
                    p.status = PStatus::Failed {
                        message: failed_text.to_owned(),
                        code: None,
                    }
                }
                (Some(_), Some(PREVIEW_CANCELLED)) if p.attempts < 2 => {
                    // A newer check stopped this one: run it again.
                    p.attempts += 1;
                    let key = cache_key.clone();
                    let version = serde_json::from_str::<Value>(&key)
                        .ok()
                        .and_then(|v| v[2].as_i64())
                        .unwrap_or(0);
                    p.cache_key = None;
                    send_plan(app, owner, version, token);
                }
                (Some(e), c) => {
                    p.status = PStatus::Failed {
                        message: if c == Some(PREVIEW_CANCELLED) || e.is_empty() {
                            failed_text.to_owned()
                        } else {
                            e.clone()
                        },
                        code: code.clone(),
                    };
                }
            }
        }
    }
    None
}

/// `apply`: sends the planned artifact with its idempotency key. False when nothing is planned.
pub fn apply_preview(app: &mut App, owner: Owner) -> bool {
    let p = preview_mut(app, owner);
    let PStatus::Ready { artifact, idem } = &p.status else {
        return false;
    };
    if p.applying {
        return false;
    }
    let (artifact, idem) = (artifact.clone(), idem.clone());
    p.applying = true;
    p.applying_artifact = Some(artifact.clone());
    let scope =
        json!({"workspace_id": artifact["workspace_id"], "profile_id": artifact["profile_id"]});
    app.send(
        "ui.review.apply",
        json!({"artifact": artifact, "idempotency_key": idem, "expected_scope": scope}),
        owner.tag("apply"),
    );
    true
}

/// `appliedId`.
pub fn applied_id(result: &Value, field: &str) -> Option<String> {
    result["operations"][0]["result"][field]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// What a previewed change does to the two sides of a move (`MoveOutcome`).
#[derive(Clone, Copy, Debug, Default)]
pub struct MoveOutcome {
    pub send_held: bool,
    pub receipt_held: bool,
    pub was_held: bool,
    pub reports_ready_after: bool,
    pub held_before: u64,
    pub held_after: u64,
}

fn held_ids(e: &Value) -> Vec<String> {
    e["quarantines"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|q| {
                    q.get("transaction_id").map(|v| {
                        v.as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| v.to_string())
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn move_outcome(artifact: &Value, out_id: &str, in_id: &str) -> MoveOutcome {
    let before = held_ids(&artifact["before"]);
    let after = held_ids(&artifact["after"]);
    let has = |v: &Vec<String>, id: &str| v.iter().any(|x| x == id);
    MoveOutcome {
        send_held: has(&after, out_id),
        receipt_held: has(&after, in_id),
        was_held: has(&before, out_id) || has(&before, in_id),
        reports_ready_after: artifact["after"]["report_ready"].as_bool().unwrap_or(false),
        held_before: artifact["before"]["quarantine_count"].as_u64().unwrap_or(0),
        held_after: artifact["after"]["quarantine_count"].as_u64().unwrap_or(0),
    }
}

// ---- Views ------------------------------------------------------------------------------------

/// `DialogSection`: a labelled block (`space-y-1.5`, `text-xs font-medium` title).
pub fn section(tk: &Tk, title: String, content: El) -> El {
    col()
        .w_full()
        .min_w(0.0)
        .gap(tk.r(0.375))
        .child(
            text(title)
                .font(tk.txs(500.0))
                .color(tk.muted_fg)
                .role("heading"),
        )
        .child(content)
}

/// A plain sentence in the dialogs' body text.
pub fn para(tk: &Tk, s: String, color: Color) -> El {
    text(s).font(tk.tsm(400.0)).color(color).wrap()
}

/// A sentence that the privacy modes blur.
pub fn para_s(app: &App, tk: &Tk, s: String, color: Color) -> El {
    sensitive(
        s,
        tk.tsm(400.0).features("'tnum' 1"),
        color,
        app.prefs.hide_sensitive(),
    )
    .wrap()
}

/// `FeeLine`.
pub fn fee_line(app: &App, tk: &Tk, send_msat: Option<i128>, fee: Option<i128>) -> El {
    let Some(fee) = fee else {
        return para(tk, t("review:transfersPage.fee.unknown"), tk.muted_fg);
    };
    let check = match send_msat {
        None => FeeCheck::Plausible,
        Some(s) => check_fee(s, fee),
    };
    let share = send_msat.and_then(|s| fee_share_bps(s, fee));
    let amount = model::msat_sats(fee.abs());
    let share_txt = |exact: bool| match share {
        None => "—".to_owned(),
        Some(0) if !exact => "<0.01".to_owned(),
        Some(b) => bps_text(b),
    };
    let line = match check {
        FeeCheck::Negative => tr("review:transfersPage.fee.more", &[("amount", &amount)]),
        FeeCheck::None => t("review:transfersPage.fee.none"),
        FeeCheck::Mismatch => tr(
            "review:transfersPage.fee.mismatch",
            &[("amount", &amount), ("share", &share_txt(true))],
        ),
        _ => tr(
            "review:transfersPage.fee.amount",
            &[("amount", &amount), ("share", &share_txt(false))],
        ),
    };
    let mut c = col()
        .w_full()
        .gap(tk.r(0.25))
        .child(para_s(app, tk, line, tk.fg));
    if matches!(
        check,
        FeeCheck::Negative | FeeCheck::High | FeeCheck::Mismatch
    ) {
        let key = match check {
            FeeCheck::Negative => "negative",
            FeeCheck::High => "high",
            _ => "mismatch",
        };
        c = c.child(
            row()
                .key("fee-flag")
                .w_full()
                .gap(tk.r(0.375))
                .items_start()
                .child(
                    ic(&I::TRIANGLE_ALERT, tk.r(0.875), kit::amber_text(tk))
                        .mt(tk.r(0.125))
                        .a11y_hidden(),
                )
                .child(
                    text(t(&format!("review:transfersPage.fee.{key}LinkedFlag")))
                        .font(tk.txs(400.0))
                        .color(kit::amber_text(tk))
                        .wrap()
                        .grow(1.0)
                        .min_w(0.0),
                ),
        );
    }
    c
}

/// A line with a leading icon (the effects lists' `li`).
fn icon_line(tk: &Tk, glyph: &'static rune::Icon, color: Color, content: El) -> El {
    row()
        .w_full()
        .gap(tk.r(0.5))
        .items_start()
        .child(ic(glyph, tk.r(0.875), color).mt(tk.r(0.1875)).a11y_hidden())
        .child(content.grow(1.0).min_w(0.0))
}

/// `TaxSentences`: how each tax year moves, from the core's after-minus-before figures.
fn tax_sentences(app: &App, tk: &Tk, artifact: &Value) -> Vec<El> {
    tax_lines(artifact, app.prefs.hide_sensitive())
        .into_iter()
        .map(|s| {
            icon_line(
                tk,
                &I::INFO,
                tk.muted_fg,
                text(s)
                    .font(tk.tsm(400.0).features("'tnum' 1"))
                    .color(tk.fg)
                    .wrap(),
            )
            .key("preview-tax")
        })
        .collect()
}

/// The tax sentences' text. Every year the core lists changed (`delta.years`) gets at least
/// one: its gain and income moves, an Austrian year's Kennzahl moves, and its indicative
/// estimate, which the core sends as null when either side has none to compare (withheld or
/// not applicable): unknown, never zero.
fn tax_lines(artifact: &Value, hidden: bool) -> Vec<String> {
    let tax = &artifact["after"]["tax"];
    let years = artifact["delta"]["years"].as_array();
    let valid = tax["fiat_currency"].is_string()
        && tax["years"].is_array()
        && years.is_some_and(|ys| {
            ys.iter().all(|y| {
                y["year"].is_i64() && y["realized_gain_loss"].is_string() && y["income"].is_string()
            })
        });
    if !valid {
        return vec![];
    }
    let cur = tax["fiat_currency"].as_str().unwrap_or("EUR");
    let money = |v: f64| {
        if hidden {
            MASK.to_owned()
        } else {
            crate::format::fiat(v.abs(), cur)
        }
    };
    let eur = |cents: f64| {
        if hidden {
            MASK.to_owned()
        } else {
            crate::format::fiat(cents.abs() / 100.0, "EUR")
        }
    };
    let decimal = |v: &Value| -> f64 {
        v.as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|v| v.is_finite())
            .unwrap_or(0.0)
    };
    let mut out = vec![];
    for y in years.unwrap() {
        let year = y["year"].to_string();
        let before = out.len();
        let gain = decimal(&y["realized_gain_loss"]);
        let income = decimal(&y["income"]);
        if gain != 0.0 {
            out.push(tr(
                if gain > 0.0 {
                    "review:transfersPage.effects.tax.gainUp"
                } else {
                    "review:transfersPage.effects.tax.gainDown"
                },
                &[("year", &year), ("amount", &money(gain))],
            ));
        }
        if income != 0.0 {
            out.push(tr(
                if income > 0.0 {
                    "review:transfersPage.effects.tax.incomeUp"
                } else {
                    "review:transfersPage.effects.tax.incomeDown"
                },
                &[("year", &year), ("amount", &money(income))],
            ));
        }
        // An Austrian year: each Kennzahl's change in EUR cents.
        if let Some(fields) = y["kennzahlen"].as_object() {
            for (code, change) in fields {
                let cents = change.as_f64().unwrap_or(0.0);
                if cents != 0.0 {
                    out.push(tr(
                        if cents > 0.0 {
                            "review:transfersPage.effects.tax.kennzahlUp"
                        } else {
                            "review:transfersPage.effects.tax.kennzahlDown"
                        },
                        &[("year", &year), ("code", code), ("amount", &eur(cents))],
                    ));
                }
            }
        }
        match y.get("estimated_tax_eur_cents") {
            Some(Value::Null) => out.push(tr(
                if y["indicative_status"] == "withheld" {
                    "review:transfersPage.effects.tax.taxWithheld"
                } else {
                    "review:transfersPage.effects.tax.taxUnavailable"
                },
                &[("year", &year)],
            )),
            Some(v) => {
                let cents = v.as_f64().unwrap_or(0.0);
                if cents != 0.0 {
                    out.push(tr(
                        if cents > 0.0 {
                            "review:transfersPage.effects.tax.taxUp"
                        } else {
                            "review:transfersPage.effects.tax.taxDown"
                        },
                        &[("year", &year), ("amount", &eur(cents))],
                    ));
                }
            }
            None => {}
        }
        // Listed as changed with nothing above to say (a status the copy doesn't name).
        if out.len() == before {
            out.push(tr(
                "review:transfersPage.effects.tax.yearChanged",
                &[("year", &year)],
            ));
        }
    }
    // The plan compares only the latest years and counts the rest per side; a change there
    // isn't in `delta`, so "no changes" would overclaim.
    let omitted = ["before", "after"]
        .iter()
        .filter_map(|side| artifact[side]["tax"]["years_omitted"].as_u64())
        .max()
        .unwrap_or(0);
    if omitted > 0 {
        if out.is_empty() {
            out.push(t("review:transfersPage.effects.tax.noneInCompared"));
        }
        out.push(tr(
            "review:transfersPage.effects.tax.yearsOmitted",
            &[("count", &omitted.to_string())],
        ));
    } else if out.is_empty() {
        out.push(t("review:transfersPage.effects.tax.none"));
    }
    out
}

/// `PreviewOutcomeView`.
#[allow(clippy::too_many_arguments)]
pub fn outcome_view(
    app: &App,
    tk: &Tk,
    key: &str,
    p: &Preview,
    outcome: Option<MoveOutcome>,
    lines: &[String],
    reports_said: bool,
    waiting: Option<String>,
    refusal: Option<String>,
    on_retry: Option<fn(&mut App)>,
) -> El {
    if let Some(w) = waiting {
        return row()
            .key("preview-waiting")
            .gap(tk.r(0.5))
            .center_y()
            .role("status")
            .child(kit::spinner(tk, tk.r(1.0), tk.muted_fg))
            .child(para(tk, w, tk.muted_fg));
    }
    match &p.status {
        PStatus::Idle | PStatus::Checking(_) => {
            let started = if let PStatus::Checking(s) = p.status {
                Some(s)
            } else {
                None
            };
            let secs = started
                .map(|s| (app.clock - s).max(0.0).floor() as usize)
                .unwrap_or(0);
            let mut line = row()
                .gap(tk.r(0.5))
                .center_y()
                .child(kit::spinner(tk, tk.r(1.0), tk.muted_fg))
                .child(
                    text(t("review:transfersPage.effects.checking"))
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg),
                );
            if secs >= 3 {
                line = line.child(
                    text(tc("review:transfersPage.effects.elapsed", secs, &[]))
                        .font(tk.tsm(400.0).features("'tnum' 1"))
                        .color(tk.muted_fg)
                        .a11y_hidden(),
                );
            }
            let mut c = col()
                .key("preview-checking")
                .w_full()
                .gap(tk.r(0.25))
                .role("status")
                .child(line);
            if secs >= 10 {
                c = c.child(
                    text(t("review:transfersPage.effects.checkingLong"))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .wrap()
                        .pl(tk.r(1.5)),
                );
            }
            c
        }
        PStatus::Failed { message, .. } => {
            let mut c = col()
                .key("preview-failed")
                .w_full()
                .gap(tk.r(0.375))
                .role("alert");
            match refusal {
                Some(r) => {
                    c = c.child(para(tk, r, tk.destructive)).child(
                        text(message.clone())
                            .font(tk.txs(400.0))
                            .color(tk.muted_fg)
                            .wrap_anywhere(),
                    );
                }
                None => {
                    c = c.child(
                        text(tr(
                            "review:transfersPage.effects.failed",
                            &[("message", message)],
                        ))
                        .font(tk.tsm(400.0))
                        .color(tk.destructive)
                        .wrap_anywhere(),
                    )
                }
            }
            if let Some(f) = on_retry {
                c = c.child(
                    row().child(
                        kit::btn_xs(
                            tk,
                            format!("{key}-retry"),
                            Var::Outline,
                            Some(&I::ROTATE_CCW),
                            &t("review:transfersPage.effects.retry"),
                            true,
                        )
                        .on_press(f),
                    ),
                );
            }
            c
        }
        PStatus::Ready { artifact, .. } => {
            let mut c = col().key("preview-outcome").w_full().gap(tk.r(0.375));
            for l in lines {
                c = c.child(icon_line(
                    tk,
                    &I::INFO,
                    tk.muted_fg,
                    para(tk, l.clone(), tk.fg),
                ));
            }
            if let Some(o) = outcome
                && !(reports_said && !o.reports_ready_after)
            {
                let (glyph, color, s) = if o.reports_ready_after {
                    (
                        &I::CIRCLE_CHECK,
                        kit::emerald_icon(tk),
                        t("review:transfersPage.effects.reportsReady"),
                    )
                } else {
                    (
                        &I::TRIANGLE_ALERT,
                        kit::amber_icon(tk),
                        tc(
                            "review:transfersPage.effects.reportsBlocked",
                            o.held_after as usize,
                            &[],
                        ),
                    )
                };
                c = c.child(icon_line(tk, glyph, color, para(tk, s, tk.fg)));
            }
            c.children(tax_sentences(app, tk, artifact))
        }
    }
}

/// What the leg card shows before the leg is read.
#[derive(Clone, Debug)]
pub struct Fallback {
    pub wallet: String,
    pub rail: Rail,
    pub amount_msat: Option<i128>,
    pub occurred_at: Option<String>,
}

/// `LegAmount`: exact in its own asset, or its value in fiat when chosen and known.
pub fn leg_amount(
    app: &App,
    tk: &Tk,
    key: &str,
    leg: &LinkLeg,
    style: rune::TextStyle,
    toggle: bool,
) -> El {
    let hidden = app.prefs.hide_sensitive();
    let fiat_mode = app.prefs.currency == Currency::Fiat;
    let (content, name) = match (fiat_mode, leg.fiat) {
        (true, Some(Some(v))) => {
            let s = kit::money(app, v);
            (
                sensitive(s.clone(), style.features("'tnum' 1"), tk.fg, hidden),
                s,
            )
        }
        (true, Some(None)) => {
            let s = t("review:transfersPage.linked.unpriced");
            (sensitive(s.clone(), style, tk.fg, hidden), s)
        }
        _ => (
            kit::asset_figure(
                tk,
                leg.amount_msat,
                &leg.asset,
                style.features("'tnum' 1"),
                tk.fg,
                hidden,
            ),
            model::msat_in(leg.amount_msat, &leg.asset),
        ),
    };
    if toggle {
        kit::currency_toggle(app, tk, key.to_owned(), name, content)
    } else {
        row().child(content)
    }
}

/// `MoveLeg`: one side of a move — wallet with its rail, when, how much, the txid to copy and
/// the way to it on Transactions. `on_open` runs before the transaction opens.
#[allow(clippy::too_many_arguments)]
pub fn move_leg(
    app: &App,
    cx: &Cx<App>,
    tk: &Tk,
    key: &str,
    label: String,
    leg: Option<&LinkLeg>,
    loading: bool,
    fallback: Option<&Fallback>,
    highlight: bool,
    action: Option<El>,
    on_open: fn(&mut App),
) -> El {
    let hidden = app.prefs.hide_sensitive();
    let wallet = leg
        .map(|l| l.wallet.clone())
        .or_else(|| fallback.map(|f| f.wallet.clone()))
        .unwrap_or_else(|| "—".into());
    let rail = leg
        .map(|l| l.rail)
        .or_else(|| fallback.map(|f| f.rail))
        .unwrap_or(Rail::Onchain);
    let occurred = leg
        .and_then(|l| l.occurred_at.clone())
        .or_else(|| fallback.and_then(|f| f.occurred_at.clone()));
    let mut head_right = row().gap(tk.r(0.5)).center_y().child(
        text(date_time(occurred.as_deref()))
            .font(tk.txs(400.0).features("'tnum' 1"))
            .color(tk.muted_fg),
    );
    if let Some(a) = action {
        head_right = head_right.child(a);
    }
    let head = row()
        .w_full()
        .gap(tk.r(0.5))
        .center_y()
        .justify_between()
        .child(
            text(label.to_uppercase())
                .font(tk.txs(500.0).tracking(0.025 * tk.r(0.75)))
                .color(tk.muted_fg),
        )
        .child(head_right);
    let name = row()
        .w_full()
        .min_w(0.0)
        .mt(tk.r(0.375))
        .gap(tk.r(0.375))
        .center_y()
        .child(kit::rail_icon(app, cx, tk, rail))
        .child(kit::wallet_name(tk, &wallet, tk.tsm(500.0), tk.fg, hidden));
    let big = tk.tbase(600.0);
    let amount = match (leg, fallback.and_then(|f| f.amount_msat)) {
        (Some(l), _) => leg_amount(app, tk, &format!("{key}-amount"), l, big, true),
        (None, Some(m)) => kit::asset_figure(tk, m, "BTC", big.features("'tnum' 1"), tk.fg, hidden),
        (None, None) => text("—").font(big).color(tk.fg),
    };
    let mut c = col()
        .key(key.to_owned())
        .w_full()
        .min_w(0.0)
        .px(tk.r(0.75))
        .py(tk.r(0.625))
        .radius(tk.r(0.5))
        .border(1.0, if highlight { tk.primary } else { tk.border })
        .fill(if highlight {
            tk.primary.alpha(0.05)
        } else {
            tk.muted.alpha(0.2)
        })
        .child(head)
        .child(name)
        .child(row().mt(tk.r(0.25)).child(amount));
    if let Some(l) = leg {
        if let (true, Some(o)) = (link_model::leg_has_open_part(l), l.open) {
            c = c.child(
                sensitive(
                    tr(
                        "review:transfersPage.link.partLeft",
                        &[
                            ("left", &model::msat_btc(o.unlinked_msat)),
                            ("total", &model::msat_btc(o.linked_msat + o.unlinked_msat)),
                        ],
                    ),
                    tk.txs(400.0).features("'tnum' 1"),
                    kit::amber_text(tk),
                    hidden,
                )
                .key(format!("{key}-part")),
            );
        }
        if let Some(f) = l.fee_msat.filter(|f| *f != 0) {
            c = c.child(sensitive(
                tr(
                    "review:transfersPage.leg.networkFee",
                    &[("amount", &model::msat_sats(f))],
                ),
                tk.txs(400.0).features("'tnum' 1"),
                tk.muted_fg,
                hidden,
            ));
        }
    }
    let mut foot = row()
        .w_full()
        .min_h(tk.r(1.5))
        .mt(tk.r(0.375))
        .gap(tk.r(0.25))
        .center_y();
    let txid = leg.and_then(|l| l.external_id.clone());
    if loading {
        foot = foot.child(kit::skeleton(tk, tk.r(8.0), tk.r(1.0)));
    } else if let Some(id) = txid {
        let mut short = sensitive(
            short_id(Some(&id), 8, 6),
            tk.txs(400.0).family(rune::Family::Mono),
            tk.muted_fg,
            hidden,
        );
        if !hidden {
            short = short.help(id.clone());
        }
        let payload = id.clone();
        foot = foot.child(short).child(
            kit::icon_btn(
                tk,
                format!("{key}-copy"),
                Var::Ghost,
                tk.r(1.5),
                &I::COPY,
                tk.r(0.875),
                t("review:transfersPage.leg.copyTxid"),
            )
            .on_press_tx(move |a: &mut App, tx: &mut Tx| {
                crate::settings::effects::copy(a, tx, payload.clone())
            }),
        );
    } else {
        foot = foot.child(
            text(t("review:transfersPage.leg.noTxid"))
                .font(tk.txs(400.0))
                .color(tk.muted_fg),
        );
    }
    if let Some(l) = leg {
        let id = l.id.clone();
        foot = foot.child(el().grow(1.0)).child(
            row()
                .key(format!("{key}-open"))
                .gap(tk.r(0.125))
                .center_y()
                .radius(tk.r(0.125))
                .role("link")
                .label(t("review:transfersPage.showInTransactions"))
                .focusable(true)
                .hover_group()
                .on_press(move |a: &mut App| {
                    on_open(a);
                    super::super::show_transaction(a, &id, None);
                })
                .child(
                    text(t("review:transfersPage.showInTransactions"))
                        .font(tk.txs(500.0))
                        .color(tk.fg),
                )
                .child(ic(&I::ARROW_UP_RIGHT, tk.r(0.75), tk.fg)),
        );
    }
    c.child(foot)
}

/// The panel frame (`aside.kb-surface p-4 gap-4`, height-bounded to the window, scrolling).
#[allow(clippy::too_many_arguments)]
pub fn panel_shell(
    app: &App,
    cx: &Cx<App>,
    tk: &Tk,
    key: &str,
    title: String,
    desc: El,
    close_label: String,
    close_enabled: bool,
    close: fn(&mut App),
    content: Vec<El>,
) -> El {
    let win_h = if app.shell.window_h > 0.0 {
        app.shell.window_h
    } else {
        900.0
    };
    let head = row()
        .w_full()
        .gap(tk.r(0.5))
        .items_start()
        .child(
            col()
                .min_w(0.0)
                .grow(1.0)
                .gap(tk.r(0.25))
                .child(
                    text(title)
                        .font(tk.tbase(600.0).lh(tk.r(1.25)))
                        .color(tk.fg)
                        .wrap()
                        .role("heading"),
                )
                .child(desc),
        )
        .child({
            let b = kit::icon_btn(
                tk,
                format!("{key}-close"),
                Var::Ghost,
                tk.r(1.75),
                &I::X,
                tk.r(1.0),
                close_label,
            );
            if close_enabled {
                b.on_press(close)
            } else {
                b.disabled(true).opacity(0.5).focusable(false)
            }
        });
    // `max-h-[var(--kb-panel-room, calc(100vh-6rem))]`: the room under the panel's top edge.
    let top = cx
        .window_layout(key)
        .map(|r| r[1].max(0.0))
        .unwrap_or(tk.r(14.0));
    let room = (win_h - top - 16.0).max(320.0);
    kit::surface(tk)
        .key(key.to_owned())
        .w_full()
        .max_h(room)
        .padding(tk.r(1.0))
        .gap(tk.r(1.0))
        .role("complementary")
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .child(head)
        .children(content)
}

/// `DialogContent` (`max-h-[90vh] overflow-y-auto`) at `max_w` rem, scrolling inside.
#[allow(clippy::too_many_arguments)]
pub fn dialog_shell(
    app: &App,
    tk: &Tk,
    layer: &mut Vec<El>,
    key: &str,
    max_w: f32,
    title: String,
    desc: El,
    close: fn(&mut App),
    content: Vec<El>,
) {
    let wash = if tk.dark {
        Color::BLACK.alpha(0.22)
    } else {
        tk.fg.alpha(0.06)
    };
    layer.push(
        crate::shell::window_backdrop(format!("{key}-scrim"), wash)
            .on_press(close)
            .role("none"),
    );
    let win_h = if app.shell.window_h > 0.0 {
        app.shell.window_h
    } else {
        900.0
    };
    let win_w = if app.shell.window_w > 0.0 {
        app.shell.window_w
    } else {
        1440.0
    };
    let max_h = win_h * 0.9;
    let top = crate::shell::dialog_top(tk, win_h);
    let (border, shadows) = if tk.dark {
        (
            Color::WHITE.alpha(0.08),
            vec![
                Shadow {
                    y: 1.0,
                    color: Color::WHITE.alpha(0.04),
                    inset: true,
                    ..Default::default()
                },
                Shadow {
                    y: 24.0,
                    blur: 72.0,
                    spread: -20.0,
                    color: Color::BLACK.alpha(0.9),
                    ..Default::default()
                },
            ],
        )
    } else {
        (
            tk.fg.alpha(0.1),
            vec![Shadow {
                y: 24.0,
                blur: 64.0,
                spread: -24.0,
                color: Color::BLACK.alpha(0.65),
                ..Default::default()
            }],
        )
    };
    let header = col()
        .w_full()
        .gap(tk.r(0.5))
        .pr(tk.r(1.5))
        .child(
            text(title)
                .font(tk.tlg(600.0).lh(tk.r(1.125)))
                .color(tk.fg)
                .wrap()
                .role("heading"),
        )
        .child(desc);
    let close_btn = kit::icon_btn(
        tk,
        format!("{key}-close"),
        Var::Ghost,
        tk.r(1.75),
        &I::X,
        tk.r(1.0),
        t("common:actions.close"),
    )
    .pin(None, Some(tk.r(0.75)), Some(tk.r(0.75)), None)
    .on_press(close);
    let body = col()
        .w_full()
        .gap(tk.r(1.0))
        .child(header)
        .children(content);
    let mut surface = col()
        .key(key.to_owned())
        .role("dialog")
        .w(tk.r(max_w).min(win_w - tk.r(2.0)))
        .max_h(max_h.min(win_h - top - tk.r(1.0)))
        .radius(tk.r(1.0))
        .fill(tk.background.alpha(0.8))
        .backdrop_blur(6.0)
        .transition(crate::shell::dialog_enter())
        .border(1.0, border)
        .modal(true, close)
        .child(
            col()
                .w_full()
                .padding(tk.r(1.5))
                .scroll(rune::ScrollSpec {
                    stick_bottom: false,
                })
                .child(body),
        )
        .child(close_btn);
    for sh in shadows {
        surface = surface.box_shadow(sh);
    }
    // Its own layer root, so its entrance and exit carry the blur (shell::centered).
    layer.push(crate::shell::centered(
        surface,
        win_w,
        tk.r(max_w).min(win_w - tk.r(2.0)),
        top,
    ));
}

/// The amber list of everything to look at again (`Warnings`).
pub fn warnings(app: &App, tk: &Tk, lines: &[String]) -> El {
    let fg = kit::tw(if tk.dark { "amber-200" } else { "amber-900" });
    let mut c = col()
        .key("link-warnings")
        .w_full()
        .gap(tk.r(0.25))
        .px(tk.r(0.75))
        .py(tk.r(0.5))
        .radius(tk.r(0.375))
        .border(1.0, kit::tw("amber-500").alpha(0.4))
        .fill(kit::tw("amber-500").alpha(0.1));
    for l in lines {
        c = c.child(
            row()
                .w_full()
                .gap(tk.r(0.5))
                .items_start()
                .child(
                    ic(&I::TRIANGLE_ALERT, tk.r(0.875), fg)
                        .mt(tk.r(0.125))
                        .a11y_hidden(),
                )
                .child(
                    sensitive(l.clone(), tk.tsm(400.0), fg, app.prefs.hide_sensitive())
                        .wrap()
                        .grow(1.0)
                        .min_w(0.0),
                ),
        );
    }
    c
}

/// A destructive button (`variant="destructive"`), with the spinner while busy.
pub fn destructive_btn(
    tk: &Tk,
    key: impl Into<String>,
    label: &str,
    enabled: bool,
    busy: bool,
) -> El {
    crate::ui::button::button_busy(
        tk,
        key,
        Var::Destructive,
        crate::ui::button::Size::Default,
        busy,
        None,
        label,
        enabled,
    )
}

/// `Button` in the default size, the spinner leading while busy.
pub fn md_btn(
    tk: &Tk,
    key: impl Into<String>,
    variant: Var,
    glyph: Option<&'static rune::Icon>,
    label: &str,
    enabled: bool,
    busy: bool,
) -> El {
    crate::ui::button::button_busy(
        tk,
        key,
        variant,
        crate::ui::button::Size::Default,
        busy,
        glyph,
        label,
        enabled,
    )
}

#[cfg(test)]
mod preview_tests {
    use super::*;

    /// Link closed while its plan runs: the late reply still ends the shared run, so reopening
    /// the same operation sends a new plan instead of joining an abandoned one forever.
    #[test]
    fn a_plan_reply_after_close_ends_its_run() {
        let mut app = crate::app::tests::quiet_app();
        let key = "[\"w\",\"p\",7,\"ops\"]".to_owned();
        RUNNING.with(|r| r.borrow_mut().insert(key.clone(), 0.0));
        let p = preview_mut(&mut app, Owner::Link);
        p.token = 3;
        p.cache_key = Some(key.clone());
        p.status = PStatus::Checking(0.0);
        p.queue.push_back((Stage::Plan, 3, key.clone()));
        crate::custody::link::close(&mut app);
        let p = preview_mut(&mut app, Owner::Link);
        assert_eq!(p.token, 3, "the token keeps counting");
        assert!(matches!(p.status, PStatus::Idle));
        let reply = Reply {
            kind: "ui.review.plan".into(),
            data: Value::Null,
            error: Some("cancelled".into()),
            error_detail: Value::Null,
            elapsed: Default::default(),
        };
        on_preview_reply(&mut app, Owner::Link, "plan", &reply, "failed");
        assert!(RUNNING.with(|r| !r.borrow().contains_key(&key)));
        assert!(preview_mut(&mut app, Owner::Link).queue.is_empty());
    }
}

#[cfg(test)]
mod reset_tests {
    use super::*;

    /// A lock or book switch orphans a running plan: the same plan asked again later must not
    /// join it (and wait forever).
    #[test]
    fn a_book_reset_forgets_running_plans() {
        let mut app = crate::app::tests::quiet_app();
        RUNNING.with(|r| r.borrow_mut().insert("[\"w\",\"p\",7,\"ops\"]".into(), 0.0));
        app.reset_book_state();
        assert!(RUNNING.with(|r| r.borrow().is_empty()));
    }
}

#[cfg(test)]
mod tax_tests {
    use super::*;
    use serde_json::json;

    /// A year the core lists only for a Kennzahl move and an estimate that became withheld
    /// (sent as null) is a change: never "No tax year changes", never a zero estimate.
    #[test]
    fn a_kennzahl_move_and_a_withheld_estimate_are_tax_changes() {
        let artifact = json!({
            "after": {"tax": {"fiat_currency": "EUR", "years": []}},
            "delta": {"years": [{
                "year": 2024, "realized_gain_loss": "0.00", "income": "0.00",
                "kennzahlen": {"172": -12_345, "174": 12_345},
                "estimated_tax_eur_cents": null, "indicative_status": "withheld"
            }, {
                "year": 2025, "realized_gain_loss": "0.00", "income": "0.00",
                "kennzahlen": {}, "estimated_tax_eur_cents": null, "indicative_status": "indicative"
            }]}
        });
        let lines = tax_lines(&artifact, false);
        assert!(
            !lines.iter().any(|l| l == "No tax year changes."),
            "{lines:?}"
        );
        assert_eq!(lines.len(), 4, "{lines:?}");
        assert!(lines[0].starts_with("2024: KZ 172"), "{lines:?}");
        assert!(lines[2].contains("withheld"), "{lines:?}");
        assert!(lines[3].starts_with("2025:"), "{lines:?}");
        // A year with an estimate delta still reads as before.
        let mut up = artifact.clone();
        up["delta"]["years"] = json!([{
            "year": 2024, "realized_gain_loss": "100.00", "income": "0.00",
            "estimated_tax_eur_cents": 2750
        }]);
        assert_eq!(tax_lines(&up, false).len(), 2);
    }

    /// Pairing a 2015 sale in a book with ten later income years removes a 2015 gain the plan
    /// never compared: 2015 is counted in `before.tax.years_omitted` and `delta` is empty.
    #[test]
    fn omitted_years_are_never_no_changes() {
        let artifact = json!({
            "before": {"tax": {"fiat_currency": "EUR", "years": [], "years_omitted": 1}},
            "after": {"tax": {"fiat_currency": "EUR", "years": [], "years_omitted": 0}},
            "delta": {"years": []}
        });
        assert_eq!(
            tax_lines(&artifact, false),
            vec![
                "No changes in the tax years compared.",
                "1 older tax year was not compared and may change.",
            ]
        );
        // Listed changes keep their lines; the larger side's count follows them.
        let mut changed = artifact.clone();
        changed["after"]["tax"]["years_omitted"] = json!(3);
        changed["delta"]["years"] = json!([{
            "year": 2024, "realized_gain_loss": "-300.00", "income": "0.00"
        }]);
        let lines = tax_lines(&changed, false);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines[0].starts_with("2024:"), "{lines:?}");
        assert_eq!(
            lines[1],
            "3 older tax years were not compared and may change."
        );
        // Nothing omitted: the plain sentence stands.
        let mut whole = artifact;
        whole["before"]["tax"]["years_omitted"] = json!(0);
        assert_eq!(tax_lines(&whole, false), vec!["No tax year changes."]);
    }
}
