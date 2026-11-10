//! Bounded, process-local computation receipts and staged local sources.
//!
//! Workers never receive the caller's SQLite connection. Entropy and dataset
//! preview workers are pure computations over submitted inputs; staged sources
//! are local files selected by a trusted picker. Nothing here persists bytes,
//! contacts a provider or grants accounting authority. A book switch, lock or
//! restore clears every receipt and grant and cancels running workers.
use super::Text;
use kassiber_envelope::{dict, s, AppError};
use kassiber_pyjson::{dumps, Options, Value};
use kassiber_store::rusqlite::Connection;
use std::collections::BTreeMap;
use std::fs::{File, Metadata};
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

type Result<T> = std::result::Result<T, AppError>;

const JOB_CAPACITY: usize = 16;
const JOB_CONCURRENCY: usize = 2;
const JOB_TTL: Duration = Duration::from_secs(3600);
const SOURCE_TTL: Duration = Duration::from_secs(1800);
const SOURCE_CAPACITY: usize = 32;
pub const PSBT_LIMIT: u64 = 4 * 1024 * 1024;
pub const DATASET_LIMIT: u64 = 8 * 1024 * 1024 * 1024;

fn limit(purpose: &str) -> Option<u64> {
    match purpose {
        "psbt" => Some(PSBT_LIMIT),
        "dataset" => Some(DATASET_LIMIT),
        _ => None,
    }
}

/// `scope_key`: the main database file and profile. Receipts and grants never
/// cross books, even when two books share a profile identifier.
pub fn scope_key(conn: &Connection, profile: &str) -> Vec<u8> {
    let identity = match conn.path().filter(|p| !p.is_empty()) {
        Some(path) => path.to_owned(),
        None => format!("memory-connection:{:p}", conn),
    };
    let mut key = identity.into_bytes();
    key.push(0);
    key.extend_from_slice(profile.as_bytes());
    key
}

/// `secrets.token_urlsafe(32)` from SQLite's OS-seeded generator.
fn token(conn: &Connection) -> Result<String> {
    let raw: Vec<u8> = conn
        .query_row("SELECT randomblob(32)", [], |r| r.get(0))
        .map_err(crate::database_error)?;
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(43);
    for chunk in raw.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..=chunk.len() {
            out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    Ok(out)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// The compute side of a receipt: progress updates and cancellation.
pub struct Worker<'a> {
    progress: &'a dyn Fn(Value),
    cancel: &'a AtomicBool,
}
impl Worker<'_> {
    pub fn progress(&self, value: Value) {
        (self.progress)(value)
    }
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }
}

pub type Compute = Box<dyn FnOnce(&Worker<'_>) -> Result<Value> + Send>;

struct Job {
    scope: Vec<u8>,
    request: Value,
    created: Instant,
    cancel: Arc<AtomicBool>,
    status: &'static str,
    progress: Value,
    result: Value,
    error_code: Value,
    finished: Option<Instant>,
}

#[derive(Default)]
struct JobState {
    jobs: BTreeMap<String, Job>,
    active: BTreeMap<String, Arc<AtomicBool>>,
    running: usize,
    quiescing: usize,
}

#[derive(Default)]
struct Jobs {
    state: Mutex<JobState>,
    idle: Condvar,
}

fn busy(message: &str) -> AppError {
    AppError::new("chain_analysis_busy", message).retryable(true)
}

impl Jobs {
    fn prune(state: &mut JobState) {
        let now = Instant::now();
        state.jobs.retain(|_, job| {
            if now.duration_since(job.created) < JOB_TTL {
                return true;
            }
            job.cancel.store(true, Ordering::SeqCst);
            job.status == "running"
        });
    }

    fn receipt(ident: &str, job: &Job) -> Value {
        let end = job.finished.unwrap_or_else(Instant::now);
        dict([
            ("schema_version", Value::Int(1.into())),
            ("job_id", s(ident)),
            ("status", s(job.status)),
            (
                "cancel_requested",
                Value::Bool(job.cancel.load(Ordering::SeqCst)),
            ),
            ("request", job.request.clone()),
            ("progress", job.progress.clone()),
            ("result", job.result.clone()),
            ("error_code", job.error_code.clone()),
            (
                "elapsed_ms",
                Value::Int((end.duration_since(job.created).as_millis() as u64).into()),
            ),
            ("receipt_scope", s("submitted_snapshot")),
            ("local_only", Value::Bool(true)),
        ])
    }
}

/// Grants are pinned to one regular file identity and one book.
#[derive(Clone)]
struct Grant {
    staged: Instant,
    order: u64,
    scope: Vec<u8>,
    path: PathBuf,
    purpose: String,
    identity: Identity,
}

#[derive(Clone, PartialEq, Eq)]
struct Identity(u64, u64, u64, i64, i64, i64, i64);

fn identity(info: &Metadata) -> Identity {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Identity(
            info.dev(),
            info.ino(),
            info.size(),
            info.mtime(),
            info.mtime_nsec(),
            info.ctime(),
            info.ctime_nsec(),
        )
    }
    #[cfg(not(unix))]
    {
        let modified = info
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .unwrap_or_default();
        Identity(
            0,
            0,
            info.len(),
            modified.as_secs() as i64,
            i64::from(modified.subsec_nanos()),
            0,
            0,
        )
    }
}

