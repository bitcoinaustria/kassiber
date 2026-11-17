//! The notification center (NotificationCenter.tsx): live conditions rebuilt from local reads
//! (`ui.review.badges`, `ui.freshness.status`), running jobs, and a RAM-only recent list.

use rune::{Color, El, col, el, row, text};
use rune_kit::Placement;

use super::Popup;
use super::titlebar::bar_button;
use crate::app::{App, Privacy, ToastTone};
use crate::i18n::{t, tc};
use crate::icons as I;
use crate::theme::{Tk, Tone};
use crate::ui::ic;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Act {
    ProcessJournals,
    Review,
    Sources,
    Retry,
    RetryJournals,
    /// Settings → Security at the Touch ID row (the saved-unlock conditions).
    Security,
}

struct Condition {
    blocking: bool,
    error: bool,
    title: String,
    body: String,
    /// What went wrong and where, one line per cause (`projectDetails`).
    details: Vec<String>,
    action: Option<(String, Act)>,
}

// ---- notificationCauses.ts ------------------------------------------------------------------

/// `NOTIFICATION_CAUSES` (keys under `chrome:notifications.cause.*`).
#[cfg(test)]
const CAUSES: &[&str] = &[
    "offline",
    "rateLimited",
    "unavailable",
    "certificate",
    "certificateChanged",
    "credentials",
    "noBackend",
    "connection",
    "cancelled",
];

/// `CODE_CAUSES`: codes that say what happened on their own.
fn code_cause(code: &str) -> Option<&'static str> {
    Some(match code {
        "network_egress_disabled" => "offline",
        "network_error" | "backend_timeout" => "connection",
        "backend_tls_config_invalid" => "certificate",
        // Distinct from a generic TLS failure: the trust flow can resolve these.
        "tls_certificate_untrusted" => "certificate",
        "tls_certificate_changed" => "certificateChanged",
        "backend_not_configured" => "noBackend",
        "cancelled" => "cancelled",
        _ => return None,
    })
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `needle` with a word boundary on both ends (`\bneedle\b`), in lower-case `hay`.
fn has_word(hay: &str, needle: &str) -> bool {
    hay.match_indices(needle).any(|(i, _)| {
        let before = hay[..i].chars().next_back().is_none_or(|c| !is_word(c));
        let after = hay[i + needle.len()..]
            .chars()
            .next()
            .is_none_or(|c| !is_word(c));
        before && after
    })
}

/// `templateText`: the message with quoted names and the source's own label removed, so a
/// backend called "tls-node" is not read as a TLS failure. Lower case (every pattern is `/i`).
fn template_text(message: &str, label: Option<&str>) -> String {
    let chars: Vec<char> = message.chars().collect();
    let opens = |c: char| c.is_whitespace() || "(:,=[".contains(c);
    let closes = |c: Option<&char>| c.is_none_or(|c| c.is_whitespace() || ").,:;!?]".contains(*c));
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let q = chars[i];
        if "'\"`".contains(q) && (i == 0 || opens(chars[i - 1])) {
            // The quote that closes a word: followed by a space, punctuation or the end.
            if let Some(j) =
                (i + 1..chars.len()).find(|&j| chars[j] == q && closes(chars.get(j + 1)))
            {
                out.push_str("''");
                i = j + 1;
                continue;
            }
        }
        out.push(q);
        i += 1;
    }
    let mut text = out.to_lowercase();
    if let Some(l) = label.map(str::trim).filter(|l| !l.is_empty()) {
        text = text.replace(&l.to_lowercase(), "''");
    }
    text
}

