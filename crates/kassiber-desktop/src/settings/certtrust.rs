//! Confirmed trust for a saved backend's TLS certificate (lib/certificateTrust.ts,
//! components/kb/settings/TrustCertificateDialog.tsx and CertificateTrustOffer.tsx;
//! docs/reference/backends.md#self-signed-certificates).
//!
//! The daemon fetches and pins; this module decides where the flow is offered, maps the
//! daemon's codes and fixed texts to the two problems the UI explains, and draws the dialog.
//! Nothing contacts the server until the user presses Fetch certificate, and nothing is trusted
//! until they compare the fingerprint and press Trust this server: the trust request carries the
//! exact fingerprint and `revision` of the certificate the daemon returned and the user saw, and
//! the daemon fetches again and refuses anything else. Showing the offer, opening the dialog,
//! Settings and connection tests never fetch a certificate.
//!
//! Two hosts use it: the backend page in Settings (`backend::CERT_HOST`) and the wallet page's
//! sync error (`wallets::detail::CERT_HOST`). Each keeps its own [`Trust`] state and reply tags.

use rune::{Color, El, Shadow, col, row, text};
use serde_json::{Value, json};

use crate::app::{App, Reply};
use crate::format::{Day, parse_ts};
use crate::i18n::{Lang, lang, t, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::txdetail::widgets::{Size, Variant, button, button_surface, icon_button};
use crate::ui::ic;

// ---- lib/certificateTrust.ts ----------------------------------------------------------------

/// Backend kinds whose `certificate` setting pins the server certificate.
pub const TRUST_KINDS: &[&str] = &[
    "bitcoinrpc",
    "electrum",
    "esplora",
    "liquid-esplora",
    "mempool",
];

/// `canTrustCertificate`: a saved backend of a supported kind over TLS.
pub fn can_trust(kind: Option<&str>, url: Option<&str>) -> bool {
    let Some(kind) = kind.filter(|k| TRUST_KINDS.contains(k)) else {
        return false;
    };
    let _ = kind;
    let scheme = url.unwrap_or("").trim().to_lowercase();
    scheme.starts_with("ssl://") || scheme.starts_with("https://")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Problem {
    Untrusted,
    Changed,
}

/// `certificateProblemFromCode`.
pub fn problem_from_code(code: Option<&str>) -> Option<Problem> {
    match code {
        Some("tls_certificate_untrusted") => Some(Problem::Untrusted),
        Some("tls_certificate_changed") => Some(Problem::Changed),
        _ => None,
    }
}

/// `certificateProblemFromText`: connection tests report failures as log lines carrying the
/// daemon's fixed English messages for these two errors (never server text).
pub fn problem_from_text(text: Option<&str>) -> Option<Problem> {
    let s = text?.to_lowercase();
    if s.contains("this server's certificate changed") {
        Some(Problem::Changed)
    } else if s.contains("tls certificate is not signed by an authority kassiber trusts") {
        Some(Problem::Untrusted)
    } else {
        None
    }
}

/// `CERTIFICATE_ERROR_KEYS`: refusals and failures the dialog explains in its own words (the
/// translation test walks them; the view maps each refusal itself).
#[cfg(test)]
const ERROR_KEYS: &[&str] = &[
    "alreadyTrusted",
    "unpinnable",
    "plaintext",
    "unsupported",
    "mismatch",
    "changedSinceReview",
    "offline",
    "connection",
    "writeFailed",
    "overridden",
    "unknown",
];

/// `certificateErrorKey`.
pub fn error_key(code: Option<&str>) -> &'static str {
    match code.unwrap_or("") {
        "backend_certificate_already_trusted" => "alreadyTrusted",
        "backend_certificate_unpinnable" => "unpinnable",
        "backend_certificate_plaintext" => "plaintext",
        "backend_certificate_unsupported" => "unsupported",
        "backend_certificate_mismatch" => "mismatch",
        "backend_certificate_config_changed" => "changedSinceReview",
        "network_egress_disabled" => "offline",
        "network_error"
        | "backend_timeout"
        | "network_proxy_required"
        | "network_proxy_unsupported"
        | "protocol_error" => "connection",
        "backend_certificate_write_failed" => "writeFailed",
        "backend_certificate_overridden" => "overridden",
        _ => "unknown",
    }
}

/// `ui.backends.certificate.fetch` data (and, with `certificate`, `.trust` data).
#[derive(Clone, Debug, PartialEq)]
pub struct Presented {
    pub backend: String,
    pub host: String,
    pub port: u64,
    pub fingerprint: String,
    pub subject: Option<String>,
    pub issuer: Option<String>,
    pub not_before: Option<String>,
    pub not_after: Option<String>,
    pub self_signed: bool,
    pub replaces: Option<String>,
    pub clears_insecure: bool,
    /// The backend setup this certificate was fetched for; trust sends it back.
    pub revision: String,
    /// The pin file the backend's `certificate` setting now names (trust only).
    pub certificate: Option<String>,
}

fn opt_str(v: &Value, k: &str) -> Option<String> {
    v.get(k)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

impl Presented {
    /// A fetch reply. Fails closed: without a fingerprint and the `revision` Trust sends back,
    /// there is nothing the user could confirm.
    pub fn parse(v: &Value) -> Option<Presented> {
        let p = Presented::parse_trusted(v)?;
        (!p.revision.is_empty()).then_some(p)
    }

    /// A trust reply: the pinned certificate (the daemon does not echo the revision).
    pub fn parse_trusted(v: &Value) -> Option<Presented> {
        let fingerprint = opt_str(v, "fingerprint_sha256")?;
        let revision = opt_str(v, "revision").unwrap_or_default();
        Some(Presented {
            backend: opt_str(v, "backend").unwrap_or_default(),
            host: opt_str(v, "host").unwrap_or_default(),
            port: v.get("port").and_then(Value::as_u64).unwrap_or(0),
            fingerprint,
            subject: opt_str(v, "subject"),
            issuer: opt_str(v, "issuer"),
            not_before: opt_str(v, "not_before"),
            not_after: opt_str(v, "not_after"),
            self_signed: v
                .get("self_signed")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            replaces: opt_str(v, "replaces_fingerprint_sha256"),
            clears_insecure: v
                .get("clears_insecure")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            revision,
            certificate: opt_str(v, "certificate"),
        })
    }
}

// ---- The dialog's state ---------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Intro,
    Fetching,
    Review,
    Trusting,
    Done,
    Error,
}

#[derive(Default)]
pub struct Trust {
    pub open: bool,
    /// The saved backend's name, as the daemon knows it.
    pub backend: String,
    /// What to call it on screen.
    pub label: String,
    pub problem: Option<Problem>,
    pub phase: Phase,
    /// The certificate the daemon returned (shown, and bound into Trust).
    pub presented: Option<Presented>,
    pub error: Option<&'static str>,
    /// The fingerprint copy button shows its check until then.
    pub copied_until: f64,
}

impl Trust {
    pub fn busy(&self) -> bool {
        matches!(self.phase, Phase::Fetching | Phase::Trusting)
    }
}

/// Where the dialog lives: its state, its reply tags, and what a confirmed trust changes.
pub struct Host {
    pub key: &'static str,
    pub get: fn(&App) -> &Trust,
    pub get_mut: fn(&mut App) -> &mut Trust,
    pub fetch_tag: &'static str,
    pub trust_tag: &'static str,
    /// Focus returns here when the dialog closes.
    pub trigger: &'static str,
    pub on_trusted: fn(&mut App, &Presented),
}

/// Opens the dialog on its intro (contacts nothing).
pub fn open(app: &mut App, host: &Host, backend: &str, label: &str, problem: Option<Problem>) {
    *(host.get_mut)(app) = Trust {
        open: true,
        backend: backend.to_owned(),
        label: label.to_owned(),
        problem,
        ..Trust::default()
    };
}

/// Cancel, ×, Escape, the backdrop: closes and forgets, except while a request runs.
pub fn close(app: &mut App, host: &Host) {
    let st = (host.get_mut)(app);
    if st.busy() {
        return;
    }
    *st = Trust::default();
}

/// Fetch certificate / Retry: the user's press is the only thing that reaches the server.
pub fn fetch(app: &mut App, host: &Host) {
    let st = (host.get_mut)(app);
    if !st.open || st.busy() || st.backend.is_empty() {
        return;
    }
    st.phase = Phase::Fetching;
    st.presented = None;
    st.error = None;
    let args = json!({"name": st.backend});
    if app
        .send("ui.backends.certificate.fetch", args, host.fetch_tag)
        .is_none()
    {
        let st = (host.get_mut)(app);
        st.phase = Phase::Error;
        st.error = Some("unknown");
    }
}

/// Trust this server: sends exactly the fingerprint and revision the user reviewed.
pub fn trust(app: &mut App, host: &Host) {
    let st = (host.get_mut)(app);
    if st.phase != Phase::Review {
        return;
    }
    let Some(p) = st.presented.as_ref() else {
        return;
    };
    let args = json!({"name": st.backend, "fingerprint": p.fingerprint, "revision": p.revision});
    st.phase = Phase::Trusting;
    if app
        .send("ui.backends.certificate.trust", args, host.trust_tag)
        .is_none()
    {
        let st = (host.get_mut)(app);
        st.phase = Phase::Error;
        st.error = Some("unknown");
    }
}

/// The host's `on_reply` hands its two tags here. A reply that no longer matches what the
/// dialog waits for (closed, reopened for another backend) is dropped.
pub fn on_reply(app: &mut App, host: &Host, tag: &str, r: &Reply) -> bool {
    let fetching = tag == host.fetch_tag;
    if !fetching && tag != host.trust_tag {
        return false;
    }
    let st = (host.get_mut)(app);
    let waiting = if fetching {
        Phase::Fetching
    } else {
        Phase::Trusting
    };
    if !st.open || st.phase != waiting {
        return true;
    }
    let fail = |st: &mut Trust, key: &'static str| {
        st.phase = Phase::Error;
        st.error = Some(key);
        st.presented = None;
    };
    if r.error.is_some() {
        let code = r.error_detail.get("code").and_then(Value::as_str);
        fail(st, error_key(code));
        return true;
    }
    let parsed = if fetching {
        Presented::parse(&r.data)
    } else {
        Presented::parse_trusted(&r.data)
    };
    let Some(p) = parsed else {
        fail(st, "unknown");
        return true;
    };
    // Bound to the backend the dialog asked about. A trust the daemon confirms shows what it
    // pinned (it refuses any fingerprint but the one sent with `backend_certificate_mismatch`).
    if (!p.backend.is_empty() && p.backend != st.backend)
        || (!fetching && r.data.get("trusted").and_then(Value::as_bool) != Some(true))
    {
        fail(st, "unknown");
        return true;
    }
    if fetching {
        st.presented = Some(p);
        st.phase = Phase::Review;
    } else {
        st.presented = Some(p.clone());
        st.phase = Phase::Done;
        (host.on_trusted)(app, &p);
    }
    true
}