/// Book scope, grant token and the canonical preview recipe.
type PreviewKey = (Vec<u8>, String, String);

#[derive(Default)]
struct SourceState {
    grants: BTreeMap<String, Grant>,
    previews: BTreeMap<PreviewKey, (Instant, u64, Value)>,
    counter: u64,
}

/// The daemon's analysis runtime. CLI calls carry none.
#[derive(Default)]
pub struct Runtime {
    jobs: Arc<Jobs>,
    sources: Mutex<SourceState>,
}

/// A checked staged source. Every read re-verifies the pinned file identity
/// before its bytes reach a parser.
pub struct Source {
    file: BufReader<File>,
    identity: Identity,
}
fn changed() -> AppError {
    AppError::new(
        "chain_analysis_source_changed",
        "Selected analysis source changed; select and preview it again",
    )
}
impl Source {
    fn verify(&self) -> Result<()> {
        let info = self.file.get_ref().metadata().map_err(|_| changed())?;
        if !info.is_file() || identity(&info) != self.identity {
            return Err(changed());
        }
        Ok(())
    }
    /// `read(size)`: up to `size` bytes, verified after the read.
    pub fn read_up_to(&mut self, size: u64) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        let read = (&mut self.file).take(size).read_to_end(&mut out);
        self.verify()?;
        read.map_err(|_| changed())?;
        Ok(out)
    }
    /// `readline(size)`: one `\n`-terminated record of at most `size` bytes.
    pub fn read_line(&mut self, size: usize) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        let read = (|| -> std::io::Result<()> {
            while out.len() < size {
                let buffer = self.file.fill_buf()?;
                if buffer.is_empty() {
                    break;
                }
                let want = (size - out.len()).min(buffer.len());
                let end = buffer[..want]
                    .iter()
                    .position(|b| *b == b'\n')
                    .map_or(want, |i| i + 1);
                out.extend_from_slice(&buffer[..end]);
                self.file.consume(end);
                if out.last() == Some(&b'\n') {
                    break;
                }
            }
            Ok(())
        })();
        self.verify()?;
        read.map_err(|_| changed())?;
        Ok(out)
    }
}

/// Holds admission closed until a restore has finished replacing the book.
pub struct Quiesced(Arc<Jobs>);
impl Drop for Quiesced {
    fn drop(&mut self) {
        lock(&self.0.state).quiescing -= 1;
    }
}

impl Runtime {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// `clear_runtime`: cancel every worker and forget receipts and grants.
    pub fn clear(&self) {
        let mut state = lock(&self.jobs.state);
        for flag in state.active.values() {
            flag.store(true, Ordering::SeqCst);
        }
        for job in state.jobs.values() {
            job.cancel.store(true, Ordering::SeqCst);
        }
        state.jobs.clear();
        drop(state);
        let mut sources = lock(&self.sources);
        sources.grants.clear();
        sources.previews.clear();
    }

