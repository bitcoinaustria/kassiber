//! "Open" and "Show in Finder / Explorer / folder" for a file an export just wrote: the text
//! actions on its confirmation toast (`App::toast_with`) and on inline "saved" cards.
//!
//! Owner request over the web (which only offers Open on the Reports status strip): every
//! save confirmation can open the file with the system's default app or reveal it in the file
//! manager. Rules:
//! - An action only targets the path the export flow itself just wrote: [`Saved::new`] records
//!   its canonical path then, and a press re-checks that the same file is still there (and, for
//!   files in the book's managed export folder, `reports::report::managed`). An unknown or
//!   missing path gives no actions rather than broken ones.
//! - "Open" is offered only for document types Kassiber exports (never a backup, an unknown
//!   extension or anything executable); reveal works for any saved file.
//! - Commands run without a shell, each path a separate argv item. They are local, not network
//!   egress (the Linux reveal asks the session bus's file manager, `org.freedesktop.FileManager1`).
//! - Headless tooling (scenes, scripts, the click-through, tests) never spawns anything: the
//!   intended argv is recorded ([`recorded`]) and printed to stderr instead.
//! - A command that fails (no default app, no file manager) shows an error toast.

use std::cell::Cell;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};

use rune::{El, Tx, row, text};

use crate::app::{App, ToastTone};
use crate::i18n::{Lang, t, tr};
use crate::persist::file::Os;
use crate::theme::Tk;

/// This module's own messages (Rune's; the web has none): English or German.
fn pick_in(lang: Lang, en: &str, de: &str) -> String {
    match lang {
        Lang::De => de.to_owned(),
        Lang::En => en.to_owned(),
    }
}

fn pick(en: &str, de: &str) -> String {
    pick_in(crate::i18n::lang(), en, de)
}

/// How long a toast with actions stays (a plain toast stays 6 s), and how long it stays once
/// the pointer or keyboard focus leaves it.
pub const ACTION_LIFE: f64 = 10.0;
pub const HOLD_AFTER: f64 = 4.0;

/// One action on a toast or a saved card: a text control with a visible label and an
/// accessible name.
#[derive(Clone)]
pub struct ToastAction {
    /// `open` / `reveal`: part of the element key.
    pub id: &'static str,
    pub label: String,
    /// The accessible name (names the file, so two toasts' actions stay distinct).
    pub name: String,
    pub run: Rc<dyn Fn(&mut App)>,
}

impl std::fmt::Debug for ToastAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToastAction")
            .field("id", &self.id)
            .field("label", &self.label)
            .field("name", &self.name)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Open,
    Reveal,
}

/// A file (or export bundle folder) an export just wrote.
#[derive(Clone, Debug, PartialEq)]
pub struct Saved {
    /// Canonical at the time of the write.
    pub path: PathBuf,
    pub dir: bool,
    /// Sits in the book's managed `exports/reports` folder (re-validated on press).
    pub managed: bool,
}

/// Extensions "Open" hands to the default app: the documents Kassiber's exports write.
const OPENABLE: &[&str] = &[
    "pdf", "xlsx", "csv", "tsv", "json", "jsonl", "md", "txt", "log", "zip",
];

impl Saved {
    /// The just-written `path`, or `None` when it is unknown, relative or gone.
    pub fn new(path: &str, managed: bool) -> Option<Saved> {
        let p = Path::new(path.trim());
        if path.trim().is_empty() || !p.is_absolute() {
            return None;
        }
        let path = std::fs::canonicalize(p).ok()?;
        let meta = path.metadata().ok()?;
        (meta.is_file() || meta.is_dir()).then(|| Saved {
            dir: meta.is_dir(),
            path,
            managed,
        })
    }

    pub fn name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// Open is for exported documents and bundle folders, never anything else.
    pub fn openable(&self) -> bool {
        self.dir
            || self
                .path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| OPENABLE.contains(&e.to_ascii_lowercase().as_str()))
    }

    /// Still the same file: present, same kind, same canonical path; a managed one still
    /// passes the managed-export checks.
    fn check(&self, app: &App) -> Result<PathBuf, String> {
        let now = if self.managed {
            crate::reports::report::managed(app, &self.path)?
        } else {
            std::fs::canonicalize(&self.path).map_err(|e| {
                format!(
                    "{} {e}",
                    pick(
                        "The saved file could not be found:",
                        "Die gespeicherte Datei wurde nicht gefunden:"
                    )
                )
            })?
        };
        if now != self.path || now.is_dir() != self.dir {
            return Err(pick(
                "The saved file changed since it was exported.",
                "Die gespeicherte Datei hat sich seit dem Export geändert.",
            ));
        }
        Ok(now)
    }
}

