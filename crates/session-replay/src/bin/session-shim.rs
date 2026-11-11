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
    keeps_env, keeps_secret_env, snapshot, Blobs, Exit, Root, Session, SignalEvent, StdinChunk,
    SESSION_FORMAT,
};

static CHILD: AtomicI32 = AtomicI32::new(0);
static LINES: AtomicU64 = AtomicU64::new(0);
static STDOUT_CLOSED_AFTER: AtomicU64 = AtomicU64::new(u64::MAX);
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

/// Everything a test wrote into an inherited descriptor, or `None` when it
/// is not open or does not reach its end within five seconds.
fn read_descriptor(fd: i32) -> Option<Vec<u8>> {
    use std::os::fd::FromRawFd;
    if fs::metadata(format!("/proc/self/fd/{fd}")).is_err() {
        return None;
    }
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        // SAFETY: the descriptor is open (checked above) and the test handed
        // it to this process to read; the shim reads it once.
        let mut file = unsafe { fs::File::from_raw_fd(fd) };
        let mut bytes = Vec::new();
        let _ = file.read_to_end(&mut bytes);
        let _ = sender.send(bytes);
    });
    receiver
        .recv_timeout(std::time::Duration::from_secs(5))
        .ok()
}

/// `shim-config.json` beside the shims (written by record-sessions.sh).
#[derive(Default, serde::Deserialize)]
struct Config {
    record_dir: Option<String>,
    #[serde(default)]
    root_prefixes: Vec<String>,
    repo: Option<String>,
    examples_dir: Option<String>,
    /// Where the recording's pytest plugin writes `<pid>`: the node id each
    /// worker is running.
    current_dir: Option<String>,
    /// Synthetic passphrases: one literal of the test sources per line.
    passphrase_allowlist: Option<String>,
    /// `libkassiber_clock_shim.so`, preloaded into each recorded process.
    clock_shim: Option<String>,
}

