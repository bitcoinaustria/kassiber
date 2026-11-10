//! Stands in for a product executable while a test suite records sessions.
//!
//! Install it as a symlink named after each executable, next to a
//! `shim-map.json` that maps the name to the real binary, and point the
//! test's `KASSIBER_*_BIN` variable at the symlink. With
//! `KASSIBER_SESSION_RECORD_DIR` set, every run writes one session there;
//! `KASSIBER_SESSION_ROOT_PREFIXES` (`:`-separated, absolute) names the
//! directories whose children are state roots: any argv or environment path
//! below one of them makes that child a root the session captures.
//!
//! The shim passes every byte through unchanged and forwards SIGTERM, SIGINT,
//! SIGHUP and SIGQUIT to the child; it exits with the child's status, or dies
//! by the child's signal.

#![allow(unsafe_code)] // signal forwarding needs sigaction and kill

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicI32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use kassiber_session_replay::{
    keeps_env, snapshot, Blobs, Exit, Root, Session, SignalEvent, StdinChunk, SESSION_FORMAT,
};

static CHILD: AtomicI32 = AtomicI32::new(0);
static LINES: AtomicU64 = AtomicU64::new(0);
const MAX_SIGNALS: usize = 32;
static SIGNAL_COUNT: AtomicUsize = AtomicUsize::new(0);
static SIGNALS: [AtomicI32; MAX_SIGNALS] = [const { AtomicI32::new(0) }; MAX_SIGNALS];
static SIGNAL_LINES: [AtomicU64; MAX_SIGNALS] = [const { AtomicU64::new(0) }; MAX_SIGNALS];

extern "C" fn forward(signal: libc::c_int) {
    let child = CHILD.load(Ordering::SeqCst);
    let index = SIGNAL_COUNT.fetch_add(1, Ordering::SeqCst);
    if index < MAX_SIGNALS {
        SIGNALS[index].store(signal, Ordering::SeqCst);
        SIGNAL_LINES[index].store(LINES.load(Ordering::SeqCst), Ordering::SeqCst);
    }
    if child > 0 {
        // SAFETY: kill is async-signal-safe.
        unsafe {
            libc::kill(child, signal);
        }
    }
}

fn install_forwarding() {
    for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP, libc::SIGQUIT] {
        // SAFETY: a plain handler that only touches atomics and calls kill.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = forward as *const () as usize;
            libc::sigemptyset(&mut action.sa_mask);
            action.sa_flags = libc::SA_RESTART;
            libc::sigaction(signal, &action, std::ptr::null_mut());
        }
    }
}

fn now_us() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as i64)
        .unwrap_or(0)
}

fn fail(message: &str) -> ! {
    eprintln!("session-shim: {message}");
    std::process::exit(127)
}

/// The roots this run names: for every absolute path in argv or the kept
/// environment below a prefix, the prefix's child on that path.
fn roots(argv: &[String], env: &BTreeMap<String, String>, prefixes: &[PathBuf]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut consider = |text: &str| {
        for piece in text.split(['=', ':', ',', ' ', '"', '\'']) {
            let path = Path::new(piece);
            if !path.is_absolute() {
                continue;
            }
            for prefix in prefixes {
                if let Ok(rest) = path.strip_prefix(prefix) {
                    // pytest-xdist puts each worker's tests under `popen-gwN`.
                    let mut parts = rest.components();
                    let mut root = prefix.clone();
                    let mut next = parts.next();
                    while let Some(part) = next {
                        root.push(part);
                        if !part.as_os_str().to_string_lossy().starts_with("popen-gw") {
                            break;
                        }
                        next = parts.next();
                    }
                    if root != *prefix && next.is_some() && !found.contains(&root) {
                        found.push(root);
                    }
                }
            }
        }
    };
    argv.iter().for_each(|a| consider(a));
    env.values().for_each(|v| consider(v));
    // A root inside another root is captured by the outer one.
    let all = found.clone();
    found.retain(|root| {
        !all.iter()
            .any(|other| other != root && root.starts_with(other))
    });
    found.sort();
    found
}

fn capture(
    roots: &[PathBuf],
    blobs: &Blobs,
) -> Vec<(PathBuf, bool, Vec<kassiber_session_replay::Entry>)> {
    roots
        .iter()
        .map(|root| {
            let file = root.is_file();
            let entries = if root.exists() {
                snapshot(root, blobs).unwrap_or_default()
            } else {
                Vec::new()
            };
            (root.clone(), file, entries)
        })
        .collect()
}

