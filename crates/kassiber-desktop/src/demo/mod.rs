//! The bundled demo book (docs/reference/demo.md): **Try the demo** on Welcome and in the book
//! switcher writes a fresh copy with `kassiber projects demo` (on a worker thread; it unpacks
//! the book and builds its local analysis index, ~3 s) and restarts the daemon on it with the
//! demo's own offline and agent-access preference files. While it is open a **Demo** tag sits
//! beside the book name in the title bar ([`tag`]) and the book switcher offers **Leave demo**
//! and **Reset demo** ([`switcher_rows`]); a notice says so once, when it opens.
//!
//! - Nothing here reaches the network: the CLI writes local files, and the demo book has no
//!   sync backend (sync and connection checks fail closed with "no backend"). AI and agents
//!   keep their usual explicit gates; the demo's providers are its own (they live in its
//!   database).
//! - The demo never becomes the remembered book: while it is open the preferences file keeps
//!   the real book's identity and route (`persisted_identity`, `persisted_route`), and leaving
//!   restores the daemon's previous data root and removes the copy.
//! - **Every way out is the same.** The demo's preference files travel with the daemon switch
//!   that opens it (`app::switch_root`'s `env`), never as ambient launch state, so any switch to
//!   another root (Leave demo, Reset welcome, the lock screen's "Open other books", a book set
//!   opened from the menu) runs on the user's own files, and `ended` clears the tag, the
//!   saved state and the copy. Exits that keep the root (Welcome) go back to the launch root
//!   first ([`leave_then`]), so a book set up afterwards never lands in the copy.
//! - `KASSIBER_DEMO_ROOT` (development and tests) names the directory that holds demo copies;
//!   without it the CLI uses `demo/` under the state root.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::Duration;

use rune::{El, Tx, col, row, text};
use serde_json::Value;

use crate::app::{App, ToastTone};
use crate::i18n::t;
use crate::icons as I;
use crate::onboarding::model::Identity;
use crate::theme::Tk;
use crate::ui::ic;

/// What `projects demo` wrote.
#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    pub data_root: PathBuf,
    pub state_root: PathBuf,
    /// The demo's own preference files, passed to the daemon and to agent commands.
    pub env: Vec<(String, String)>,
}

/// What the app goes back to on **Leave demo**.
#[derive(Clone, Debug, Default)]
pub struct Saved {
    root: Option<PathBuf>,
    identity: Option<Identity>,
    route: String,
}

#[derive(Default)]
pub struct Demo {
    pub active: Option<Session>,
    /// A `projects demo` run in flight.
    job: Option<Receiver<Result<Session, String>>>,
    pub error: Option<String>,
    saved: Option<Saved>,
    /// The daemon is switching to or from the demo (the tag's and switcher's spinner).
    pub switching: bool,
    /// The demo's own stack goal (`overview::goal::target`): `None` until it is edited in the
    /// demo (a round target above the demo's stack), `Some(None)` once removed there. The
    /// owner's goal lives in the shared overview layout and never shows against demo coins.
    pub goal: Option<Option<u64>>,
}

impl Demo {
    pub fn busy(&self) -> bool {
        self.job.is_some() || self.switching
    }
}

/// The identity the preferences file keeps: the real book's while the demo is open.
pub fn persisted_identity(app: &App) -> Option<Identity> {
    match (&app.demo.active, &app.demo.saved) {
        (Some(_), Some(saved)) => saved.identity.clone(),
        _ => app.onboarding.identity.clone(),
    }
}

/// The route the preferences file keeps: the real book's while the demo is open.
pub fn persisted_route(app: &App) -> String {
    match (&app.demo.active, &app.demo.saved) {
        (Some(_), Some(saved)) => saved.route.clone(),
        (Some(_), None) => String::new(),
        _ => app.route.0.clone(),
    }
}