/// `MESSAGE_CAUSES`: fixed phrases from the daemon's message templates and HTTP status text.
fn message_cause(m: &str) -> Option<&'static str> {
    let any = |ps: &[&str]| ps.iter().any(|p| m.contains(p));
    let status = |codes: &[&str]| {
        codes
            .iter()
            .any(|c| m.contains(&format!("(http {c})")) || has_word(m, &format!("http {c}")))
    };
    if any(&[
        "kassiber is in offline mode",
        "outbound requests are disabled",
    ]) {
        return Some("offline");
    }
    if any(&[
        "rate limited the request",
        "too many requests",
        "cooling down",
    ]) || status(&["429"])
    {
        return Some("rateLimited");
    }
    if any(&["temporarily unavailable", "service unavailable"]) || status(&["502", "503", "504"]) {
        return Some("unavailable");
    }
    let refused = [
        "401 unauthorized",
        "401 unauthorised",
        "401 forbidden",
        "403 unauthorized",
        "403 unauthorised",
        "403 forbidden",
    ]
    .iter()
    .any(|p| has_word(m, p));
    if status(&["401", "403"]) || refused || any(&["invalid api key", "authentication failed"]) {
        return Some("credentials");
    }
    // The daemon's fixed texts for an untrusted or changed backend certificate.
    if m.contains("this server's certificate changed") {
        return Some("certificateChanged");
    }
    if any(&[
        "certificate verify",
        "certificate is not trusted",
        "certificate has expired",
        "certificate is invalid",
        "tls handshake failed",
        "custom ca bundle",
        "not signed by an authority kassiber trusts",
    ]) {
        return Some("certificate");
    }
    if m.contains("no sync backend is configured") {
        return Some("noBackend");
    }
    if any(&[
        "network i/o failed",
        "connection refused",
        "connection reset",
        "connection closed",
        "could not connect",
        "timed out",
        "request budget exhausted",
    ]) || has_word(m, "timeout")
    {
        return Some("connection");
    }
    None
}

/// `classifyCause`.
fn classify_cause(
    message: Option<&str>,
    code: Option<&str>,
    label: Option<&str>,
    phrase: Option<&str>,
) -> Option<&'static str> {
    if phrase == Some("coolingDown") {
        return Some("rateLimited");
    }
    // `rate_limited` covers both 429 and 5xx, so its message decides.
    if let Some(c) = code.filter(|c| *c != "rate_limited").and_then(code_cause) {
        return Some(c);
    }
    let m = template_text(message.unwrap_or(""), label);
    // An HTTP status outranks any wording around it, which may be the server's own body.
    let found = m.match_indices("http ").find_map(|(i, _)| {
        let d = m.get(i + 5..i + 8)?;
        let ok = d.chars().all(|c| c.is_ascii_digit())
            && m[..i].chars().next_back().is_none_or(|c| !is_word(c))
            && m[i + 8..].chars().next().is_none_or(|c| !is_word(c));
        ok.then_some(d)
    });
    match found {
        Some("401" | "403") => return Some("credentials"),
        Some("429") => return Some("rateLimited"),
        Some("502" | "503" | "504") => return Some("unavailable"),
        _ => {}
    }
    if let Some(c) = message_cause(&m) {
        return Some(c);
    }
    if code == Some("rate_limited") {
        return Some("rateLimited");
    }
    if phrase == Some("cancelled") {
        return Some("cancelled");
    }
    None
}

/// One failing source as the condition carries it.
struct Detail<'a> {
    label: Option<&'a str>,
    message: Option<&'a str>,
}