/// The reveal action's label: macOS Finder, Windows Explorer, a folder elsewhere.
pub fn reveal_label(os: Os) -> String {
    t(match os {
        Os::Macos => "chrome:notifications.action.showInFinder",
        Os::Windows => "chrome:notifications.action.showInExplorer",
        Os::Linux => "chrome:notifications.action.showInFolder",
    })
}

fn reveal_name(os: Os, file: &str) -> String {
    let key = match os {
        Os::Macos => "chrome:notifications.action.showInFinderLabel",
        Os::Windows => "chrome:notifications.action.showInExplorerLabel",
        Os::Linux => "chrome:notifications.action.showInFolderLabel",
    };
    tr(key, &[("name", file)])
}

/// Up to two actions for a just-saved file: Open (documents only) and the reveal.
pub fn actions(saved: Option<Saved>) -> Vec<ToastAction> {
    let Some(s) = saved else { return vec![] };
    let file = s.name();
    let os = Os::current();
    let mut out = vec![];
    if s.openable() {
        let target = s.clone();
        out.push(ToastAction {
            id: "open",
            label: t("common:actions.open"),
            name: tr(
                "chrome:notifications.action.openFileLabel",
                &[("name", &file)],
            ),
            run: Rc::new(move |a: &mut App| run(a, &target, Act::Open)),
        });
    }
    let target = s;
    out.push(ToastAction {
        id: "reveal",
        label: reveal_label(os),
        name: reveal_name(os, &file),
        run: Rc::new(move |a: &mut App| run(a, &target, Act::Reveal)),
    });
    out
}

/// [`actions`] for a path string: `managed` marks a file in the book's managed export folder.
pub fn for_path(path: &str, managed: bool) -> Vec<ToastAction> {
    actions(Saved::new(path, managed))
}

/// The success toast for a saved file, with its actions (none when the path is unknown).
pub fn saved_toast(
    app: &mut App,
    tone: ToastTone,
    title: &str,
    body: Option<String>,
    key: Option<&'static str>,
    path: &str,
    managed: bool,
) {
    let acts = for_path(path, managed);
    app.toast_with(tone, title, body, key, acts);
}

/// "File saved" + the file name, for exports the web confirms with nothing (logs, the support
/// bundle, a chat transcript).
pub fn file_saved(app: &mut App, path: &Path, key: &'static str) {
    let file = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    saved_toast(
        app,
        ToastTone::Success,
        &t("chrome:notifications.result.fileSaved.title"),
        Some(file).filter(|f| !f.is_empty()),
        Some(key),
        &path.to_string_lossy(),
        false,
    );
}

/// The Open of a managed report export (Reports' status strip, the exit-tax card,
/// `handleOpenExport`): through the managed-export checks, then the same runner.
pub fn open_managed(app: &mut App, path: &str) {
    match crate::reports::report::managed(app, Path::new(path)) {
        Ok(p) => {
            let dir = p.is_dir();
            run(
                app,
                &Saved {
                    path: p,
                    dir,
                    managed: true,
                },
                Act::Open,
            );
        }
        Err(e) => app.toast(
            ToastTone::Error,
            &t("chrome:notifications.feedback.reportOpenFailed"),
            Some(e),
            Some("report-open"),
        ),
    }
}

// ---- The commands -------------------------------------------------------------------------

/// One program and its argv (no shell). `check` = a non-zero exit means it failed (Windows
/// Explorer exits 1 even when it worked, so it is not checked there).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cmd {
    pub program: &'static str,
    pub args: Vec<OsString>,
    pub check: bool,
}

impl Cmd {
    fn new(program: &'static str, args: Vec<OsString>, check: bool) -> Cmd {
        Cmd {
            program,
            args,
            check,
        }
    }
    /// The argv as text, for the tooling record and tests.
    pub fn argv(&self) -> Vec<String> {
        std::iter::once(self.program.to_owned())
            .chain(self.args.iter().map(|a| a.to_string_lossy().into_owned()))
            .collect()
    }
}