/// The demo's preference environment, for the agent command in Settings → Assistant.
pub fn agent_env(app: &App) -> Option<&[(String, String)]> {
    app.demo.active.as_ref().map(|s| s.env.as_slice())
}

/// `claude mcp add … -e KEY=VALUE … -- <launcher> <args>`.
pub fn claude_command(launcher: &[String], args: &[String], env: &[(String, String)]) -> String {
    let q = crate::settings::model::shell_quote;
    let mut parts: Vec<String> = ["claude", "mcp", "add", "kassiber"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    for (k, v) in env {
        parts.push("-e".into());
        parts.push(format!("{k}={v}"));
    }
    parts.push("--".into());
    parts.extend(launcher.iter().cloned());
    parts.extend(args.iter().cloned());
    parts.iter().map(|s| q(s)).collect::<Vec<_>>().join(" ")
}

/// The JSON host config with the demo's environment.
pub fn json_config(launcher: &[String], args: &[String], env: &[(String, String)]) -> String {
    let (cmd, rest) = launcher
        .split_first()
        .map(|(c, r)| (c.clone(), r.to_vec()))
        .unwrap_or(("kassiber".into(), vec![]));
    let all: Vec<String> = rest.into_iter().chain(args.iter().cloned()).collect();
    let env: serde_json::Map<String, Value> = env
        .iter()
        .map(|(k, v)| (k.clone(), Value::String(v.clone())))
        .collect();
    serde_json::to_string_pretty(
        &serde_json::json!({"mcpServers": {"kassiber": {"command": cmd, "args": all, "env": env}}}),
    )
    .unwrap_or_default()
}

// ---- Creating a copy ---------------------------------------------------------------------

const TIMEOUT: Duration = Duration::from_secs(180);
const MAX_OUTPUT: u64 = 256 * 1024;

/// `--path` for `projects demo`: the development override, or the CLI's default.
fn demo_base() -> Option<String> {
    std::env::var("KASSIBER_DEMO_ROOT")
        .ok()
        .filter(|v| !v.trim().is_empty())
}

/// The CLI's answer: the copy's roots and preference environment.
pub fn parse(stdout: &[u8]) -> Result<Session, String> {
    let v: Value = serde_json::from_slice(stdout)
        .map_err(|_| "The Kassiber CLI returned no readable answer.".to_owned())?;
    if v["kind"] == "error" {
        let m = v["error"]["message"]
            .as_str()
            .unwrap_or("The demo book could not be written.");
        return Err(m.to_owned());
    }
    let d = &v["data"];
    let data_root = d["data_root"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("The Kassiber CLI named no demo book.")?;
    let state_root = d["state_root"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("The Kassiber CLI named no demo book.")?;
    let env = d["preference_environment"]
        .as_object()
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
                .filter(|(k, _)| k.starts_with("KASSIBER_"))
                .collect()
        })
        .unwrap_or_default();
    Ok(Session {
        data_root: PathBuf::from(data_root),
        state_root: PathBuf::from(state_root),
        env,
    })
}

/// Runs `kassiber --machine projects demo [--path …]` and waits for it.
fn run_create() -> Result<Session, String> {
    let cmd = crate::native::cli::kassiber_command().map_err(|(m, _)| m)?;
    let mut c = Command::new(&cmd.program);
    c.args(["--machine", "projects", "demo"]);
    if let Some(base) = demo_base() {
        c.args(["--path", &base]);
    }
    let mut child = c
        .current_dir(&cmd.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "Could not start the Kassiber CLI.".to_owned())?;
    let stdout = child
        .stdout
        .take()
        .ok_or("Could not read the Kassiber CLI.")?;
    let (send, recv) = channel();
    std::thread::spawn(move || {
        use std::io::Read;
        let mut body = Vec::new();
        let r = stdout.take(MAX_OUTPUT).read_to_end(&mut body).map(|_| body);
        let _ = send.send(r);
    });
    let body = match recv.recv_timeout(TIMEOUT) {
        Ok(Ok(body)) => body,
        _ => {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Writing the demo book took too long.".into());
        }
    };
    let _ = child.wait();
    parse(&body)
}

/// **Try the demo** / **Reset demo**: a fresh copy on a worker thread.
pub fn start(app: &mut App) {
    if app.demo.busy() {
        return;
    }
    app.demo.error = None;
    let (send, recv) = channel();
    std::thread::spawn(move || {
        let _ = send.send(run_create());
    });
    app.demo.job = Some(recv);
}

pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let Some(rx) = &app.demo.job else {
        return false;
    };
    match rx.try_recv() {
        Err(TryRecvError::Empty) => {
            tx.wake_after(Duration::from_millis(100));
            false
        }
        Err(TryRecvError::Disconnected) => {
            app.demo.job = None;
            fail(app, "The demo book could not be written.".into());
            true
        }
        Ok(result) => {
            app.demo.job = None;
            match result {
                Ok(session) => open(app, session),
                Err(e) => fail(app, e),
            }
            true
        }
    }
}