/// `groupDetails` + `groupLine` for one condition's details, as `projectDetails` shows them.
fn detail_lines(details: &[Detail], privacy: crate::app::Privacy) -> Vec<String> {
    use crate::app::Privacy;
    if privacy == Privacy::HideAll || details.is_empty() {
        return Vec::new();
    }
    struct Group {
        cause: Option<&'static str>,
        message: Option<String>,
        labels: Vec<String>,
        count: usize,
    }
    let mut groups: Vec<(String, Group)> = Vec::new();
    for d in details {
        let cause = classify_cause(d.message, None, d.label, None);
        // `safeMessage`: credentials (and, stricter here, keys and descriptors) never show.
        let message = d
            .message
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .map(crate::reports::diag::redact::secret_floor_text);
        let key = match cause {
            Some(c) => format!("cause:{c}"),
            None => format!("message:{}:", message.clone().unwrap_or_default()),
        };
        let at = match groups.iter().position(|(k, _)| *k == key) {
            Some(i) => i,
            None => {
                groups.push((
                    key,
                    Group {
                        cause,
                        message: if cause.is_some() { None } else { message },
                        labels: Vec::new(),
                        count: 0,
                    },
                ));
                groups.len() - 1
            }
        };
        let g = &mut groups[at].1;
        g.count += 1;
        if let Some(l) = d.label.map(str::trim).filter(|l| !l.is_empty())
            && !g.labels.iter().any(|x| x == l)
        {
            g.labels.push(l.to_owned());
        }
    }
    // Biggest group first (a stable sort, as Array.sort).
    groups.sort_by_key(|g| std::cmp::Reverse(g.1.count));
    let digits = |s: &str| s.chars().any(|c| c.is_ascii_digit());
    let label_of = |l: &str| {
        if privacy == Privacy::HideAmounts && digits(l) {
            crate::theme::MASK.to_owned()
        } else {
            l.to_owned()
        }
    };
    groups
        .into_iter()
        .filter_map(|(_, g)| {
            let names = (!g.labels.is_empty() && g.labels.len() <= 2).then(|| {
                g.labels
                    .iter()
                    .map(|l| label_of(l))
                    .collect::<Vec<_>>()
                    .join(", ")
            });
            let text = match g.cause {
                Some(c) => Some(t(&format!("chrome:notifications.cause.{c}"))),
                None => g.message.map(|m| {
                    if privacy == Privacy::HideAmounts && digits(&m) {
                        t("chrome:notifications.withheld")
                    } else {
                        m
                    }
                }),
            };
            match (text, names) {
                (Some(text), Some(names)) => Some(if g.cause.is_some() {
                    crate::i18n::tr(
                        "chrome:notifications.detailLine",
                        &[("label", &text), ("message", &names)],
                    )
                } else {
                    crate::i18n::tr(
                        "chrome:notifications.detailLine",
                        &[("label", &names), ("message", &text)],
                    )
                }),
                (Some(text), None) if g.count > 1 => Some(crate::i18n::tr(
                    "chrome:notifications.causeCount",
                    &[("cause", &text), ("count", &g.count.to_string())],
                )),
                (Some(text), None) => Some(text),
                (None, names) => names,
            }
        })
        .collect()
}