/// The commands for `act` on `path`, the first first; each next one is the fallback when the
/// one before it failed.
pub fn commands(os: Os, act: Act, path: &Path, dir: bool) -> Vec<Cmd> {
    let p = || path.as_os_str().to_owned();
    match (os, act) {
        (Os::Macos, Act::Open) => vec![Cmd::new("open", vec![p()], true)],
        (Os::Macos, Act::Reveal) => vec![Cmd::new("open", vec!["-R".into(), p()], true)],
        (Os::Windows, Act::Open) => vec![Cmd::new("explorer", vec![p()], false)],
        (Os::Windows, Act::Reveal) => {
            vec![Cmd::new("explorer", vec!["/select,".into(), p()], false)]
        }
        (Os::Linux, Act::Open) => vec![Cmd::new("xdg-open", vec![p()], true)],
        (Os::Linux, Act::Reveal) => {
            let parent = if dir {
                path.parent().unwrap_or(path)
            } else {
                path.parent().unwrap_or(Path::new("/"))
            };
            vec![
                Cmd::new(
                    "dbus-send",
                    vec![
                        "--session".into(),
                        "--print-reply".into(),
                        "--reply-timeout=3000".into(),
                        "--dest=org.freedesktop.FileManager1".into(),
                        "--type=method_call".into(),
                        "/org/freedesktop/FileManager1".into(),
                        "org.freedesktop.FileManager1.ShowItems".into(),
                        format!("array:string:{}", file_uri(path)).into(),
                        "string:".into(),
                    ],
                    true,
                ),
                Cmd::new("xdg-open", vec![parent.as_os_str().to_owned()], true),
            ]
        }
    }
}