fn fail(app: &mut App, message: String) {
    app.demo.error = Some(message.clone());
    app.toast(
        ToastTone::Error,
        &t("chrome:demo.startFailed"),
        Some(message),
        Some("demo"),
    );
}

/// The demo book's identity (the bundled book's labels); kept in memory only.
fn identity() -> Identity {
    Identity {
        name: "Full Accounting".into(),
        workspace: "Regtest Demo".into(),
        profile: "Full Accounting".into(),
        country: "AT".into(),
        encrypted: false,
        tax_country: Some("at".into()),
        fiat_currency: Some("EUR".into()),
        tax_long_term_days: Some(0),
        gains_algorithm: Some("MOVING_AVERAGE_AT".into()),
        database_mode: Some("plaintext".into()),
        backend_setup_mode: Some("custom".into()),
        ..Identity::default()
    }
}

/// Restarts the daemon on the new copy with the demo's preference files (in the background:
/// the tag and switcher show "Preparing" until it is ready).
fn open(app: &mut App, session: Session) {
    let reset = app.demo.active.is_some();
    // What Leave demo goes back to: the book open now, or (a reset) what the first open saved.
    // The switch ends the copy it replaces (`ended`), so it is carried across.
    let saved = if reset {
        app.demo.saved.clone()
    } else {
        Some(Saved {
            root: app.daemon.launch.data_root.clone(),
            identity: app.onboarding.identity.clone(),
            route: app.route.0.clone(),
        })
    };
    app.demo.switching = true;
    let (root, env) = (Some(session.data_root.clone()), session.env.clone());
    crate::app::switch_root(
        app,
        root,
        env,
        move |app: &mut App, result: Result<(), String>| {
            app.demo.switching = false;
            if let Err(e) = result {
                // The book that was open stays; its screens read again.
                if app.onboarding.identity.is_some() && !app.locked {
                    app.load_book();
                }
                fail(app, e);
                return;
            }
            app.demo.saved = saved;
            app.demo.active = Some(session);
            app.demo.goal = None;
            crate::onboarding::enter_app(app, identity());
            // The one notice: the tag in the title bar marks the demo from here on.
            let (tone, key) = if reset {
                (ToastTone::Success, "chrome:demo.resetDone")
            } else {
                (ToastTone::Info, "chrome:demo.opened")
            };
            app.toast(tone, &t(key), None, Some("demo"));
        },
    );
}

/// What follows leaving the demo.
pub enum Then {
    /// **Leave demo**: the book that was open before, on its page.
    Restore,
    /// Welcome (Reset welcome, "Open other books"): the launch's own root, no identity.
    Welcome,
    /// A book set picked from the menu: opened on the user's own daemon.
    SelectProject(String),
}

/// **Leave demo**.
pub fn leave(app: &mut App) {
    leave_then(app, Then::Restore);
}