// ---- Dates (formatDateTime: dateStyle medium, timeStyle short, local time) -------------------

#[repr(C)]
struct Tm {
    tm_sec: i32,
    tm_min: i32,
    tm_hour: i32,
    tm_mday: i32,
    tm_mon: i32,
    tm_year: i32,
    tm_wday: i32,
    tm_yday: i32,
    tm_isdst: i32,
    tm_gmtoff: std::ffi::c_long,
    tm_zone: *const std::ffi::c_char,
}

unsafe extern "C" {
    fn localtime_r(t: *const i64, out: *mut Tm) -> *mut Tm;
}

fn local_offset(secs: f64) -> i64 {
    let t = secs as i64;
    let mut tm = Tm {
        tm_sec: 0,
        tm_min: 0,
        tm_hour: 0,
        tm_mday: 0,
        tm_mon: 0,
        tm_year: 0,
        tm_wday: 0,
        tm_yday: 0,
        tm_isdst: 0,
        tm_gmtoff: 0,
        tm_zone: std::ptr::null(),
    };
    // SAFETY: `localtime_r` writes only into `tm`, a properly laid-out `struct tm`.
    let ok = unsafe { !localtime_r(&t, &mut tm).is_null() };
    if ok { tm.tm_gmtoff as i64 } else { 0 }
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// `formatDateTime(value, language) ?? value`: `Jan 1, 2026, 1:00 AM` / `01.01.2026, 01:00`.
pub fn date_time(value: &str, lang: Lang, offset: impl Fn(f64) -> i64) -> String {
    let Some(secs) = parse_ts(value) else {
        return value.to_owned();
    };
    let local = (secs as i64) + offset(secs);
    let day = Day::from_epoch_days(local.div_euclid(86_400));
    let r = local.rem_euclid(86_400);
    let (h, m) = (r / 3600, r / 60 % 60);
    match lang {
        Lang::En => {
            let ap = if h < 12 { "AM" } else { "PM" };
            let h12 = if h % 12 == 0 { 12 } else { h % 12 };
            format!(
                "{} {}, {}, {}:{:02}\u{202f}{ap}",
                MONTHS[(day.m - 1) as usize],
                day.d,
                day.y,
                h12,
                m
            )
        }
        Lang::De => format!("{:02}.{:02}.{}, {:02}:{:02}", day.d, day.m, day.y, h, m),
    }
}

// ---- View -----------------------------------------------------------------------------------

fn hex(s: &str) -> Color {
    rune::color::hex(s).unwrap()
}

/// `text-red-700 dark:text-red-300`.
fn red_ink(tk: &Tk) -> Color {
    if tk.dark {
        hex("#fca5a5")
    } else {
        hex("#b91c1c")
    }
}

/// `rounded-md border px-3 py-2` notice box in a tone (`border-<c>-500/30 bg-<c>-500/10`).
fn tone_box(tk: &Tk, key: &str, base: Color, ink: Color, msg: String, alert: bool) -> El {
    let b = col()
        .key(key.to_owned())
        .w_full()
        .px(tk.r(0.75))
        .py(tk.r(0.5))
        .radius(tk.r(0.375))
        .border(1.0, base.alpha(0.3))
        .border_layout()
        .fill(base.alpha(0.1))
        .child(text(msg).font(tk.tsm(400.0)).color(ink).wrap().w_full());
    if alert { b.role("alert") } else { b }
}

fn muted(tk: &Tk, s: String) -> El {
    text(s)
        .font(tk.tsm(400.0))
        .color(tk.muted_fg)
        .wrap()
        .w_full()
}

/// `Field`: `grid sm:grid-cols-[9rem_1fr] sm:gap-3`, the label muted, the value mono and
/// broken anywhere for fingerprints, with an optional action beside it.
fn field(tk: &Tk, key: &str, label: String, value: String, mono: bool, action: Option<El>) -> El {
    let style = if mono {
        tk.tsm(400.0).mono()
    } else {
        tk.tsm(400.0)
    };
    let mut v = text(value).font(style).color(tk.fg);
    v = if mono { v.wrap_anywhere() } else { v.wrap() };
    let dd = row()
        .grow(1.0)
        .w(0.0)
        .min_w(0.0)
        .gap(tk.r(0.5))
        .items_start()
        .child(v.grow(1.0).min_w(0.0))
        .children(action);
    row()
        .key(key.to_owned())
        .w_full()
        .gap(tk.r(0.75))
        .items_start()
        .child(
            text(label)
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .w(tk.r(9.0))
                .wrap(),
        )
        .child(dd)
}

/// `CopyButton` (outline `icon-xs`, Copy → Check for 1.1 s, honouring the clipboard policy).
fn copy_button(app: &App, tk: &Tk, host: &'static Host, value: String) -> El {
    let copied = (host.get)(app).copied_until > app.clock;
    let aria = if copied {
        t("chrome:copyButton.copied")
    } else {
        t("settings:certificateTrust.copyFingerprint")
    };
    let glyph = if copied {
        ic(
            &I::CHECK,
            tk.r(0.75),
            if tk.dark {
                hex("#34d399")
            } else {
                hex("#059669")
            },
        )
    } else {
        ic(&I::COPY, tk.r(0.75), tk.fg)
    };
    button_surface(
        tk,
        row()
            .key(format!("{}-copy", host.key))
            .size(tk.r(1.5), tk.r(1.5))
            .center(),
        Variant::Outline,
    )
    .role("button")
    .label(aria)
    .focusable(true)
    .on_press_tx(move |a: &mut App, tx: &mut rune::Tx| {
        super::effects::copy(a, tx, value.clone());
        let until = a.clock + 1.1;
        (host.get_mut)(a).copied_until = until;
        tx.wake_after(std::time::Duration::from_millis(1150));
    })
    .child(glyph.a11y_hidden())
}

/// `TrustCertificateView`: header, notices, the reviewed certificate, the footer actions.
fn content(app: &App, tk: &Tk, host: &'static Host) -> El {
    let st = (host.get)(app);
    let phase = st.phase;
    let busy = st.busy();
    let presented = st.presented.as_ref();
    // A fetched certificate that would replace a pin is a changed certificate, however the
    // dialog was opened.
    let changed = phase != Phase::Done
        && (st.problem == Some(Problem::Changed)
            || presented.is_some_and(|p| p.replaces.is_some()));
    let k = host.key;
    let mut body = col().w_full().gap(tk.r(0.75));
    if changed {
        body = body.child(tone_box(
            tk,
            &format!("{k}-changed"),
            hex("#ef4444"),
            red_ink(tk),
            t("settings:certificateTrust.changedWarning"),
            true,
        ));
    } else if st.problem == Some(Problem::Untrusted) {
        body = body.child(muted(tk, t("settings:certificateTrust.untrustedNotice")));
    }
    if matches!(phase, Phase::Intro | Phase::Fetching) {
        body = body.child(muted(tk, t("settings:certificateTrust.introBody")));
    }
    if let Some(p) =
        presented.filter(|_| matches!(phase, Phase::Review | Phase::Trusting | Phase::Done))
    {
        let done = phase == Phase::Done;
        let mut dl = col()
            .key(format!("{k}-fields"))
            .w_full()
            .gap(tk.r(0.5))
            .px(tk.r(0.75))
            .py(tk.r(0.75))
            .radius(tk.r(0.375))
            .border(1.0, tk.border)
            .border_layout();
        dl = dl.child(field(
            tk,
            &format!("{k}-server"),
            t("settings:certificateTrust.server"),
            format!("{}:{}", p.host, p.port),
            true,
            None,
        ));
        dl = dl.child(field(
            tk,
            &format!("{k}-fingerprint"),
            t("settings:certificateTrust.fingerprint"),
            p.fingerprint.clone(),
            true,
            Some(copy_button(app, tk, host, p.fingerprint.clone())),
        ));
        if let Some(prev) = p.replaces.as_ref().filter(|_| !done) {
            dl = dl.child(field(
                tk,
                &format!("{k}-previous"),
                t("settings:certificateTrust.previousFingerprint"),
                prev.clone(),
                true,
                None,
            ));
        }
        if let Some(s) = &p.subject {
            dl = dl.child(field(
                tk,
                &format!("{k}-subject"),
                t("settings:certificateTrust.subject"),
                s.clone(),
                false,
                None,
            ));
        }
        let issuer = if p.self_signed {
            t("settings:certificateTrust.selfSigned")
        } else {
            p.issuer
                .clone()
                .unwrap_or_else(|| t("settings:certificateTrust.unknown"))
        };
        dl = dl.child(field(
            tk,
            &format!("{k}-issuer"),
            t("settings:certificateTrust.issuer"),
            issuer,
            false,
            None,
        ));
        if let (Some(from), Some(to)) = (&p.not_before, &p.not_after) {
            let l = lang();
            let range = tr(
                "settings:certificateTrust.validityRange",
                &[
                    ("from", &date_time(from, l, local_offset)),
                    ("to", &date_time(to, l, local_offset)),
                ],
            );
            dl = dl.child(field(
                tk,
                &format!("{k}-validity"),
                t("settings:certificateTrust.validity"),
                range,
                false,
                None,
            ));
        }
        body = body.child(dl);
        if !done {
            body = body.child(
                text(t("settings:certificateTrust.compare"))
                    .font(tk.tsm(500.0))
                    .color(tk.fg)
                    .wrap()
                    .w_full(),
            );
        }
        if p.replaces.is_some() && !done {
            let ink = if tk.dark {
                hex("#fde68a")
            } else {
                hex("#92400e")
            };
            body = body.child(tone_box(
                tk,
                &format!("{k}-replaces"),
                hex("#f59e0b"),
                ink,
                t("settings:certificateTrust.replaces"),
                false,
            ));
        }
        if p.clears_insecure && !done {
            body = body.child(muted(tk, t("settings:certificateTrust.clearsInsecure")));
        }
        if done {
            let ink = if tk.dark {
                hex("#6ee7b7")
            } else {
                hex("#047857")
            };
            body = body.child(
                text(tr(
                    "settings:certificateTrust.trusted",
                    &[("host", &p.host)],
                ))
                .font(tk.tsm(500.0))
                .color(ink)
                .wrap()
                .w_full()
                .role("status"),
            );
        }
    }
    if phase == Phase::Error
        && let Some(e) = st.error
    {
        body = body.child(
            text(t(&format!("settings:certificateTrust.errors.{e}")))
                .font(tk.tsm(400.0))
                .color(red_ink(tk))
                .wrap()
                .w_full()
                .role("alert"),
        );
    }
    let mut actions: Vec<El> = Vec::new();
    if phase == Phase::Done {
        actions.push(
            button(
                tk,
                format!("{k}-done"),
                Variant::Default,
                Size::Default,
                None,
                &t("common:actions.done"),
                true,
            )
            .autofocus()
            .on_press(move |a: &mut App| close(a, host)),
        );
    } else {
        let mut cancel = button(
            tk,
            format!("{k}-cancel"),
            Variant::Outline,
            Size::Default,
            None,
            &t("common:actions.cancel"),
            !busy,
        );
        if !busy {
            cancel = cancel.on_press(move |a: &mut App| close(a, host));
        }
        actions.push(cancel);
        let (label, review) = match phase {
            Phase::Review | Phase::Trusting => (t("settings:certificateTrust.trust"), true),
            Phase::Error => (t("common:actions.retry"), false),
            _ => (t("settings:certificateTrust.fetch"), false),
        };
        let key = if review {
            format!("{k}-trust")
        } else {
            format!("{k}-fetch")
        };
        let mut primary = button(
            tk,
            key,
            Variant::Default,
            Size::Default,
            None,
            &label,
            !busy,
        );
        if !busy {
            primary = if review {
                primary.on_press(move |a: &mut App| trust(a, host))
            } else {
                primary.on_press(move |a: &mut App| fetch(a, host))
            };
        }
        actions.push(primary);
    }
    let header = col()
        .w_full()
        .gap(tk.r(0.5))
        .pr(tk.r(1.5))
        .child(
            text(t("settings:certificateTrust.title"))
                .font(tk.tlg(600.0).lh(tk.r(1.125)))
                .color(tk.fg)
                .role("heading")
                .level(2),
        )
        .child(
            text(tr(
                "settings:certificateTrust.description",
                &[("backend", &st.label)],
            ))
            .font(tk.tsm(400.0))
            .color(tk.muted_fg)
            .wrap()
            .w_full(),
        );
    col()
        .w_full()
        .gap(tk.r(1.0))
        .child(header)
        .child(body)
        .child(
            row()
                .w_full()
                .gap(tk.r(0.5))
                .justify_end()
                .children(actions),
        )
}

/// The dialog (`DialogContent className="sm:max-w-[560px]"`) over whatever opened it.
pub fn mount(app: &App, tk: &Tk, layer: &mut Vec<El>, host: &'static Host) {
    let st = (host.get)(app);
    if !st.open {
        return;
    }
    let key = host.key;
    let wash = if tk.dark {
        Color::BLACK.alpha(0.22)
    } else {
        tk.fg.alpha(0.06)
    };
    let closer = move |a: &mut App| close(a, host);
    layer.push(
        crate::shell::window_backdrop(format!("{key}-scrim"), wash)
            .on_press(closer)
            .role("none"),
    );
    let win_w = if app.shell.window_w > 0.0 {
        app.shell.window_w
    } else {
        1440.0
    };
    let win_h = if app.shell.window_h > 0.0 {
        app.shell.window_h
    } else {
        900.0
    };
    let top = crate::shell::dialog_top(tk, win_h);
    let width = (win_w - tk.r(2.0)).min(560.0);
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
    let busy = st.busy();
    let mut close_btn = icon_button(
        tk,
        format!("{key}-close"),
        Variant::Ghost,
        tk.r(1.75),
        &I::X,
        tk.r(1.0),
        t("common:actions.close"),
    )
    .pin(None, Some(tk.r(0.75)), Some(tk.r(0.75)), None);
    close_btn = if busy {
        close_btn.disabled(true).opacity(0.5)
    } else {
        close_btn.on_press(closer)
    };
    let max_h = win_h - top - tk.r(1.0);
    let body = col()
        .key(format!("{key}-scroll"))
        .w_full()
        .max_h(max_h - 2.0)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .child(
            col()
                .w_full()
                .padding(tk.r(1.25))
                .child(content(app, tk, host)),
        );
    let mut surface = col()
        .key(key.to_owned())
        .role("dialog")
        .label(t("settings:certificateTrust.title"))
        .description(tr(
            "settings:certificateTrust.description",
            &[("backend", &st.label)],
        ))
        .w(width)
        .radius(tk.r(1.0))
        .fill(tk.background.alpha(0.8))
        .backdrop_blur(6.0)
        .transition(crate::shell::dialog_enter())
        .border(1.0, border)
        .border_layout()
        .clip_rounded(tk.r(1.0))
        .modal(true, closer)
        .focus_return(host.trigger.to_owned())
        .child(body)
        .child(close_btn);
    for sh in shadows {
        surface = surface.box_shadow(sh);
    }
    layer.push(crate::shell::centered(surface, win_w, width, top));
}

/// `CertificateTrustOffer`: "Check certificate…" beside an error an untrusted or changed
/// certificate caused, for a backend kind that can pin one. Rendering contacts nothing.
pub fn offer(
    tk: &Tk,
    host: &'static Host,
    problem: Option<Problem>,
    backend: Option<(&str, &str)>,
) -> Option<El> {
    let problem = problem?;
    let (name, kind) = backend?;
    if name.is_empty() || !TRUST_KINDS.contains(&kind) {
        return None;
    }
    let name = name.to_owned();
    Some(
        row().mt(tk.r(0.5)).child(
            button(
                tk,
                host.trigger,
                Variant::Outline,
                Size::Sm,
                None,
                &t("settings:certificateTrust.checkButton"),
                true,
            )
            .on_press(move |a: &mut App| open(a, host, &name, &name, Some(problem))),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trust_is_offered_for_tls_pinnable_kinds_only() {
        assert!(can_trust(Some("electrum"), Some("ssl://node.local:50002")));
        assert!(can_trust(Some("esplora"), Some(" HTTPS://node.local/api")));
        assert!(can_trust(
            Some("bitcoinrpc"),
            Some("https://127.0.0.1:8332")
        ));
        assert!(!can_trust(Some("electrum"), Some("tcp://node.local:50001")));
        assert!(!can_trust(Some("esplora"), Some("http://node.local/api")));
        assert!(!can_trust(Some("btcpay"), Some("https://pay.example")));
        assert!(!can_trust(Some("lnd"), Some("https://127.0.0.1:8080")));
        assert!(!can_trust(None, Some("https://x")));
    }

    #[test]
    fn problems_follow_the_daemon_codes_and_fixed_texts() {
        assert_eq!(
            problem_from_code(Some("tls_certificate_untrusted")),
            Some(Problem::Untrusted)
        );
        assert_eq!(
            problem_from_code(Some("tls_certificate_changed")),
            Some(Problem::Changed)
        );
        assert_eq!(problem_from_code(Some("backend_tls_config_invalid")), None);
        assert_eq!(
            problem_from_text(Some(
                "Connection failed: The server's TLS certificate is not signed by an authority Kassiber trusts"
            )),
            Some(Problem::Untrusted)
        );
        assert_eq!(
            problem_from_text(Some(
                "Connection failed: This server's certificate changed: it no longer matches"
            )),
            Some(Problem::Changed)
        );
        assert_eq!(
            problem_from_text(Some("Connection failed: connection refused")),
            None
        );
        assert_eq!(problem_from_text(None), None);
    }

    #[test]
    fn every_error_code_has_an_english_and_german_message() {
        assert_eq!(error_key(Some("backend_certificate_mismatch")), "mismatch");
        assert_eq!(
            error_key(Some("backend_certificate_config_changed")),
            "changedSinceReview"
        );
        assert_eq!(error_key(Some("protocol_error")), "connection");
        assert_eq!(error_key(Some("anything")), "unknown");
        for l in [Lang::En, Lang::De] {
            crate::i18n::set_lang(l);
            for k in ERROR_KEYS {
                let key = format!("settings:certificateTrust.errors.{k}");
                assert_ne!(t(&key), key, "{l:?} {k}");
                assert!(!t(&key).is_empty());
            }
        }
        crate::i18n::set_lang(Lang::En);
    }

    #[test]
    fn a_reply_without_fingerprint_or_revision_is_not_trustable() {
        let full = json!({"backend": "home", "host": "node.local", "port": 50002, "fingerprint_sha256": "81:4C", "revision": "5f", "self_signed": true, "replaces_fingerprint_sha256": null});
        let p = Presented::parse(&full).unwrap();
        assert_eq!(p.fingerprint, "81:4C");
        assert_eq!(p.replaces, None);
        assert!(Presented::parse(&json!({"fingerprint_sha256": "81:4C"})).is_none());
        assert!(Presented::parse(&json!({"revision": "5f"})).is_none());
        // The daemon's trust reply carries the pin but no revision.
        let trusted = json!({"backend": "home", "fingerprint_sha256": "81:4C", "certificate": "/d/certificates/814c.pem", "trusted": true});
        assert!(Presented::parse(&trusted).is_none());
        assert_eq!(
            Presented::parse_trusted(&trusted)
                .unwrap()
                .certificate
                .as_deref(),
            Some("/d/certificates/814c.pem")
        );
    }

    #[test]
    fn validity_dates_follow_the_interface_language() {
        let utc = |_: f64| 0;
        assert_eq!(
            date_time("2026-01-01T00:00:00Z", Lang::De, utc),
            "01.01.2026, 00:00"
        );
        assert_eq!(
            date_time("2036-01-01T13:05:00Z", Lang::En, utc),
            "Jan 1, 2036, 1:05\u{202f}PM"
        );
        assert_eq!(date_time("not a date", Lang::En, utc), "not a date");
    }
}