/// `file://` + the percent-encoded path. Commas are encoded too: dbus-send splits array items
/// on them.
pub fn file_uri(path: &Path) -> String {
    #[cfg(unix)]
    let bytes = {
        use std::os::unix::ffi::OsStrExt;
        path.as_os_str().as_bytes().to_vec()
    };
    #[cfg(not(unix))]
    let bytes = path.to_string_lossy().replace('\\', "/").into_bytes();
    let mut out = String::from("file://");
    for b in bytes {
        if b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

static RECORDED: Mutex<Vec<Vec<String>>> = Mutex::new(Vec::new());

/// The argv tooling runs would have spawned, oldest first.
#[allow(dead_code)] // tests and scenes
pub fn recorded() -> Vec<Vec<String>> {
    RECORDED.lock().map(|v| v.clone()).unwrap_or_default()
}

/// The outcome of a finished command chain: the error toast's title and message.
type Failure = (String, String);

thread_local! {
    static CHANNEL: (Sender<Failure>, Receiver<Failure>) = channel();
    static RUNNING: Cell<usize> = const { Cell::new(0) };
}

fn run(app: &mut App, saved: &Saved, act: Act) {
    let failed_title = match act {
        Act::Open if saved.managed => t("chrome:notifications.feedback.reportOpenFailed"),
        Act::Open => t("chrome:notifications.feedback.fileOpenFailed"),
        Act::Reveal => t("chrome:notifications.feedback.fileRevealFailed"),
    };
    if act == Act::Open && !saved.openable() {
        app.toast(
            ToastTone::Error,
            &failed_title,
            Some(pick(
                "This file type is not opened from Kassiber.",
                "Dieser Dateityp wird nicht aus Kassiber geöffnet.",
            )),
            Some("file-open"),
        );
        return;
    }
    let path = match saved.check(app) {
        Ok(p) => p,
        Err(e) => return app.toast(ToastTone::Error, &failed_title, Some(e), Some("file-open")),
    };
    let cmds = commands(Os::current(), act, &path, saved.dir);
    if crate::persist::file::tooling_run() {
        let argv = cmds[0].argv();
        if !cfg!(test) {
            eprintln!("toastopen: not spawned in a tooling run: {argv:?}");
        }
        if let Ok(mut v) = RECORDED.lock() {
            v.push(argv);
        }
        return;
    }
    let tx = CHANNEL.with(|(s, _)| s.clone());
    RUNNING.with(|n| n.set(n.get() + 1));
    // The UI language is per thread: the command thread is told which one to answer in.
    let lang = crate::i18n::lang();
    std::thread::spawn(move || {
        if let Err(message) = run_chain(&cmds, lang) {
            let _ = tx.send((failed_title, message));
        } else {
            let _ = tx.send((String::new(), String::new()));
        }
    });
}

/// Runs each command until one works; the last failure's message otherwise (in `lang`).
fn run_chain(cmds: &[Cmd], lang: Lang) -> Result<(), String> {
    let mut last = pick_in(
        lang,
        "No command could open it.",
        "Kein Befehl konnte sie öffnen.",
    );
    for c in cmds {
        let out = std::process::Command::new(c.program)
            .args(&c.args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .output();
        match out {
            Ok(o) if o.status.success() || !c.check => return Ok(()),
            Ok(o) => {
                let err = String::from_utf8_lossy(&o.stderr);
                let line = err
                    .lines()
                    .map(str::trim)
                    .find(|l| !l.is_empty())
                    .unwrap_or("")
                    .chars()
                    .take(200)
                    .collect::<String>();
                last = if line.is_empty() {
                    format!(
                        "{} {}{}",
                        c.program,
                        pick_in(lang, "exited with ", "endete mit "),
                        status_text(&o.status, lang)
                    )
                } else {
                    line
                };
            }
            Err(e) => {
                last = format!(
                    "{} {} {e}",
                    c.program,
                    pick_in(lang, "could not start:", "konnte nicht starten:")
                )
            }
        }
    }
    Err(last)
}

/// `exit status: 1` / `Exit-Status 1`, with the sentence's full stop.
fn status_text(status: &std::process::ExitStatus, lang: Lang) -> String {
    match status.code() {
        Some(c) => pick_in(lang, &format!("status {c}."), &format!("Status {c}.")),
        None => pick_in(lang, "a signal.", "einem Signal."),
    }
}

/// Every frame, before toasts expire: holds a toast the pointer or keyboard focus is on, and
/// turns a failed command into its error toast.
pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let mut changed = false;
    let focused = tx.focused().map(str::to_owned);
    let clock = app.clock;
    for toast in &mut app.toasts {
        let id = toast.id;
        let focus_in = focused
            .as_deref()
            .is_some_and(|k| k.starts_with("sh-toast") && k.ends_with(&format!("-{id}")));
        if (toast.hovered || focus_in) && toast.until < clock + HOLD_AFTER {
            toast.until = clock + HOLD_AFTER;
        }
    }
    let done: Vec<Failure> = CHANNEL.with(|(_, r)| r.try_iter().collect());
    for (title, message) in done {
        RUNNING.with(|n| n.set(n.get().saturating_sub(1)));
        if !title.is_empty() {
            app.toast(ToastTone::Error, &title, Some(message), Some("file-open"));
            changed = true;
        }
    }
    if RUNNING.with(|n| n.get()) > 0 {
        tx.wake_after(std::time::Duration::from_millis(200));
    }
    changed
}

// ---- The view -----------------------------------------------------------------------------

/// The element key of a toast's action (`…-{id}` so focus inside it holds the toast).
pub fn toast_key(action: &ToastAction, toast_id: u64) -> String {
    format!("sh-toast-{}-{toast_id}", action.id)
}

/// The actions as plain text controls on one line: `rest` colour, `hover` colour while the
/// pointer or keyboard focus is on one. No button chrome, no icons.
pub fn links(
    tk: &Tk,
    actions: &[ToastAction],
    key: impl Fn(&ToastAction) -> String,
    rest: rune::Color,
    hover: rune::Color,
) -> El {
    let mut r = row().gap(tk.r(0.75)).center_y();
    for a in actions.iter().take(2) {
        let run = a.run.clone();
        r = r.child(
            row()
                .key(key(a))
                .role("button")
                .label(a.name.clone())
                .focusable(true)
                .cursor(rune::Cursor::Pointer)
                .hover_group()
                .on_press(move |s: &mut App| run(s))
                .child(
                    text(a.label.clone())
                        .font(tk.txs(500.0))
                        .color(rest)
                        .group_color(hover),
                ),
        );
    }
    r
}

// ---- Scenes ---------------------------------------------------------------------------------

/// A file written for the scenes (in the temp folder; never opened: scenes are tooling runs).
fn scene_file() -> String {
    let dir = std::env::temp_dir().join("kassiber-toastopen-scene");
    let _ = std::fs::create_dir_all(&dir);
    let p = dir.join("kassiber-report-20261116-112725.pdf");
    if !p.exists() {
        let _ = std::fs::write(&p, b"%PDF-1.4\n%scene\n");
    }
    p.to_string_lossy().into_owned()
}

fn scene_saved(a: &mut App) {
    let path = scene_file();
    let title = t("chrome:notifications.result.reportExported.title");
    saved_toast(
        a,
        ToastTone::Success,
        &title,
        Some("kassiber-report-20261116-112725.pdf".into()),
        Some("report-export"),
        &path,
        false,
    );
}

pub const SCENES: &[crate::app::Scene] = &[
    ("toastopen-saved", scene_saved),
    ("toastopen-saved-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        crate::i18n::set_lang(crate::i18n::Lang::De);
        scene_saved(a);
    }),
    ("toastopen-saved-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        scene_saved(a);
    }),
    // The platform did not report where the file went: the toast has no actions.
    ("toastopen-no-path", |a| {
        let title = t("chrome:notifications.result.reportExported.title");
        saved_toast(
            a,
            ToastTone::Success,
            &title,
            Some("kassiber-report-20261116-112725.pdf".into()),
            Some("report-export"),
            "",
            false,
        );
    }),
    // A reveal pressed in a tooling run: nothing spawns; the toast stays.
    ("toastopen-pressed", |a| {
        scene_saved(a);
        if let Some(act) = a.toasts.last().and_then(|t| t.actions.last()).cloned() {
            (act.run)(a);
        }
    }),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn macos_commands() {
        let p = Path::new("/Users/me/Exports/a b,c.pdf");
        assert_eq!(
            commands(Os::Macos, Act::Open, p, false)
                .iter()
                .map(Cmd::argv)
                .collect::<Vec<_>>(),
            vec![s(&["open", "/Users/me/Exports/a b,c.pdf"])]
        );
        assert_eq!(
            commands(Os::Macos, Act::Reveal, p, false)
                .iter()
                .map(Cmd::argv)
                .collect::<Vec<_>>(),
            vec![s(&["open", "-R", "/Users/me/Exports/a b,c.pdf"])]
        );
        assert!(commands(Os::Macos, Act::Reveal, p, false)[0].check);
    }

    #[test]
    fn windows_commands_pass_the_path_alone() {
        let p = Path::new(r"C:\Users\me\My Exports\report.pdf");
        let open = commands(Os::Windows, Act::Open, p, false);
        assert_eq!(
            open[0].argv(),
            s(&["explorer", r"C:\Users\me\My Exports\report.pdf"])
        );
        let reveal = commands(Os::Windows, Act::Reveal, p, false);
        assert_eq!(reveal.len(), 1);
        assert_eq!(
            reveal[0].argv(),
            s(&["explorer", "/select,", r"C:\Users\me\My Exports\report.pdf"])
        );
        // Explorer's exit code means nothing.
        assert!(!reveal[0].check && !open[0].check);
    }

    #[test]
    fn linux_reveal_asks_the_file_manager_then_opens_the_folder() {
        let p = Path::new("/home/me/out/a b,c.pdf");
        assert_eq!(
            commands(Os::Linux, Act::Open, p, false)[0].argv(),
            s(&["xdg-open", "/home/me/out/a b,c.pdf"])
        );
        let r = commands(Os::Linux, Act::Reveal, p, false);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].program, "dbus-send");
        assert!(
            r[0].argv()
                .contains(&"array:string:file:///home/me/out/a%20b%2Cc.pdf".to_owned())
        );
        assert_eq!(r[0].argv().last().map(String::as_str), Some("string:"));
        assert_eq!(r[1].argv(), s(&["xdg-open", "/home/me/out"]));
        // A bundle folder opens its parent too.
        assert_eq!(
            commands(
                Os::Linux,
                Act::Reveal,
                Path::new("/home/me/out/kassiber-audit-package-2025"),
                true
            )[1]
            .argv(),
            s(&["xdg-open", "/home/me/out"])
        );
    }

    #[test]
    fn file_uris_encode_everything_but_unreserved() {
        assert_eq!(
            file_uri(Path::new("/tmp/ä #1.pdf")),
            "file:///tmp/%C3%A4%20%231.pdf"
        );
    }

    fn temp(name: &str, body: &[u8]) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("kassiber-toastopen-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn unknown_paths_get_no_actions() {
        assert!(for_path("", false).is_empty());
        assert!(for_path("relative/report.pdf", false).is_empty());
        assert!(for_path("/definitely/not/here/report.pdf", false).is_empty());
    }

    #[test]
    fn open_only_for_exported_documents() {
        let pdf = temp("r.pdf", b"x");
        let acts = for_path(&pdf.to_string_lossy(), false);
        assert_eq!(
            acts.iter().map(|a| a.id).collect::<Vec<_>>(),
            vec!["open", "reveal"]
        );
        let backup = temp("backup.kassiber", b"x");
        assert_eq!(
            for_path(&backup.to_string_lossy(), false)
                .iter()
                .map(|a| a.id)
                .collect::<Vec<_>>(),
            vec!["reveal"]
        );
        let script = temp("x.command", b"x");
        assert!(
            !Saved::new(&script.to_string_lossy(), false)
                .unwrap()
                .openable()
        );
    }

    #[test]
    fn tooling_runs_record_and_never_spawn() {
        let pdf = temp("tooling.pdf", b"x");
        let mut app = App::new();
        let acts = for_path(&pdf.to_string_lossy(), false);
        let before = recorded().len();
        for a in &acts {
            (a.run)(&mut app);
        }
        let rec = recorded();
        assert!(rec.len() >= before + 2);
        let canon = std::fs::canonicalize(&pdf)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(rec.iter().any(|v| v.last() == Some(&canon)
            && v[0] == commands(Os::current(), Act::Open, Path::new(&canon), false)[0].program));
        // Nothing ran, so nothing is pending and no error toast appeared.
        assert_eq!(RUNNING.with(|n| n.get()), 0);
        assert!(app.toasts.iter().all(|t| t.tone != ToastTone::Error));
    }

    #[test]
    fn a_replaced_file_is_refused() {
        let pdf = temp("gone.pdf", b"x");
        let mut app = App::new();
        let acts = for_path(&pdf.to_string_lossy(), false);
        std::fs::remove_file(&pdf).unwrap();
        (acts[0].run)(&mut app);
        // The record is shared with tests running in parallel: look for this file only.
        let name = pdf.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            !recorded()
                .iter()
                .any(|v| v.iter().any(|a| a.ends_with(&name)))
        );
        assert!(app.toasts.iter().any(|t| t.tone == ToastTone::Error));
    }

    #[test]
    fn errors_speak_german_too() {
        use crate::i18n::set_lang;
        let pdf = temp("changed-de.pdf", b"x");
        let saved = Saved::new(&pdf.to_string_lossy(), false).unwrap();
        std::fs::remove_file(&pdf).unwrap();
        let app = App::new();
        let script = temp("x-de.command", b"x");
        let mut app2 = App::new();
        set_lang(Lang::De);
        let gone = saved.check(&app).unwrap_err();
        run(
            &mut app2,
            &Saved::new(&script.to_string_lossy(), false).unwrap(),
            Act::Open,
        );
        set_lang(Lang::En);
        assert!(
            gone.starts_with("Die gespeicherte Datei wurde nicht gefunden:"),
            "{gone}"
        );
        assert!(app2.toasts.iter().any(
            |t| t.body.as_deref() == Some("Dieser Dateityp wird nicht aus Kassiber geöffnet.")
        ));
        // The command thread answers in the language it was given.
        let missing = Cmd::new("kassiber-no-such-opener", vec![], true);
        assert!(
            run_chain(std::slice::from_ref(&missing), Lang::De)
                .unwrap_err()
                .contains("konnte nicht starten:")
        );
        assert!(run_chain(&[], Lang::De).unwrap_err() == "Kein Befehl konnte sie öffnen.");
        assert!(run_chain(&[], Lang::En).unwrap_err() == "No command could open it.");
    }

    #[test]
    fn toasts_with_actions_live_longer_and_replace_their_file() {
        let a = temp("one.pdf", b"x");
        let b = temp("two.pdf", b"x");
        let mut app = App::new();
        saved_toast(
            &mut app,
            ToastTone::Success,
            "Report exported",
            Some("one.pdf".into()),
            Some("report-export"),
            &a.to_string_lossy(),
            false,
        );
        assert_eq!(app.toasts[0].actions.len(), 2);
        assert!(app.toasts[0].until >= app.clock + ACTION_LIFE);
        saved_toast(
            &mut app,
            ToastTone::Success,
            "Report exported",
            Some("two.pdf".into()),
            Some("report-export"),
            &b.to_string_lossy(),
            false,
        );
        assert_eq!(app.toasts.len(), 1);
        assert_eq!(app.toasts[0].body.as_deref(), Some("two.pdf"));
        assert!(app.toasts[0].actions[0].name.contains("two.pdf"));
    }
}