/// The test a pytest ancestor of this process is running.
fn current_test(dir: Option<&str>) -> Option<String> {
    let dir = Path::new(dir?);
    let mut pid = std::process::id();
    for _ in 0..12 {
        if let Ok(test) = fs::read_to_string(dir.join(pid.to_string())) {
            return Some(test.trim().to_owned());
        }
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        // `pid (comm) state ppid ...`; comm may contain spaces.
        let after = stat.rsplit_once(')')?.1;
        pid = after.split_whitespace().nth(1)?.parse().ok()?;
        if pid <= 1 {
            return None;
        }
    }
    None
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

/// The roots this run names. A path below one of the prefixes is captured
/// as itself when it is (or will be) a directory, with its siblings when it
/// is a data root named `data` (the product keeps `config`, `exports` and
/// `attachments` beside it), and as its directory when it is a file, so a
/// book's `-wal` and `-shm` come along. Only what the process names is
/// captured, never the whole test directory other books share.
fn roots(argv: &[String], env: &BTreeMap<String, String>, prefixes: &[PathBuf]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut consider = |text: &str| {
        for piece in text.split([
            '=', ':', ',', ' ', '"', '\'', '\n', '\t', '{', '}', '[', ']',
        ]) {
            let path = Path::new(piece.trim_end_matches(['/', '\\']));
            if !path.is_absolute()
                || path
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                continue;
            }
            let Some(prefix) = prefixes
                .iter()
                .find(|p| path.starts_with(p) && path != p.as_path())
            else {
                continue;
            };
            // pytest-xdist's `popen-gwN` directory alone is never a root.
            if path.parent() == Some(prefix.as_path())
                && path
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("popen-gw"))
            {
                continue;
            }
            let is_file = path.is_file() || (!path.exists() && path.extension().is_some());
            let data_root = path.file_name().is_some_and(|n| n == "data");
            let root = if is_file || data_root {
                path.parent().unwrap_or(path).to_path_buf()
            } else {
                path.to_path_buf()
            };
            let below_prefix = root.starts_with(prefix) && root != *prefix;
            let worker_dir = root.parent() == Some(prefix.as_path())
                && root
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("popen-gw"));
            if below_prefix && !worker_dir && !found.contains(&root) {
                found.push(root);
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
    // Tests often start children with a minimal environment, so the
    // recording settings live in a file beside the shims, not in variables.
    let config: Config = fs::read_to_string(map_path.with_file_name("shim-config.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    // An example a test built during the run is not in the map yet.
    let built = config
        .examples_dir
        .clone()
        .map(|dir| PathBuf::from(dir).join(&name));
    let fallback = built
        .filter(|p| p.is_file())
        .map(|p| p.to_string_lossy().into_owned());
    let Some(real) = map.get(&name).or(fallback.as_ref()) else {
        fail(&format!("{name} is not in {}", map_path.display()))
    };

    let record = config.record_dir.clone().map(PathBuf::from);
    let mut command = Command::new(real);
    command
        .args(&rest)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // A test-harness binary records its connections' bytes here.
    let tape = record.as_ref().map(|dir| {
        let tapes = dir.join("tapes");
        let _ = fs::create_dir_all(&tapes);
        tapes.join(format!("{}-{}.tape", now_us(), std::process::id()))
    });
    if let Some(tape) = &tape {
        command.env(
            "KASSIBER_TEST_NET_TAPE",
            format!("record:{}", tape.display()),
        );
    }

    let argv: Vec<String> = rest
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    // Passphrases and tokens the test hands over must be synthetic: literals
    // of the test sources.
    let allowlist = kassiber_session_replay::encrypted::Allowlist::load(Path::new(
        config
            .passphrase_allowlist
            .as_deref()
            .unwrap_or("/nonexistent"),
    ));
    let mut env: BTreeMap<String, String> = std::env::vars()
        .filter(|(k, v)| keeps_env(k) || keeps_secret_env(k, v, &allowlist))
        .collect();
    // A fresh SQLite seed per recorded process (kassiber_store::
    // seed_test_prng): its random row ids, and the hashes over them, replay
    // identically. Kept in the session's environment like any test switch.
    if record.is_some() {
        let mut bytes = [0u8; 4];
        let seed = fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut bytes))
            .map(|_| i32::from_le_bytes(bytes) & 0x3fff_ffff)
            .unwrap_or(0)
            .max(1);
        env.insert("KASSIBER_TEST_SQLITE_SEED".into(), seed.to_string());
        command.env("KASSIBER_TEST_SQLITE_SEED", seed.to_string());
    }
    // The session's wall clock (crates/session-replay/clock-shim): a short
    // process reads the same times, and the book the same hashes over them,
    // when it is replayed. It starts at the whole second the session began.
    if let (Some(_), Some(clock)) = (&record, config.clock_shim.as_deref()) {
        if Path::new(clock).is_file() {
            let base = now_us() / 1_000_000 * 1_000_000;
            env.insert("KASSIBER_TEST_CLOCK_BASE_US".into(), base.to_string());
            command.env("KASSIBER_TEST_CLOCK_BASE_US", base.to_string());
            let preload = match std::env::var("LD_PRELOAD") {
                Ok(existing) if !existing.is_empty() => format!("{clock}:{existing}"),
                _ => clock.to_owned(),
            };
            command.env("LD_PRELOAD", preload);
        }
    }
    let prefixes: Vec<PathBuf> = config.root_prefixes.iter().map(PathBuf::from).collect();
    let blobs = record.as_ref().map(|dir| Blobs::new(dir));
    // Inherited input descriptors (a test's passphrase pipe): read them,
    // keep their bytes, and give the child an identical pipe.
    let mut fds = BTreeMap::new();
    let mut handed = Vec::new();
    for fd in kassiber_session_replay::descriptor_args(&argv) {
        if let Some(bytes) = read_descriptor(fd) {
            if let Ok(pipe) = kassiber_session_replay::filled_pipe(&bytes) {
                handed.push((fd, pipe));
            }
            fds.insert(fd, base64::engine::general_purpose::STANDARD.encode(&bytes));
        }
    }
    kassiber_session_replay::hand_over(&mut command, handed);
    let state_roots = roots(&argv, &env, &prefixes);
    let before = blobs
        .as_ref()
        .map(|b| capture(&state_roots, b))
        .unwrap_or_default();
    // Services learn their book over stdin: a root first named there is
    // captured before the chunk naming it reaches the child.
    let known = Arc::new(Mutex::new((state_roots, before)));

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
    let (stdin_roots, stdin_blobs, stdin_prefixes) =
        (Arc::clone(&known), blobs.clone(), prefixes.clone());
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
                    if let Some(blobs) = &stdin_blobs {
                        let text = String::from_utf8_lossy(&buffer[..n]);
                        let named = roots(&[text.into_owned()], &BTreeMap::new(), &stdin_prefixes);
                        let mut known = stdin_roots.lock().unwrap();
                        for root in named {
                            if known.0.iter().any(|k| root.starts_with(k)) {
                                continue;
                            }
                            // A root named inside this one (the scratch
                            // directory in TMPDIR, say) gives way to it.
                            let (roots, captured) = &mut *known;
                            let mut index = 0;
                            while index < roots.len() {
                                if roots[index].starts_with(&root) {
                                    roots.remove(index);
                                    captured.remove(index);
                                } else {
                                    index += 1;
                                }
                            }
                            captured.extend(capture(std::slice::from_ref(&root), blobs));
                            roots.push(root);
                        }
                    }
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
            // The reader closed our stdout: close the child's too, so it
            // sees the closed pipe it would have seen without the shim.
            if stdout
                .write_all(&buffer[..n])
                .and_then(|_| stdout.flush())
                .is_err()
            {
                STDOUT_CLOSED_AFTER.store(all.len() as u64, Ordering::SeqCst);
                break;
            }
            all.extend_from_slice(&buffer[..n]);
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
        let (state_roots, before) = std::mem::take(&mut *known.lock().unwrap());
        let after = capture(&state_roots, &blobs);
        let mut roots: Vec<Root> = before
            .into_iter()
            .zip(after)
            .map(|((path, file, before), (_, file_after, after))| Root {
                path: path.to_string_lossy().into_owned(),
                file: file || file_after,
                before,
                after,
            })
            .collect();
        // Passphrases the test handed over: only synthetic ones (literals
        // of the test sources) may enter a recording; otherwise the session
        // is refused, never written. Encrypted books are kept decrypted.
        let stdin_bytes: Vec<u8> = stdin_log
            .lock()
            .unwrap()
            .iter()
            .filter_map(|c| c.data.as_ref())
            .flat_map(|d| {
                base64::engine::general_purpose::STANDARD
                    .decode(d)
                    .unwrap_or_default()
            })
            .collect();
        let fd_bytes: Vec<Vec<u8>> = fds
            .values()
            .map(|d| {
                base64::engine::general_purpose::STANDARD
                    .decode(d)
                    .unwrap_or_default()
            })
            .collect();
        let mut candidates =
            kassiber_session_replay::passphrase_candidates(&fd_bytes, &stdin_bytes);
        // A synthetic passphrase the test put in the environment opens the
        // books too (test switches such as `..._SECRET_STORE` are no
        // passphrases).
        for (name, value) in &env {
            if keeps_secret_env(name, value, &allowlist)
                && !keeps_env(name)
                && !candidates.contains(value)
            {
                candidates.push(value.clone());
            }
        }
        let refused: Vec<&String> = candidates.iter().filter(|p| !allowlist.allows(p)).collect();
        if !refused.is_empty() {
            let _ = fs::create_dir_all(dir.join("refused"));
            let _ = fs::write(
                dir.join("refused")
                    .join(format!("{started}-{}-{name}.txt", std::process::id())),
                format!(
                    "{} passphrase(s) not in the synthetic allowlist\n",
                    refused.len()
                ),
            );
            finish(signal, status.code());
        }
        let scratch = dir
            .join("decrypt-scratch")
            .join(std::process::id().to_string());
        for root in &mut roots {
            kassiber_session_replay::decrypt_books(&mut root.before, &candidates, &blobs, &scratch);
            kassiber_session_replay::decrypt_books(&mut root.after, &candidates, &blobs, &scratch);
        }
        let _ = fs::remove_dir_all(&scratch);
        let count = SIGNAL_COUNT.load(Ordering::SeqCst).min(MAX_SIGNALS);
        let signals = (0..count)
            .map(|i| SignalEvent {
                signal: SIGNALS[i].load(Ordering::SeqCst),
                after_lines: SIGNAL_LINES[i].load(Ordering::SeqCst),
            })
            .collect();
        let test = current_test(config.current_dir.as_deref())
            .or_else(|| std::env::var("PYTEST_CURRENT_TEST").ok())
            .map(|t| {
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
            repo: config.repo.clone().unwrap_or_default(),
            fds,
            rerecorded_us: Vec::new(),
            stdout_closed_after: match STDOUT_CLOSED_AFTER.load(Ordering::SeqCst) {
                u64::MAX => None,
                n => Some(n),
            },
            net_tape: tape.as_ref().and_then(|path| {
                let bytes = fs::read(path).ok()?;
                let _ = fs::remove_file(path);
                (!bytes.is_empty())
                    .then(|| blobs.put(&bytes).ok())
                    .flatten()
            }),
        };
        let sessions = dir.join("sessions");
        let _ = fs::create_dir_all(&sessions);
        let file = sessions.join(format!("{started}-{}-{name}.json", std::process::id()));
        if let Ok(text) = serde_json::to_string(&session) {
            let _ = fs::write(file, text);
        }
    }
    finish(signal, status.code());
}

/// Exit as the child did: with its status, or by its signal.
fn finish(signal: Option<i32>, code: Option<i32>) -> ! {
    if let Some(signal) = signal {
        // SAFETY: restore the default action and die the way the child did.
        unsafe {
            libc::signal(signal, libc::SIG_DFL);
            libc::raise(signal);
        }
    }
    std::process::exit(code.unwrap_or(1));
}