fn conditions(app: &App) -> Vec<Condition> {
    let mut out = Vec::new();
    if app.shell.sync_failed {
        // Its Retry repeats the run as it was (`params.forceFull`).
        let rescan = app.shell.run.force_full;
        out.push(Condition {
            blocking: false,
            error: true,
            title: t(if rescan {
                "chrome:notifications.condition.bookRefreshFailed.rescanTitle"
            } else {
                "chrome:notifications.condition.bookRefreshFailed.title"
            }),
            body: t("chrome:notifications.condition.bookRefreshFailed.body"),
            details: Vec::new(),
            action: Some((
                t(if rescan {
                    "chrome:notifications.condition.bookRefreshFailed.rescanAction"
                } else {
                    "chrome:notifications.condition.bookRefreshFailed.action"
                }),
                Act::Retry,
            )),
        });
    }
    if let Some(message) = &app.shell.journals_failed {
        // `journals.process-failed`: the failure's message, then Retry with the run's options.
        out.push(Condition {
            blocking: false,
            error: true,
            title: t("chrome:notifications.condition.journalsFailed.title"),
            body: t("chrome:notifications.condition.journalsFailed.body"),
            details: detail_lines(
                &[Detail {
                    label: None,
                    message: Some(message.as_str()),
                }],
                app.prefs.privacy,
            ),
            action: Some((
                t("chrome:notifications.condition.journalsFailed.action"),
                Act::RetryJournals,
            )),
        });
    }
    if let Some(b) = &app.badges {
        if b.journals_needs_processing {
            out.push(Condition {
                blocking: false,
                error: false,
                title: t("chrome:notifications.condition.journalsStale.title"),
                body: t("chrome:notifications.condition.journalsStale.body"),
                details: Vec::new(),
                action: Some((
                    t("chrome:notifications.condition.journalsStale.action"),
                    Act::ProcessJournals,
                )),
            });
        }
        let n = b.total() as usize;
        if n > 0 {
            let blocking = b.quarantine.unwrap_or(0.0) > 0.0;
            let body_key = if blocking {
                "chrome:notifications.condition.reviewPending.blockingBody"
            } else {
                "chrome:notifications.condition.reviewPending.body"
            };
            out.push(Condition {
                blocking,
                error: false,
                title: t("chrome:notifications.condition.reviewPending.title"),
                body: tc(body_key, n, &[]),
                details: Vec::new(),
                action: Some((
                    t("chrome:notifications.condition.reviewPending.action"),
                    Act::Review,
                )),
            });
        }
    }
    let failed: Vec<&serde_json::Value> = app
        .read("ui.freshness.status")
        .and_then(|f| f["sources"].as_array())
        .map(|s| {
            s.iter()
                .filter(|x| x["status"] == "failed" || x["blocking_reports"] == true)
                .collect()
        })
        .unwrap_or_default();
    if !failed.is_empty() {
        let blocking = failed.iter().any(|x| x["blocking_reports"] == true);
        let n = failed.len();
        // Each source's label and last error, grouped by cause (`sourceConditionsFromSnapshot`).
        let details: Vec<Detail> = failed
            .iter()
            .map(|x| Detail {
                label: x["source_label"].as_str(),
                message: x["last_error_message"].as_str(),
            })
            .collect();
        out.push(Condition {
            blocking,
            error: true,
            title: tc("chrome:notifications.condition.sourceFailed.title", n, &[]),
            body: t(if blocking {
                "chrome:notifications.condition.sourceFailed.blockingBody"
            } else {
                "chrome:notifications.condition.sourceFailed.body"
            }),
            details: detail_lines(&details, app.prefs.privacy),
            action: Some((
                t("chrome:notifications.condition.sourceFailed.action"),
                Act::Sources,
            )),
        });
    }
    // App-level conditions about saved unlock methods (unlockNotifications.ts).
    for (_, title, body, details, action) in crate::secrets::conditions(app) {
        out.push(Condition {
            blocking: false,
            error: false,
            title,
            body,
            details: details.into_iter().collect(),
            action: Some((action, Act::Security)),
        });
    }
    out
}

fn act(s: &mut App, a: Act) {
    s.shell.popup = None;
    match a {
        Act::ProcessJournals => s.process_journals(),
        Act::Review => s.navigate("/review"),
        Act::Sources => s.navigate("/connections"),
        Act::Retry => {
            let force = s.shell.run.force_full;
            s.refresh_book(force)
        }
        Act::RetryJournals => {
            let auto_pair = s.shell.run.journals_auto_pair;
            s.process_journals_with(auto_pair)
        }
        Act::Security => crate::secrets::open_security(s),
    }
}

fn tone_of(tk: &Tk, t: ToastTone) -> Tone {
    match t {
        ToastTone::Success => tk.good,
        ToastTone::Info => crate::ui::neutral_tone(tk),
        ToastTone::Warning => tk.warn,
        ToastTone::Error => tk.alert,
    }
}

fn section(tk: &Tk, title: &str) -> El {
    row()
        .w_full()
        .px(tk.r(0.625))
        .pt(tk.r(0.5))
        .pb(tk.r(0.25))
        .child(
            text(title.to_owned())
                .font(tk.txs(500.0))
                .color(tk.muted_fg),
        )
}