    /// `JOBS.quiesce`: close admission and wait for workers to exit.
    pub fn quiesce(&self, timeout: Duration) -> Result<Quiesced> {
        let mut state = lock(&self.jobs.state);
        state.quiescing += 1;
        let guard = Quiesced(Arc::clone(&self.jobs));
        for flag in state.active.values() {
            flag.store(true, Ordering::SeqCst);
        }
        let deadline = Instant::now() + timeout;
        while state.running > 0 {
            let now = Instant::now();
            if now >= deadline {
                drop(state);
                drop(guard);
                return Err(AppError::new(
                    "project_in_use",
                    "Wait for analysis workers to finish before restoring",
                )
                .retryable(true));
            }
            state = self
                .jobs
                .idle
                .wait_timeout(state, deadline - now)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        Ok(guard)
    }

    /// `JOBS.start`: admit one bounded worker and return its first receipt.
    pub fn start(
        &self,
        conn: &Connection,
        scope: Vec<u8>,
        request: Value,
        compute: Compute,
    ) -> Result<Value> {
        let ident = token(conn)?;
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut state = lock(&self.jobs.state);
            Jobs::prune(&mut state);
            if state.quiescing > 0 || state.running >= JOB_CONCURRENCY {
                return Err(busy(
                    "Analysis workers are busy; finish or cancel a running computation",
                ));
            }
            while state.jobs.len() >= JOB_CAPACITY {
                let oldest = state
                    .jobs
                    .iter()
                    .filter(|(_, job)| job.status != "running")
                    .min_by(|a, b| (a.1.created, a.0).cmp(&(b.1.created, b.0)))
                    .map(|(k, _)| k.clone())
                    .ok_or_else(|| busy("Analysis capacity exhausted"))?;
                state.jobs.remove(&oldest);
            }
            state.jobs.insert(
                ident.clone(),
                Job {
                    scope: scope.clone(),
                    request,
                    created: Instant::now(),
                    cancel: Arc::clone(&cancel),
                    status: "running",
                    progress: dict([
                        ("phase", s("queued")),
                        ("states_explored", Value::Int(0.into())),
                    ]),
                    result: Value::Null,
                    error_code: Value::Null,
                    finished: None,
                },
            );
            state.active.insert(ident.clone(), Arc::clone(&cancel));
            state.running += 1;
        }
        let jobs = Arc::clone(&self.jobs);
        let id = ident.clone();
        let spawned = std::thread::Builder::new()
            .name("chain-analysis".into())
            .spawn(move || {
                let update = |value: Value| {
                    if let Some(job) = lock(&jobs.state).jobs.get_mut(&id) {
                        job.progress = value;
                    }
                };
                let worker = Worker {
                    progress: &update,
                    cancel: &cancel,
                };
                let outcome =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| compute(&worker)));
                let mut state = lock(&jobs.state);
                let stopped = cancel.load(Ordering::SeqCst);
                if let Some(job) = state.jobs.get_mut(&id) {
                    match outcome {
                        Ok(Ok(result)) => {
                            // A committed import receipt wins over a late cancel.
                            job.status = if kassiber_envelope::get(&result, "status")
                                == Some(&s("cancelled"))
                            {
                                "cancelled"
                            } else {
                                "completed"
                            };
                            job.result = result;
                        }
                        Ok(Err(error)) => {
                            job.error_code = if stopped { Value::Null } else { s(error.code) };
                            job.status = if stopped { "cancelled" } else { "failed" };
                        }
                        Err(_) => {
                            job.error_code = if stopped {
                                Value::Null
                            } else {
                                s("chain_analysis_computation_failed")
                            };
                            job.status = if stopped { "cancelled" } else { "failed" };
                        }
                    }
                    job.finished = Some(Instant::now());
                }
                state.running -= 1;
                state.active.remove(&id);
                jobs.idle.notify_all();
            });
        if spawned.is_err() {
            let mut state = lock(&self.jobs.state);
            state.running -= 1;
            state.active.remove(&ident);
            state.jobs.remove(&ident);
            return Err(busy(
                "Analysis workers are busy; finish or cancel a running computation",
            ));
        }
        self.get(&scope, &s(ident), false)
    }

    /// `JOBS.get`: a receipt for this book only; `cancel` requests a stop.
    pub fn get(&self, scope: &[u8], ident: &Value, cancel: bool) -> Result<Value> {
        let mut state = lock(&self.jobs.state);
        Jobs::prune(&mut state);
        let found = match ident {
            Value::Str(v) => v.as_str().and_then(|v| state.jobs.get_key_value(v)),
            _ => None,
        };
        let Some((ident, job)) = found.filter(|(_, job)| job.scope == scope) else {
            return Err(AppError::new(
                "chain_analysis_job_unavailable",
                "Analysis computation expired or belongs to another book",
            )
            .retryable(false));
        };
        if cancel && job.status == "running" {
            job.cancel.store(true, Ordering::SeqCst);
        }
        Ok(Jobs::receipt(ident, job))
    }

    /// `SOURCES.stage`: pin a picker-selected regular file to this book.
    pub fn stage(
        &self,
        conn: &Connection,
        scope: Vec<u8>,
        path: &Value,
        purpose: &Value,
    ) -> Result<Value> {
        let Some(purpose) = purpose.as_str().filter(|p| limit(p).is_some()) else {
            return Err(AppError::new(
                "validation",
                "Unsupported analysis source purpose",
            ));
        };
        let unavailable = || {
            AppError::new(
                "chain_analysis_source_unavailable",
                "Selected analysis source is unavailable or exceeds its size limit",
            )
        };
        let raw = path
            .as_str()
            .filter(|p| !p.contains('\0'))
            .ok_or_else(unavailable)?;
        let home = std::env::var("HOME").ok();
        let expanded = expand_user(raw, home.as_deref());
        let source = std::fs::canonicalize(expanded).map_err(|_| unavailable())?;
        let info = std::fs::metadata(&source).map_err(|_| unavailable())?;
        if !info.is_file() || info.len() == 0 || info.len() > limit(purpose).unwrap() {
            return Err(unavailable());
        }
        let token = token(conn)?;
        let mut sources = lock(&self.sources);
        let now = Instant::now();
        sources
            .grants
            .retain(|_, g| now.duration_since(g.staged) < SOURCE_TTL);
        while sources.grants.len() >= SOURCE_CAPACITY {
            let oldest = sources
                .grants
                .iter()
                .min_by_key(|(_, g)| (g.staged, g.order))
                .map(|(k, _)| k.clone())
                .unwrap();
            sources.grants.remove(&oldest);
        }
        sources.counter += 1;
        let order = sources.counter;
        let filename = source
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        sources.grants.insert(
            token.clone(),
            Grant {
                staged: now,
                order,
                scope,
                path: source,
                purpose: purpose.to_owned(),
                identity: identity(&info),
            },
        );
        Ok(dict([
            ("source_token", s(token)),
            ("purpose", s(purpose)),
            ("size_bytes", Value::Int(info.len().into())),
            ("filename", s(filename)),
        ]))
    }

    /// `SOURCES.open`: a checked reader for an unexpired grant of this book.
    pub fn open(&self, scope: &[u8], token: &Value, purpose: &str) -> Result<Source> {
        let grant = {
            let sources = lock(&self.sources);
            match token {
                Value::Str(v) => v.as_str().and_then(|v| sources.grants.get(v)).cloned(),
                _ => None,
            }
        }
        .filter(|g| g.scope == scope && g.purpose == purpose && g.staged.elapsed() < SOURCE_TTL)
        .ok_or_else(|| {
            AppError::new(
                "chain_analysis_source_expired",
                "Select the analysis source again in its original book",
            )
        })?;
        let file = open_nofollow(&grant.path).map_err(|_| {
            AppError::new(
                "chain_analysis_source_unavailable",
                "Selected analysis source is unavailable",
            )
        })?;
        let source = Source {
            file: BufReader::with_capacity(64 * 1024, file),
            identity: grant.identity,
        };
        source.verify()?;
        Ok(source)
    }

    fn preview_key(scope: &[u8], token: &Value, recipe: &Value) -> Option<PreviewKey> {
        let token = token.as_str()?.to_owned();
        let recipe = dumps(recipe, &Options::new().sort_keys(true).compact()).ok()?;
        Some((scope.to_vec(), token, recipe))
    }

    /// `SOURCES.remember_preview`: reuse a completed validation for consent.
    pub fn remember_preview(&self, scope: &[u8], token: &Value, recipe: &Value, result: &Value) {
        let Some(key) = Self::preview_key(scope, token, recipe) else {
            return;
        };
        let mut sources = lock(&self.sources);
        sources.counter += 1;
        let order = sources.counter;
        sources
            .previews
            .insert(key, (Instant::now(), order, result.clone()));
        while sources.previews.len() > SOURCE_CAPACITY * 2 {
            let oldest = sources
                .previews
                .iter()
                .min_by_key(|(_, v)| (v.0, v.1))
                .map(|(k, _)| k.clone())
                .unwrap();
            sources.previews.remove(&oldest);
        }
    }

    /// `SOURCES.preview`: the remembered result for these exact inputs.
    pub fn preview(
        &self,
        scope: &[u8],
        token: &Value,
        recipe: &Value,
        expected_sha256: &Value,
    ) -> Result<Value> {
        drop(self.open(scope, token, "dataset")?);
        let sources = lock(&self.sources);
        Self::preview_key(scope, token, recipe)
            .and_then(|key| sources.previews.get(&key))
            .filter(|(at, _, result)| {
                at.elapsed() < SOURCE_TTL
                    && kassiber_envelope::get(result, "sha256") == Some(expected_sha256)
            })
            .map(|(_, _, result)| result.clone())
            .ok_or_else(|| {
                AppError::new(
                    "chain_analysis_preview_required",
                    "Complete a local dataset preview for these exact source, manifest and hash inputs",
                )
            })
    }
}

#[cfg(target_os = "linux")]
const NOFOLLOW_NONBLOCK: i32 = 0o400000 | 0o4000;
#[cfg(target_os = "macos")]
const NOFOLLOW_NONBLOCK: i32 = 0x0100 | 0x0004;

fn open_nofollow(path: &std::path::Path) -> std::io::Result<File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(NOFOLLOW_NONBLOCK);
    }
    options.open(path)
}

/// `Path.expanduser` for the leading `~` and `~/` forms only.
pub(crate) fn expand_user(raw: &str, home: Option<&str>) -> PathBuf {
    match (raw, home) {
        ("~", Some(home)) => PathBuf::from(home),
        (raw, Some(home)) if raw.starts_with("~/") => PathBuf::from(home).join(&raw[2..]),
        _ => PathBuf::from(raw),
    }
}