/// The one way out of the demo: the daemon goes back to the root the demo replaced (the
/// launch's own root for Welcome) on the user's own preference files, the copy and the demo's
/// state go (`ended`, from the switch), then `then`. While it switches the tag shows
/// "Preparing"; requests made meanwhile reach the restored daemon, never the copy. If the
/// switch fails the demo stays open.
pub fn leave_then(app: &mut App, then: Then) {
    if app.demo.active.is_none() {
        return;
    }
    let saved = app.demo.saved.clone().unwrap_or_default();
    app.demo.switching = true;
    let root = match then {
        Then::Welcome => app.onboarding.seed.clone(),
        _ => saved.root.clone(),
    };
    if matches!(then, Then::Welcome) {
        // Welcome takes over now (it reads nothing until the switch is decided).
        app.onboarding.identity = None;
        crate::onboarding::enter_welcome(app);
    }
    crate::app::switch_root(
        app,
        root,
        Vec::new(),
        move |app: &mut App, result: Result<(), String>| {
            app.demo.switching = false;
            if let Err(e) = result {
                // Still in the demo: back into it, never Welcome on the demo's daemon.
                if app.demo.active.is_some() {
                    crate::onboarding::enter_app(app, identity());
                }
                app.toast(
                    ToastTone::Error,
                    &t("chrome:demo.leaveFailed"),
                    Some(e),
                    Some("demo"),
                );
                return;
            }
            // `ended` already ran (the environment changed); this is the case of a demo whose own
            // root and environment equalled the restored ones, which cannot happen, but leaves
            // nothing behind either way.
            if app.demo.active.is_some() {
                ended(app);
            }
            match then {
                Then::Restore => match saved.identity {
                    Some(identity) => {
                        crate::onboarding::enter_app(app, identity);
                        if !saved.route.is_empty() && saved.route != "/" {
                            app.navigate(&saved.route);
                        }
                    }
                    None => {
                        app.onboarding.identity = None;
                        crate::onboarding::enter_welcome(app);
                    }
                },
                Then::Welcome => {
                    let explicit = app.onboarding.seed_explicit;
                    app.daemon.launch.explicit_root = explicit;
                    app.launch.explicit_root = explicit;
                }
                Then::SelectProject(id) => {
                    if let Some(identity) = saved.identity {
                        crate::onboarding::enter_app(app, identity);
                    }
                    crate::shell::books::select_project(app, &id);
                }
            }
        },
    );
}

/// The daemon left the demo's environment (any switch away from it): the tag, the saved
/// state and the copy go. Nothing of the demo is kept.
pub fn ended(app: &mut App) {
    let Some(session) = app.demo.active.take() else {
        return;
    };
    app.demo.saved = None;
    app.demo.error = None;
    app.demo.goal = None;
    remove_copy(&session.state_root);
}

/// Removes a demo copy, only when it carries the demo marker (the CLI prunes the rest).
fn remove_copy(root: &Path) {
    if root.join("config").join("demo.json").is_file() {
        let _ = std::fs::remove_dir_all(root);
    }
}

// ---- Views -------------------------------------------------------------------------------

/// The **Demo** tag beside the book name in the title bar while the demo is open (part of the
/// book crumb, so a press opens the switcher with Leave and Reset). A spinner while switching.
pub fn tag(app: &App, tk: &Tk) -> Option<El> {
    app.demo.active.as_ref()?;
    let amber = amber(tk);
    let glyph = if app.demo.busy() {
        ic(&I::LOADER_CIRCLE, tk.r(0.75), amber).looping(rune::LoopSpec::rotation(1.0))
    } else {
        ic(&I::FLASK_CONICAL, tk.r(0.75), amber)
    };
    Some(
        row()
            .key("demo-tag")
            .gap(tk.r(0.25))
            .px(tk.r(0.375))
            .h(tk.r(1.25))
            .center_y()
            .radius(tk.r(0.375))
            .fill(amber.alpha(0.12))
            .a11y_hidden()
            .child(glyph)
            .child(
                text(t("chrome:demo.badge"))
                    .font(tk.txs(600.0))
                    .color(amber),
            ),
    )
}