fn card(
    tk: &Tk,
    key: String,
    tone: &Tone,
    glyph: &'static rune::Icon,
    title: &str,
    body: Option<&str>,
    action: Option<El>,
) -> El {
    card_with(tk, key, tone, glyph, title, body, &[], action)
}

/// `InlineDetails`: up to two lines under the body, then "+N more".
#[allow(clippy::too_many_arguments)]
fn card_with(
    tk: &Tk,
    key: String,
    tone: &Tone,
    glyph: &'static rune::Icon,
    title: &str,
    body: Option<&str>,
    details: &[String],
    action: Option<El>,
) -> El {
    let mut texts = col().grow(1.0).min_w(0.0).gap(tk.r(0.125)).child(
        text(title.to_owned())
            .font(tk.tsm(500.0))
            .color(tk.fg)
            .wrap(),
    );
    if let Some(b) = body {
        texts = texts.child(
            text(b.to_owned())
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .wrap(),
        );
    }
    if !details.is_empty() {
        let mut list = col()
            .w_full()
            .gap(tk.r(0.125))
            .role("list")
            .label(t("chrome:notifications.details"));
        for line in details.iter().take(2) {
            list = list.child(
                text(line.clone())
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg)
                    .wrap_anywhere()
                    .role("listitem"),
            );
        }
        if details.len() > 2 {
            list = list.child(
                text(tc(
                    "chrome:notifications.moreDetails",
                    details.len() - 2,
                    &[],
                ))
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .role("listitem"),
            );
        }
        texts = texts.child(list);
    }
    if let Some(a) = action {
        texts = texts.child(row().pt(tk.r(0.25)).child(a));
    }
    row()
        .key(key)
        .w_full()
        .px(tk.r(0.625))
        .py(tk.r(0.5))
        .radius(tk.r(0.5))
        .gap(tk.r(0.625))
        .items_start()
        .child(
            row()
                .size(tk.r(1.75), tk.r(1.75))
                .center()
                .radius(tk.r(0.375))
                .fill(tone.bg)
                .child(ic(glyph, tk.r(1.0), tone.text)),
        )
        .child(texts)
}

pub fn button(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let conds = conditions(app);
    let blocking = conds.iter().any(|c| c.blocking);
    let open = app.shell.popup == Some(Popup::Notifications);
    let label = if conds.is_empty() {
        t("chrome:notifications.label")
    } else {
        tc("chrome:notifications.labelActive", conds.len(), &[])
    };
    let mut b = bar_button(tk, "sh-bell", &I::BELL, &label, false)
        .expanded(open)
        .on_press(|s: &mut App| s.shell.toggle(Popup::Notifications));
    if blocking {
        b = b
            .fill(tk.alert.solid.alpha(0.10))
            .border(1.0, tk.alert.solid.alpha(0.35))
            .border_layout();
    } else if !conds.is_empty() {
        b = b
            .fill(tk.warn.solid.alpha(0.10))
            .border(1.0, tk.warn.solid.alpha(0.35))
            .border_layout();
    }
    if !conds.is_empty() {
        b = b.child(
            row()
                .absolute(tk.r(1.5), -tk.r(0.25))
                .size(tk.r(1.0), tk.r(1.0))
                .center()
                .radius(tk.r(0.5))
                .fill(tk.destructive)
                .a11y_hidden()
                .child(
                    text(format!("{}", conds.len()))
                        .font(tk.t3xs(600.0))
                        .color(Color::WHITE),
                ),
        );
    }
    if open {
        super::mount_popup_as(
            tk,
            layer,
            "sh-notifications",
            "sh-bell",
            Placement::BottomEnd,
            panel(app, tk, conds),
            false,
        );
    }
    b
}