fn main() {
    let mut args = std::env::args_os();
    let argv0 = PathBuf::from(args.next().unwrap_or_default());
    let rest: Vec<OsString> = args.collect();
    let name = argv0
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_owned();
    let map_path = argv0
        .parent()
        .unwrap_or(Path::new("."))
        .join("shim-map.json");
    let map: BTreeMap<String, String> = fs::read_to_string(&map_path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| fail(&format!("no shim map at {}", map_path.display())));
    let Some(real) = map.get(&name) else {
        fail(&format!("{name} is not in {}", map_path.display()))
    };

    let record = std::env::var_os("KASSIBER_SESSION_RECORD_DIR").map(PathBuf::from);
    let mut command = Command::new(real);
    command
        .args(&rest)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // The child must not record its own children through another shim.
    command.env_remove("KASSIBER_SESSION_RECORD_DIR");

    let argv: Vec<String> = rest
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let env: BTreeMap<String, String> = std::env::vars().filter(|(k, _)| keeps_env(k)).collect();
    let prefixes: Vec<PathBuf> = std::env::var("KASSIBER_SESSION_ROOT_PREFIXES")
        .unwrap_or_default()
        .split(':')
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .collect();
    let blobs = record.as_ref().map(|dir| Blobs::new(dir));
    let state_roots = roots(&argv, &env, &prefixes);
    let before = blobs
        .as_ref()
        .map(|b| capture(&state_roots, b))
        .unwrap_or_default();

    install_forwarding();
    let started = now_us();
    let mut child = command
        .spawn()
        .unwrap_or_else(|e| fail(&format!("could not start {real}: {e}")));
    CHILD.store(child.id() as i32, Ordering::SeqCst);

    let stdin_log: Arc<Mutex<Vec<StdinChunk>>> = Arc::default();
    let mut child_stdin = child.stdin.take().unwrap();
    let log = Arc::clone(&stdin_log);
    let recording = record.is_some();
    // Not joined: a parent that never closes our stdin must not hold us up.
    std::thread::spawn(move || {
        let mut input = std::io::stdin().lock();
        let mut buffer = vec![0u8; 65536];
        loop {
            match input.read(&mut buffer) {
                Ok(0) | Err(_) => {
                    if recording {
                        log.lock().unwrap().push(StdinChunk {
                            after_lines: LINES.load(Ordering::SeqCst),
                            data: None,
                        });
                    }
                    break;
                }
                Ok(n) => {
                    if recording {
                        let data = base64::engine::general_purpose::STANDARD.encode(&buffer[..n]);
                        log.lock().unwrap().push(StdinChunk {
                            after_lines: LINES.load(Ordering::SeqCst),
                            data: Some(data),
                        });
                    }
                    if child_stdin
                        .write_all(&buffer[..n])
                        .and_then(|_| child_stdin.flush())
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
    });
    let mut child_stdout = child.stdout.take().unwrap();
    let out = std::thread::spawn(move || {
        let mut all = Vec::new();
        let mut buffer = vec![0u8; 65536];
        let mut stdout = std::io::stdout();
        while let Ok(n) = child_stdout.read(&mut buffer) {
            if n == 0 {
                break;
            }
            all.extend_from_slice(&buffer[..n]);
            let _ = stdout.write_all(&buffer[..n]).and_then(|_| stdout.flush());
            let lines = buffer[..n].iter().filter(|b| **b == b'\n').count() as u64;
            LINES.fetch_add(lines, Ordering::SeqCst);
        }
        all
    });
    let mut child_stderr = child.stderr.take().unwrap();
    let err = std::thread::spawn(move || {
        let mut all = Vec::new();
        let mut buffer = vec![0u8; 65536];
        let mut stderr = std::io::stderr();
        while let Ok(n) = child_stderr.read(&mut buffer) {
            if n == 0 {
                break;
            }
            all.extend_from_slice(&buffer[..n]);
            let _ = stderr.write_all(&buffer[..n]).and_then(|_| stderr.flush());
        }
        all
    });
    let status = child.wait().unwrap_or_else(|e| fail(&format!("wait: {e}")));
    let stdout = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();
    let ended = now_us();
    #[cfg(unix)]
    let signal = std::os::unix::process::ExitStatusExt::signal(&status);
    #[cfg(not(unix))]
    let signal: Option<i32> = None;

    if let (Some(dir), Some(blobs)) = (record, blobs) {
        let after = capture(&state_roots, &blobs);
        let roots = before
            .into_iter()
            .zip(after)
            .map(|((path, file, before), (_, file_after, after))| Root {
                path: path.to_string_lossy().into_owned(),
                file: file || file_after,
                before,
                after,
            })
            .collect();
        let count = SIGNAL_COUNT.load(Ordering::SeqCst).min(MAX_SIGNALS);
        let signals = (0..count)
            .map(|i| SignalEvent {
                signal: SIGNALS[i].load(Ordering::SeqCst),
                after_lines: SIGNAL_LINES[i].load(Ordering::SeqCst),
            })
            .collect();
        let test = std::env::var("PYTEST_CURRENT_TEST").ok().map(|t| {
            t.strip_suffix(" (call)")
                .or_else(|| t.strip_suffix(" (setup)"))
                .or_else(|| t.strip_suffix(" (teardown)"))
                .unwrap_or(&t)
                .to_owned()
        });
        let session = Session {
            format: SESSION_FORMAT.into(),
            binary: name.clone(),
            argv,
            cwd: std::env::current_dir()
                .map(|d| d.to_string_lossy().into_owned())
                .unwrap_or_default(),
            env,
            test,
            started_us: started,
            ended_us: ended,
            stdin: stdin_log.lock().unwrap().clone(),
            stdout: blobs.put(&stdout).unwrap_or_default(),
            stderr: blobs.put(&stderr).unwrap_or_default(),
            signals,
            exit: Exit {
                code: status.code(),
                signal,
            },
            roots,
            repo: std::env::var("KASSIBER_SESSION_REPO").unwrap_or_default(),
        };
        let sessions = dir.join("sessions");
        let _ = fs::create_dir_all(&sessions);
        let file = sessions.join(format!("{started}-{}-{name}.json", std::process::id()));
        if let Ok(text) = serde_json::to_string(&session) {
            let _ = fs::write(file, text);
        }
    }
    if let Some(signal) = signal {
        // SAFETY: restore the default action and die the way the child did.
        unsafe {
            libc::signal(signal, libc::SIG_DFL);
            libc::raise(signal);
        }
    }
    std::process::exit(status.code().unwrap_or(1));
}