/// The demo's mark colour (amber, as the web's demo badge).
fn amber(tk: &Tk) -> rune::Color {
    rune::color::hex(if tk.dark { "#fbbf24" } else { "#b45309" }).expect("hex")
}

/// The book switcher's demo rows (above **Manage books**): **Try the demo book**, or in the demo
/// **Leave demo** and **Reset demo**.
pub fn switcher_rows(app: &App, tk: &Tk) -> Vec<El> {
    let mut rows = vec![switcher_row(app, tk)];
    if app.demo.active.is_some() && !app.demo.busy() {
        rows.push(
            menu_row(
                tk,
                "sh-books-demo-reset",
                &I::ROTATE_CCW,
                t("chrome:demo.reset"),
            )
            .on_press(|s: &mut App| {
                s.shell.popup = None;
                start(s);
            }),
        );
    }
    rows
}

fn menu_row(tk: &Tk, key: &str, glyph: &'static rune::Icon, label: String) -> El {
    row()
        .key(key)
        .w_full()
        .px(tk.r(0.5))
        .py(tk.r(0.375))
        .gap(tk.r(0.5))
        .center_y()
        .radius(tk.r(0.25))
        .hover(tk.accent)
        .role("button")
        .label(label.clone())
        .child(ic(glyph, tk.r(1.0), tk.muted_fg))
        .child(text(label).font(tk.tsm(400.0)).color(tk.fg))
}

fn switcher_row(app: &App, tk: &Tk) -> El {
    let busy = app.demo.busy();
    let in_demo = app.demo.active.is_some();
    let label = t(if busy {
        "chrome:demo.preparing"
    } else if in_demo {
        "chrome:demo.leave"
    } else {
        "chrome:demo.tryEntry"
    });
    let glyph = if in_demo {
        &I::LOG_OUT
    } else {
        &I::FLASK_CONICAL
    };
    let mut r = row()
        .key("sh-books-demo")
        .w_full()
        .px(tk.r(0.5))
        .py(tk.r(0.375))
        .gap(tk.r(0.5))
        .center_y()
        .radius(tk.r(0.25))
        .hover(tk.accent)
        .role("button")
        .label(label.clone())
        .disabled(busy)
        .child(if busy {
            ic(&I::LOADER_CIRCLE, tk.r(1.0), tk.muted_fg).looping(rune::LoopSpec::rotation(1.0))
        } else {
            ic(glyph, tk.r(1.0), tk.muted_fg)
        })
        .child(text(label).font(tk.tsm(400.0)).color(tk.fg));
    if !busy {
        r = r.on_press(move |s: &mut App| {
            s.shell.popup = None;
            if in_demo {
                leave(s);
            } else {
                start(s);
            }
        });
    }
    r
}

/// The note under the agent command in Settings → Assistant while the demo is open.
pub fn agents_note(app: &App, tk: &Tk) -> Option<El> {
    app.demo.active.as_ref()?;
    Some(
        col().w_full().px(tk.r(1.0)).pb(tk.r(0.5)).child(
            text(t("chrome:demo.mcpHint"))
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .wrap(),
        ),
    )
}

// ---- Scenes ------------------------------------------------------------------------------

/// Headless: pumps until a switch is decided and the reads it sent have answered (or 30 s pass).
fn settle(a: &mut App) {
    a.settle(Duration::from_secs(30));
}

/// A live demo for headless runs: refuses without `KASSIBER_DEMO_ROOT`, so a scene never
/// writes under the real state root.
fn live(a: &mut App) {
    if demo_base().is_none() {
        a.demo.error = Some("KASSIBER_DEMO_ROOT is not set".into());
        return;
    }
    match run_create() {
        Ok(session) => open(a, session),
        Err(e) => fail(a, e),
    }
    settle(a);
    settle(a);
}