fn panel(app: &App, tk: &Tk, conds: Vec<Condition>) -> El {
    // The glass header (`h2` and a ghost `xs` "Clear recent", disabled while there's nothing
    // recent), then the list; rows line up with the header's 1 rem inset.
    let can_clear = app.shell.recent_here().next().is_some();
    let mut clear = row()
        .key("sh-notes-clear")
        .h(tk.r(1.75))
        .px(tk.r(0.5))
        .center()
        .radius(tk.r(0.375))
        .role("button")
        .label(t("chrome:notifications.clearRecent"))
        .child(
            text(t("chrome:notifications.clearRecent"))
                .font(tk.txs(500.0))
                .color(tk.fg),
        );
    clear = if can_clear {
        clear
            .hover(if tk.dark {
                tk.accent.alpha(0.5)
            } else {
                tk.accent
            })
            .focusable(true)
            .on_press(|s: &mut App| {
                let here = s.shell.book.clone();
                s.shell.recent.retain(|r| Some(&r.book) != here.as_ref());
            })
    } else {
        clear.opacity(0.5).disabled(true)
    };
    let header = super::popover_header(tk, &t("chrome:notifications.label"), None, Some(clear));
    let mut body = col()
        .key("sh-notes-body")
        .w_full()
        .max_h(tk.r(44.0).min(app.shell.window_h - 4.0 * tk.rem))
        .padding(super::menu_pad(tk))
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        });
    let running = app.busy.syncing || app.busy.processing;
    if conds.is_empty() && !running && app.shell.recent_here().next().is_none() {
        // `px-3 py-4 text-center text-xs text-muted-foreground`, in place of the list.
        body = col()
            .key("sh-notes-body")
            .w_full()
            .px(tk.r(1.0))
            .py(tk.r(1.25))
            .items_center()
            .child(
                text(t("chrome:notifications.empty"))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg),
            );
    }
    if !conds.is_empty() {
        body = body.child(section(tk, &t("chrome:notifications.sections.attention")));
        for (i, c) in conds.iter().enumerate() {
            let tone = if c.blocking || c.error {
                tk.alert
            } else {
                tk.warn
            };
            let glyph = if c.error || c.blocking {
                &I::CIRCLE_ALERT
            } else {
                &I::TRIANGLE_ALERT
            };
            let action = c.action.clone().map(|(label, a)| {
                row()
                    .key(format!("sh-note-act{i}"))
                    .h(tk.r(1.5))
                    .px(tk.r(0.5))
                    .center()
                    .radius(tk.r(0.375))
                    .border(1.0, tk.input)
                    .border_layout()
                    .hover(tk.accent)
                    .role("button")
                    .label(label.clone())
                    .focusable(true)
                    .on_press(move |s: &mut App| act(s, a))
                    .child(text(label).font(tk.txs(500.0)).color(tk.fg))
            });
            body = body.child(card_with(
                tk,
                format!("sh-note{i}"),
                &tone,
                glyph,
                &c.title,
                Some(&c.body),
                &c.details,
                action,
            ));
        }
    }
    if running {
        body = body.child(section(tk, &t("chrome:notifications.sections.running")));
        body = body.child(job_row(app, tk));
    }
    if app.shell.recent_here().next().is_some() {
        body = body.child(section(tk, &t("chrome:notifications.sections.recent")));
        for (i, r) in app.shell.recent_here().enumerate() {
            let tone = tone_of(tk, r.tone);
            let glyph = match r.tone {
                ToastTone::Success => &I::CIRCLE_CHECK,
                ToastTone::Error => &I::CIRCLE_ALERT,
                ToastTone::Warning => &I::TRIANGLE_ALERT,
                ToastTone::Info => &I::INFO,
            };
            let ago = app.clock - r.at;
            let when = if ago < 60.0 {
                t("chrome:footer.justNow")
            } else {
                crate::i18n::tr(
                    "chrome:footer.minutesAgo",
                    &[("minutes", &format!("{}", (ago / 60.0).floor()))],
                )
            };
            let body_text = match &r.body {
                Some(b) if !app.prefs.hide_sensitive() => format!("{b} · {when}"),
                Some(_) => format!("{} · {when}", t("chrome:notifications.withheld")),
                None => when,
            };
            body = body.child(card(
                tk,
                format!("sh-recent{i}"),
                &tone,
                glyph,
                &r.title,
                Some(&body_text),
                None,
            ));
        }
    }
    col()
        .w(tk.r(30.0))
        .role("dialog")
        .label(t("chrome:notifications.label"))
        .child(header)
        .child(body)
}