fn light(a: &mut App) {
    a.prefs.theme = crate::app::ThemeMode::Light;
}

fn de(a: &mut App) {
    a.prefs.lang = crate::i18n::Lang::De;
}

pub static SCENES: &[crate::app::Scene] = &[
    ("demo-live", live),
    ("demo-live-de", |a| {
        de(a);
        live(a);
    }),
    ("demo-live-light", |a| {
        light(a);
        live(a);
    }),
    // Leave: back on the launch book, the copy removed.
    ("demo-left", |a| {
        live(a);
        leave(a);
        settle(a);
        settle(a);
    }),
    // Stack goal in the demo: the owner's goal (₿ 10 here) stays theirs; the demo measures
    // against a round target above its own stack.
    ("demo-goal", |a| {
        use crate::overview::grid::model::{Size, WidgetId};
        a.ov.layout.set_goal_sats(Some(10 * 100_000_000));
        a.ov.layout.add(WidgetId::Goal, Size::Medium);
        live(a);
    }),
    // The switcher inside the demo: Leave demo and Reset demo.
    ("demo-switcher-open", |a| {
        live(a);
        a.shell.popup = Some(crate::shell::Popup::Books);
        a.send("ui.profiles.snapshot", Value::Null, "app.profiles");
        settle(a);
    }),
    ("demo-transactions", |a| {
        live(a);
        a.navigate("/transactions");
        crate::transactions::prime(a, false);
    }),
    ("demo-switcher", |a| {
        a.shell.popup = Some(crate::shell::Popup::Books);
        a.send("ui.profiles.snapshot", Value::Null, "app.profiles");
        settle(a);
    }),
    ("demo-switcher-de", |a| {
        de(a);
        a.shell.popup = Some(crate::shell::Popup::Books);
        a.send("ui.profiles.snapshot", Value::Null, "app.profiles");
        settle(a);
    }),
    // Agents on inside the demo: the demo daemon writes the demo's own agent-access file.
    ("demo-agents-on", |a| {
        live(a);
        a.send(
            "ui.agent_access.configure",
            serde_json::json!({"mcp_enabled": true, "ai_features_enabled": true}),
            "demo.scene_agents",
        );
        settle(a);
        crate::settings::open_section(a, "assistant");
        crate::settings::prime(a);
        settle(a);
    }),
    ("demo-agents", |a| {
        live(a);
        crate::settings::open_section(a, "assistant");
        crate::settings::prime(a);
        settle(a);
    }),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reads_the_copy_and_its_preferences() {
        let out = br#"{"kind":"projects.demo","schema_version":1,"data":{"data_root":"/d/g/data","state_root":"/d/g","preference_environment":{"KASSIBER_OFFLINE_PREFERENCE_FILE":"/d/g/config/offline-mode.json","KASSIBER_AGENT_ACCESS_FILE":"/d/g/config/agent-access.json","OTHER":"x"}}}"#;
        let s = parse(out).unwrap();
        assert_eq!(s.data_root, PathBuf::from("/d/g/data"));
        assert_eq!(s.state_root, PathBuf::from("/d/g"));
        assert_eq!(s.env.len(), 2, "only Kassiber preference variables pass");
        let err = parse(br#"{"kind":"error","error":{"message":"The demo path holds files that are not Kassiber demo copies"}}"#).unwrap_err();
        assert!(err.contains("not Kassiber demo copies"));
        assert!(parse(b"garbage").is_err());
    }

    #[test]
    fn agent_commands_carry_the_demo_environment() {
        let env = vec![(
            "KASSIBER_AGENT_ACCESS_FILE".to_owned(),
            "/d/g/config/agent-access.json".to_owned(),
        )];
        let args: Vec<String> = ["--data-root", "/d/g/data", "mcp", "serve"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let cmd = claude_command(&["kassiber".into()], &args, &env);
        assert_eq!(
            cmd,
            "claude mcp add kassiber -e KASSIBER_AGENT_ACCESS_FILE=/d/g/config/agent-access.json -- kassiber --data-root /d/g/data mcp serve"
        );
        let json: Value = serde_json::from_str(&json_config(
            &["/A/Kassiber".into(), "--cli".into()],
            &args,
            &env,
        ))
        .unwrap();
        assert_eq!(json["mcpServers"]["kassiber"]["command"], "/A/Kassiber");
        assert_eq!(json["mcpServers"]["kassiber"]["args"][0], "--cli");
        assert_eq!(
            json["mcpServers"]["kassiber"]["env"]["KASSIBER_AGENT_ACCESS_FILE"],
            "/d/g/config/agent-access.json"
        );
    }

    #[test]
    fn only_marked_copies_are_removed() {
        let dir = std::env::temp_dir().join(format!("kassiber-demo-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("plain")).unwrap();
        remove_copy(&dir.join("plain"));
        assert!(dir.join("plain").is_dir());
        std::fs::create_dir_all(dir.join("copy/config")).unwrap();
        std::fs::write(dir.join("copy/config/demo.json"), "{}").unwrap();
        remove_copy(&dir.join("copy"));
        assert!(!dir.join("copy").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- Leaving the demo (review finding 1) ----

    use crate::app::Route;
    use crate::daemon::{Launch, Supervisor};

    #[cfg_attr(not(unix), allow(dead_code))] // used by the unix-only stub-daemon tests
    /// An app on a stub daemon (`daemon::tests::stub`) serving the user's own book "home-book".
    fn stub_app() -> App {
        let mut app = crate::app::tests::quiet_app();
        let stub = crate::daemon::tests::stub();
        app.daemon = Supervisor::new(Launch {
            data_root: Some("home-book".into()),
            explicit_root: true,
            bin_flag: Some(stub.clone()),
            binary: stub,
            env: Vec::new(),
        });
        app.launch = app.daemon.launch.clone();
        app.onboarding.seed = Some("home-book".into());
        app.onboarding.seed_explicit = true;
        app.onboarding.identity = Some(Identity {
            name: "Home".into(),
            ..Identity::default()
        });
        app.route = Route("/transactions".into());
        app
    }

    #[cfg_attr(not(unix), allow(dead_code))] // used by the unix-only stub-daemon tests
    /// A demo copy on disk (with its marker) and its preference files.
    fn copy(name: &str) -> Session {
        let root =
            std::env::temp_dir().join(format!("kassiber-demo-exit-{}-{name}", std::process::id()));
        std::fs::create_dir_all(root.join("config")).unwrap();
        std::fs::write(root.join("config/demo.json"), "{}").unwrap();
        let file = |f: &str| root.join("config").join(f).to_string_lossy().into_owned();
        Session {
            data_root: root.join("data"),
            env: vec![
                (
                    "KASSIBER_AGENT_ACCESS_FILE".into(),
                    file("agent-access.json"),
                ),
                (
                    "KASSIBER_OFFLINE_PREFERENCE_FILE".into(),
                    file("offline-mode.json"),
                ),
            ],
            state_root: root,
        }
    }

    #[cfg_attr(not(unix), allow(dead_code))] // used by the unix-only stub-daemon tests
    fn pid_of(app: &App, kind: &str) -> Option<i64> {
        app.read(kind).and_then(|d| d["pid"].as_i64())
    }

    #[cfg_attr(not(unix), allow(dead_code))] // used by the unix-only stub-daemon tests
    #[derive(Clone, Copy, Debug)]
    enum Exit {
        Leave,
        ResetWelcome,
        OpenOtherBooks,
        SelectProject,
    }

    #[cfg(unix)]
    #[test]
    fn every_way_out_leaves_the_demo_completely() {
        for exit in [
            Exit::Leave,
            Exit::ResetWelcome,
            Exit::OpenOtherBooks,
            Exit::SelectProject,
        ] {
            let mut app = stub_app();
            let session = copy(&format!("{exit:?}"));
            open(&mut app, session.clone());
            // Opening never blocks: the banner's spinner shows while the copy's daemon warms.
            assert!(app.demo.busy() && app.switching());
            app.settle(Duration::from_secs(20));
            assert_eq!(app.demo.active.as_ref(), Some(&session), "{exit:?}");
            assert_eq!(app.daemon.launch.env, session.env);
            assert_eq!(app.daemon.data_root(), Some(session.data_root.as_path()));
            assert_eq!(
                persisted_identity(&app).map(|i| i.name),
                Some("Home".into()),
                "the real book stays the remembered one"
            );
            assert_eq!(persisted_route(&app), "/transactions");
            let demo_pid = app.daemon.pid();

            match exit {
                Exit::Leave => leave(&mut app),
                Exit::ResetWelcome => crate::onboarding::reset_welcome(&mut app),
                Exit::OpenOtherBooks => crate::onboarding::reset_local_session(&mut app),
                Exit::SelectProject => crate::shell::books::select_project(&mut app, "other-set"),
            }
            // Something set up right away (Welcome's "Create new books") waits for the restored
            // daemon: it never reaches the copy.
            if matches!(exit, Exit::ResetWelcome | Exit::OpenOtherBooks) {
                assert!(crate::onboarding::shows(&app));
                app.send(
                    "ui.onboarding.complete",
                    serde_json::json!({"workspace": "Mine"}),
                    "onboarding.complete",
                );
            }
            app.settle(Duration::from_secs(20));

            assert!(
                app.demo.active.is_none() && app.demo.saved.is_none() && !app.demo.busy(),
                "{exit:?}"
            );
            assert!(
                app.daemon.launch.env.is_empty(),
                "{exit:?}: the user's own preference files"
            );
            assert!(app.launch.env.is_empty());
            assert_eq!(
                app.daemon.data_root(),
                Some(Path::new("home-book")),
                "{exit:?}"
            );
            assert!(!session.state_root.exists(), "{exit:?}: the copy is gone");
            assert_ne!(app.daemon.pid(), demo_pid);
            match exit {
                Exit::Leave => {
                    assert_eq!(
                        app.onboarding.identity.as_ref().map(|i| i.name.as_str()),
                        Some("Home")
                    );
                    assert_eq!(app.route.path(), "/transactions");
                }
                Exit::ResetWelcome | Exit::OpenOtherBooks => {
                    assert!(crate::onboarding::shows(&app));
                    let pid = pid_of(&app, "ui.onboarding.complete");
                    assert!(
                        pid.is_some() && pid.map(|p| p as u32) == app.daemon.pid(),
                        "{exit:?}: the new book went to the user's daemon"
                    );
                }
                Exit::SelectProject => {
                    // The book set was asked for once the user's daemon served (its answer then
                    // started a new session, which drops the cached reads).
                    assert!(app.timings.iter().any(|(k, _)| k == "ui.projects.select"));
                    assert!(app.pending.is_empty());
                }
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_reset_keeps_the_way_back_and_removes_the_old_copy() {
        let mut app = stub_app();
        let first = copy("reset-a");
        open(&mut app, first.clone());
        app.settle(Duration::from_secs(20));
        let second = copy("reset-b");
        open(&mut app, second.clone());
        app.settle(Duration::from_secs(20));
        assert_eq!(app.demo.active.as_ref(), Some(&second));
        assert!(!first.state_root.exists());
        assert_eq!(
            persisted_identity(&app).map(|i| i.name),
            Some("Home".into())
        );
        leave(&mut app);
        app.settle(Duration::from_secs(20));
        assert_eq!(app.daemon.data_root(), Some(Path::new("home-book")));
        assert!(!second.state_root.exists());
    }
}