/// `JobRow`: a spinner, the job's title (for a book refresh, a button that brings its card back),
/// its progress bar, and the phase under it.
fn job_row(app: &App, tk: &Tk) -> El {
    use super::maint::Job;
    let refresh = app.busy.syncing;
    let maint = app.shell.maint.as_ref().filter(|m| {
        m.job
            == if refresh {
                Job::BookRefresh
            } else {
                Job::Journals
            }
            && !m.failed
    });
    let title = t(match (refresh, app.shell.run.force_full) {
        (true, true) => "chrome:notifications.job.bookRescan",
        (true, false) => "chrome:notifications.job.bookRefresh",
        (false, _) => "chrome:notifications.job.journalProcessing",
    });
    // `projectFreeText`: a phase can name a source, and a name can carry an amount.
    let phase = maint
        .map(|m| m.label.trim().to_owned())
        .or_else(|| app.busy.progress.clone())
        .filter(|p| !p.is_empty() && *p != title)
        .and_then(|p| match app.prefs.privacy {
            Privacy::HideAll => None,
            Privacy::HideAmounts if p.chars().any(|c| c.is_ascii_digit()) => {
                Some(t("chrome:notifications.withheld"))
            }
            _ => Some(p),
        });
    let indeterminate = maint.is_none_or(|m| m.indeterminate || m.value.is_none());
    let value = maint.and_then(|m| m.value).unwrap_or(0.0).clamp(0.0, 100.0) as f32;
    // `h-1.5 rounded-full bg-muted`, the fill `bg-primary` (half width while indeterminate).
    let track = tk.r(26.0);
    let fill = if indeterminate {
        let w = track / 2.0;
        let mut f = el().w(w).h_full().radius(tk.r(0.1875)).fill(tk.primary);
        if !app.shell.marks.reduced_motion() {
            f = f.looping(rune::LoopSpec::slide(0.9, [-w, 0.0], [track, 0.0]));
        } else {
            f = f.w_full();
        }
        f
    } else {
        el().w(track * value / 100.0)
            .h_full()
            .radius(tk.r(0.1875))
            .fill(tk.primary)
    };
    let bar = el()
        .key("sh-note-running-bar")
        .w_full()
        .h(tk.r(0.375))
        .radius(tk.r(0.1875))
        .fill(tk.muted)
        .clip_rounded(tk.r(0.1875))
        .a11y_hidden()
        .child(fill);
    let mut head = row()
        .gap(tk.r(0.25))
        .center_y()
        .child(text(title.clone()).font(tk.tsm(500.0)).color(tk.fg).wrap());
    let mut words = col().grow(1.0).min_w(0.0);
    if refresh {
        head = head.child(ic(&I::CHEVRON_RIGHT, tk.r(0.875), tk.muted_fg));
        words = words.child(
            row()
                .key("sh-note-running-open")
                .radius(tk.r(0.125))
                .role("button")
                .label(title)
                .focusable(true)
                .on_press(|s: &mut App| {
                    s.shell.sync_card_open = true;
                    s.shell.popup = None;
                })
                .child(head),
        );
    } else {
        words = words.child(head);
    }
    let mut progress = col().w_full().pt(tk.r(0.375)).child(bar);
    if let Some(p) = phase {
        progress = progress.child(
            text(p)
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .wrap()
                .mt(tk.r(0.25)),
        );
    }
    row()
        .key("sh-note-running")
        .w_full()
        .px(tk.r(0.5))
        .py(tk.r(0.5))
        .gap(tk.r(0.5))
        .radius(tk.r(0.375))
        .items_start()
        .hover(tk.accent.alpha(tk.accent.a * 0.4))
        .child(
            ic(&I::LOADER_CIRCLE, tk.r(1.0), tk.muted_fg)
                .looping(rune::LoopSpec::rotation(1.0))
                .mt(tk.r(0.125)),
        )
        .child(words.child(progress))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn certificate_causes_follow_codes_and_fixed_texts() {
        assert_eq!(
            classify_cause(None, Some("tls_certificate_untrusted"), None, None),
            Some("certificate")
        );
        assert_eq!(
            classify_cause(None, Some("tls_certificate_changed"), None, None),
            Some("certificateChanged")
        );
        assert_eq!(
            classify_cause(
                Some(
                    "This server's certificate changed: it no longer matches the certificate trusted for this backend"
                ),
                None,
                None,
                None
            ),
            Some("certificateChanged")
        );
        assert_eq!(
            classify_cause(
                Some("The server's TLS certificate is not signed by an authority Kassiber trusts"),
                None,
                None,
                None
            ),
            Some("certificate")
        );
        assert_eq!(
            classify_cause(Some("TLS handshake failed"), None, None, None),
            Some("certificate")
        );
    }

    #[test]
    fn names_in_quotes_and_the_label_do_not_classify() {
        assert_eq!(
            classify_cause(
                Some("backend 'tls handshake failed' rejected the request"),
                None,
                None,
                None
            ),
            None
        );
        assert_eq!(
            classify_cause(Some("tls-node timed out"), None, Some("tls-node"), None),
            Some("connection")
        );
        assert_eq!(
            classify_cause(Some("'a' timed out contacting 'b'"), None, None, None),
            Some("connection")
        );
        assert_eq!(
            classify_cause(
                Some("Bob's node: HTTP 503 Service Unavailable"),
                None,
                None,
                None
            ),
            Some("unavailable")
        );
        assert_eq!(
            classify_cause(
                Some("server said (HTTP 401) but certificate verify failed"),
                None,
                None,
                None
            ),
            Some("credentials")
        );
        assert_eq!(
            classify_cause(
                Some("mempool rate limited the request"),
                Some("rate_limited"),
                None,
                None
            ),
            Some("rateLimited")
        );
        assert_eq!(
            classify_cause(Some("something odd"), Some("rate_limited"), None, None),
            Some("rateLimited")
        );
        assert_eq!(
            classify_cause(Some("timeouts happen"), None, None, None),
            None
        );
        assert_eq!(
            classify_cause(Some("something odd"), None, None, None),
            None
        );
    }

    #[test]
    fn every_cause_has_a_line_in_both_languages() {
        for l in [crate::i18n::Lang::En, crate::i18n::Lang::De] {
            crate::i18n::set_lang(l);
            for c in CAUSES {
                let k = format!("chrome:notifications.cause.{c}");
                assert_ne!(t(&k), k);
            }
        }
        crate::i18n::set_lang(crate::i18n::Lang::En);
    }

    #[test]
    fn details_group_by_cause_with_few_names() {
        crate::i18n::set_lang(crate::i18n::Lang::En);
        let changed = Some("This server's certificate changed: it no longer matches");
        let d = [
            Detail {
                label: Some("Home"),
                message: changed,
            },
            Detail {
                label: Some("Cold"),
                message: Some("network I/O failed"),
            },
            Detail {
                label: Some("Vault 2"),
                message: changed,
            },
        ];
        let lines = detail_lines(&d, crate::app::Privacy::Normal);
        assert_eq!(
            lines,
            vec![
                "Server certificate changed; not trusted: Home, Vault 2".to_owned(),
                "Connection to the server failed: Cold".to_owned()
            ]
        );
        let hidden = detail_lines(&d, crate::app::Privacy::HideAmounts);
        assert_eq!(
            hidden[0],
            "Server certificate changed; not trusted: Home, ••••"
        );
        assert!(detail_lines(&d, crate::app::Privacy::HideAll).is_empty());
    }
}
