//! `tests/test_operator_service.py`, against a fake backend: the real
//! ownership crate for project identity, scripted owners, connections and
//! modes, and a small argv grammar. Tests that launch real children live in
//! `crates/kassiber/tests/operator_broker.rs`.

use super::*;
use crate::telemetry::{LogRing, Record};
use kassiber_ownership::{OwnerKind, OwnerNamespace};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicUsize;
use std::sync::Barrier;

const WAIT: Duration = Duration::from_secs(10);

pub(crate) fn io_error(message: &str) -> Error {
    Error::Io(std::io::Error::other(message.to_owned()))
}

// ----- the fake backend -----

#[derive(Default)]
pub(crate) struct DbScript {
    pub(crate) identity: Option<String>,
    pub(crate) context: (Option<String>, Option<String>),
    pub(crate) close_failures: Mutex<VecDeque<String>>,
    pub(crate) closes: AtomicUsize,
}

pub(crate) struct FakeDb(pub(crate) Arc<DbScript>);

impl Database for FakeDb {
    fn instance_id(&self) -> Result<Option<String>> {
        Ok(self.0.identity.clone())
    }
    fn context(&self) -> Result<(Option<String>, Option<String>)> {
        Ok(self.0.context.clone())
    }
    fn close(&mut self) -> Result<()> {
        self.0.closes.fetch_add(1, Ordering::SeqCst);
        match locked(&self.0.close_failures).pop_front() {
            Some(message) => Err(io_error(&message)),
            None => Ok(()),
        }
    }
}

/// `set_unlock_mode(root, mode, database_identity, expected_project_identity)`.
type ModeCall = (String, String, Option<String>, Option<String>);

/// The runs a recording runner saw: data root and argv.
type Seen = Arc<Mutex<Vec<(String, Vec<String>)>>>;

type Opener = dyn Fn(&str, &str, Option<&str>) -> Result<Box<dyn Database>> + Send + Sync;

/// A scripted owner (Python's `mock.Mock()` owner).
#[derive(Default)]
pub(crate) struct OwnerScript {
    pub(crate) release_failures: Mutex<VecDeque<String>>,
    pub(crate) releases: AtomicUsize,
    pub(crate) duplicate_failures: Mutex<VecDeque<String>>,
    pub(crate) child_close_failures: Mutex<VecDeque<String>>,
    pub(crate) child_closes: AtomicUsize,
    pub(crate) duplicate_gate: Mutex<Option<(Arc<Event>, Arc<Event>)>>,
}

struct ScriptedOwner(Arc<OwnerScript>);

struct ScriptedChild(Arc<OwnerScript>);

impl ChildOwner for ScriptedChild {
    fn tokens(&self) -> Vec<kassiber_ipc::process::Inheritable> {
        Vec::new()
    }
    fn close(self: Box<Self>) -> Result<()> {
        self.0.child_closes.fetch_add(1, Ordering::SeqCst);
        match locked(&self.0.child_close_failures).pop_front() {
            Some(message) => Err(io_error(&message)),
            None => Ok(()),
        }
    }
}

impl Owner for ScriptedOwner {
    fn add_alias(&mut self, _project: &CanonicalProject) -> Result<()> {
        Ok(())
    }
    fn duplicate_for_child(&self) -> Result<Box<dyn ChildOwner>> {
        let gate = locked(&self.0.duplicate_gate).clone();
        if let Some((entered, release)) = gate {
            entered.set();
            assert!(release.wait(Duration::from_secs(2)), "timed out inheriting");
        }
        match locked(&self.0.duplicate_failures).pop_front() {
            Some(message) => Err(io_error(&message)),
            None => Ok(Box::new(ScriptedChild(Arc::clone(&self.0)))),
        }
    }
    fn release(&mut self) -> Result<()> {
        self.0.releases.fetch_add(1, Ordering::SeqCst);
        match locked(&self.0.release_failures).pop_front() {
            Some(message) => Err(io_error(&message)),
            None => Ok(()),
        }
    }
}

/// The real ownership crate's lease, in an isolated namespace.
struct RealOwner(Option<kassiber_ownership::OwnerLease>);

impl Owner for RealOwner {
    fn add_alias(&mut self, project: &CanonicalProject) -> Result<()> {
        self.0
            .as_mut()
            .expect("held")
            .add_alias(project)
            .map_err(crate::error::from_paths)
    }
    fn duplicate_for_child(&self) -> Result<Box<dyn ChildOwner>> {
        Ok(Box::new(ScriptedChild(Arc::new(OwnerScript::default()))))
    }
    fn release(&mut self) -> Result<()> {
        self.0.take();
        Ok(())
    }
}

pub(crate) struct Fake {
    pub(crate) _owners: tempfile::TempDir,
    pub(crate) namespace: OwnerNamespace,
    pub(crate) opener: Mutex<Arc<Opener>>,
    pub(crate) opened: AtomicUsize,
    pub(crate) owners: Mutex<VecDeque<Arc<OwnerScript>>>,
    pub(crate) modes: Mutex<HashMap<String, String>>,
    pub(crate) mode_calls: Mutex<Vec<ModeCall>>,
    pub(crate) fixed_mode: Mutex<Option<String>>,
    pub(crate) unclassified: AtomicBool,
}

fn default_opener() -> Arc<Opener> {
    Arc::new(|_, _, _| Ok(Box::new(FakeDb(Arc::new(DbScript::default())))))
}

impl Fake {
    pub(crate) fn new() -> Arc<Fake> {
        let owners = tempfile::tempdir().unwrap();
        std::fs::create_dir(owners.path().join("owners")).unwrap();
        Arc::new(Fake {
            namespace: OwnerNamespace::isolated(owners.path().join("owners")),
            _owners: owners,
            opener: Mutex::new(default_opener()),
            opened: AtomicUsize::new(0),
            owners: Mutex::new(VecDeque::new()),
            modes: Mutex::new(HashMap::new()),
            mode_calls: Mutex::new(Vec::new()),
            fixed_mode: Mutex::new(None),
            unclassified: AtomicBool::new(false),
        })
    }

    pub(crate) fn open_with(
        &self,
        opener: impl Fn(&str, &str, Option<&str>) -> Result<Box<dyn Database>> + Send + Sync + 'static,
    ) {
        *locked(&self.opener) = Arc::new(opener);
    }

    pub(crate) fn scripted_owner(&self) -> Arc<OwnerScript> {
        let script = Arc::new(OwnerScript::default());
        locked(&self.owners).push_back(Arc::clone(&script));
        script
    }

    pub(crate) fn opened(&self) -> usize {
        self.opened.load(Ordering::SeqCst)
    }
}

impl Backend for Fake {
    fn canonical_project(&self, data_root: &str) -> Result<CanonicalProject> {
        self.namespace
            .canonical_project(Path::new(data_root))
            .map_err(crate::error::from_paths)
    }
    fn acquire_owner(
        &self,
        project: &CanonicalProject,
        generation: &str,
    ) -> Result<Box<dyn Owner>> {
        if let Some(script) = locked(&self.owners).pop_front() {
            return Ok(Box::new(ScriptedOwner(script)));
        }
        let lease = self
            .namespace
            .acquire(project.clone(), OwnerKind::Broker, generation)
            .map_err(crate::error::from_paths)?;
        Ok(Box::new(RealOwner(Some(lease))))
    }
    fn open_database(
        &self,
        data_root: &str,
        passphrase: &str,
        expected: Option<&str>,
    ) -> Result<Box<dyn Database>> {
        self.opened.fetch_add(1, Ordering::SeqCst);
        let opener = Arc::clone(&locked(&self.opener));
        opener(data_root, passphrase, expected)
    }
    fn auth_backoff_path(&self, data_root: &str) -> Result<Option<PathBuf>> {
        Ok(Some(
            Path::new(data_root)
                .join("config")
                .join(crate::backoff::AUTH_BACKOFF_FILENAME),
        ))
    }
    fn effective_unlock_mode(&self, data_root: &str) -> Result<String> {
        Ok(locked(&self.modes)
            .get(data_root)
            .cloned()
            .unwrap_or_else(|| "manual".into()))
    }
    fn set_unlock_mode(
        &self,
        data_root: &str,
        mode: &str,
        database_identity: Option<&str>,
        expected_project_identity: Option<&str>,
    ) -> Result<String> {
        locked(&self.mode_calls).push((
            data_root.to_owned(),
            mode.to_owned(),
            database_identity.map(str::to_owned),
            expected_project_identity.map(str::to_owned),
        ));
        if let Some(fixed) = locked(&self.fixed_mode).clone() {
            return Ok(fixed);
        }
        locked(&self.modes).insert(data_root.to_owned(), mode.to_owned());
        Ok(mode.to_owned())
    }
    fn classify(&self, argv: &[String]) -> Result<Classified> {
        let parsed = fake_classify(argv)?;
        if self.unclassified.load(Ordering::SeqCst) {
            return Err(Error::App(
                AppError::new(
                    "operator_unclassified_command",
                    "the broker received an unclassified command",
                )
                .with_hint("Upgrade Kassiber or classify the command before brokered use.")
                .with_details(dict([("command", s(&parsed.command_path))])),
            ));
        }
        Ok(parsed)
    }
    fn resolve_data_root(
        &self,
        data_root: Option<&str>,
        _env_file: Option<&str>,
        _project: Option<&str>,
    ) -> Result<String> {
        Ok(data_root.unwrap_or("/unused").to_owned())
    }
}

/// The commands these tests submit: path, capability, scoped.
const COMMANDS: &[(&[&str], Capability, bool)] = &[
    (&["status"], Capability::Read, false),
    (&["health"], Capability::Read, true),
    (&["next-actions"], Capability::Read, true),
    (&["transactions", "list"], Capability::Read, true),
    (
        &["journals", "process"],
        Capability::AccountingDecisions,
        true,
    ),
    (&["context", "set"], Capability::Operator, true),
    (&["secrets", "verify"], Capability::Admin, false),
    (&["secrets", "change-passphrase"], Capability::Admin, false),
    (&["secrets", "remember-unlock"], Capability::Admin, false),
    (&["secrets", "forget-unlock"], Capability::Admin, false),
    (&["backup", "import"], Capability::Admin, false),
    (&["operator", "status"], Capability::Read, false),
    (&["mcp", "call"], Capability::Read, true),
];

/// A small stand-in for argparse: global flags, then a command path from
/// [`COMMANDS`] and its options.
fn fake_classify(argv: &[String]) -> Result<Classified> {
    let invalid = || {
        Error::App(AppError::new(
            "operator_invalid_command",
            "the broker received invalid command arguments",
        ))
    };
    let mut parsed = Classified {
        command_path: String::new(),
        capability: Capability::Read,
        workspace: None,
        profile: None,
        install: false,
        data_root: None,
        project: None,
        env_file: None,
    };
    let mut index = 0;
    while index < argv.len() && argv[index].starts_with("--") {
        let flag = argv[index].as_str();
        match flag {
            "--machine" | "--non-interactive" => index += 1,
            "--data-root" | "--project" | "--env-file" | "--output" | "--format"
            | "--db-passphrase-fd" => {
                let value = argv.get(index + 1).ok_or_else(invalid)?.clone();
                match flag {
                    "--data-root" => parsed.data_root = Some(value),
                    "--project" => parsed.project = Some(value),
                    "--env-file" => parsed.env_file = Some(value),
                    _ => {}
                }
                index += 2;
            }
            _ => return Err(invalid()),
        }
    }
    let rest = &argv[index..];
    let (path, capability, scoped) = COMMANDS
        .iter()
        .filter(|(path, _, _)| rest.len() >= path.len() && rest[..path.len()] == **path)
        .max_by_key(|(path, _, _)| path.len())
        .ok_or_else(invalid)?;
    parsed.command_path = path.join(".");
    parsed.capability = *capability;
    if *scoped {
        parsed.workspace = Some(None);
        parsed.profile = Some(None);
    }
    let mut index = path.len();
    while index < rest.len() {
        match rest[index].as_str() {
            "--workspace" | "--profile" if *scoped => {
                let value = rest.get(index + 1).ok_or_else(invalid)?;
                let slot = if rest[index] == "--workspace" {
                    &mut parsed.workspace
                } else {
                    &mut parsed.profile
                };
                *slot = Some(Some(value.clone()).filter(|value| !value.is_empty()));
                index += 2;
            }
            "--install" => {
                parsed.install = true;
                index += 1;
            }
            _ => index += 1,
        }
    }
    Ok(parsed)
}

// ----- helpers -----

/// `threading.Event`.
#[derive(Default)]
pub(crate) struct Event {
    set: Mutex<bool>,
    changed: Condvar,
}

impl Event {
    pub(crate) fn set(&self) {
        *locked(&self.set) = true;
        self.changed.notify_all();
    }
    pub(crate) fn is_set(&self) -> bool {
        *locked(&self.set)
    }
    pub(crate) fn wait(&self, timeout: Duration) -> bool {
        let guard = locked(&self.set);
        let (guard, _) = self
            .changed
            .wait_timeout_while(guard, timeout, |set| !*set)
            .unwrap_or_else(|poison| poison.into_inner());
        *guard
    }
}

pub(crate) fn ok_runner() -> Arc<dyn Runner> {
    Arc::new(|_: &Dispatch, _: &[u8]| Ok(OperationResult::new(0, "", "")))
}

pub(crate) fn service_with(fake: &Arc<Fake>, runner: Arc<dyn Runner>) -> OperatorService {
    OperatorService::new("generation", Arc::clone(fake) as Arc<dyn Backend>, runner)
}

fn service_with_limits(
    fake: &Arc<Fake>,
    runner: Arc<dyn Runner>,
    limits: Limits,
) -> OperatorService {
    OperatorService::with_options(
        "generation",
        Arc::clone(fake) as Arc<dyn Backend>,
        runner,
        Arc::new(Discard),
        limits,
    )
}

/// Closes the service when a test ends, as Python's `finally`.
pub(crate) struct Closing(pub(crate) OperatorService);

impl Drop for Closing {
    fn drop(&mut self) {
        let _ = self.0.close();
    }
}

impl std::ops::Deref for Closing {
    type Target = OperatorService;
    fn deref(&self) -> &OperatorService {
        &self.0
    }
}

pub(crate) fn argv(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

fn scoped(command: &str) -> Vec<String> {
    argv(&[command, "--workspace", "workspace-a", "--profile", "book-a"])
}

pub(crate) fn root(dir: &tempfile::TempDir) -> String {
    dir.path().to_string_lossy().into_owned()
}

pub(crate) fn unlock(service: &OperatorService, data_root: &str) -> Result<Value> {
    service.unlock(data_root, b"passphrase", UnlockRequest::default())
}

fn submit(service: &OperatorService, data_root: &str, command: Vec<String>) -> Result<Value> {
    service.submit(data_root, &command, SubmitRequest::default())
}

pub(crate) fn text(value: &Value, key: &str) -> String {
    match kassiber_envelope::get(value, key) {
        Some(Value::Str(text)) => text.as_str().unwrap_or_default().to_owned(),
        other => panic!("{key} is {other:?}"),
    }
}

fn id(value: &Value) -> String {
    text(value, "operation_id")
}

pub(crate) fn code(result: Result<impl std::fmt::Debug>) -> String {
    match result {
        Err(Error::App(error)) => error.code,
        other => panic!("expected an AppError, got {other:?}"),
    }
}

fn wait_with(
    service: &OperatorService,
    operation_id: &str,
    states: &[&str],
    within: Duration,
) -> Value {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        let status = service.operation_status(operation_id, true);
        if states.contains(&text(&status, "state").as_str()) {
            return status;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("operation {operation_id} did not finish");
}

fn wait(service: &OperatorService, operation_id: &str) -> Value {
    wait_with(
        service,
        operation_id,
        &["completed", "failed"],
        Duration::from_secs(2),
    )
}

fn wait_terminal(service: &OperatorService, operation_id: &str) -> Value {
    wait_with(
        service,
        operation_id,
        &["completed", "failed", "cancelled", "result_unknown"],
        WAIT,
    )
}

fn state_of(service: &OperatorService, operation_id: &str) -> String {
    text(&service.operation_status(operation_id, true), "state")
}

pub(crate) fn identity(fake: &Fake, data_root: &str) -> String {
    fake.canonical_project(data_root).unwrap().identity
}

fn with_lease<T>(
    service: &OperatorService,
    fake: &Fake,
    data_root: &str,
    change: impl FnOnce(&mut Lease) -> T,
) -> T {
    let identity = identity(fake, data_root);
    let mut state = service.state();
    change(state.leases.get_mut(&identity).expect("lease"))
}

fn expire(lease: &mut Lease) {
    lease.expires_at = Some(Instant::now() - Duration::from_secs(1));
}

/// A runner that blocks its first call until released.
struct Blocking {
    started: Arc<Event>,
    release: Arc<Event>,
    calls: Arc<Mutex<Vec<String>>>,
    first_only: bool,
    result: OperationResult,
}

impl Blocking {
    fn new(first_only: bool) -> Arc<Blocking> {
        Arc::new(Blocking {
            started: Arc::new(Event::default()),
            release: Arc::new(Event::default()),
            calls: Arc::new(Mutex::new(Vec::new())),
            first_only,
            result: OperationResult::new(0, "", ""),
        })
    }
    fn calls(&self) -> Vec<String> {
        locked(&self.calls).clone()
    }
}

impl Runner for Blocking {
    fn run(&self, dispatch: &Dispatch, _: &[u8]) -> Result<OperationResult> {
        let first = {
            let mut calls = locked(&self.calls);
            calls.push(dispatch.command_path.clone());
            calls.len() == 1
        };
        if first || !self.first_only {
            self.started.set();
            self.release.wait(Duration::from_secs(2));
        }
        Ok(self.result.clone())
    }
}

// ----- the tests -----

#[test]
fn test_native_auth_restart_requires_a_fully_idle_broker() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let service = Closing(service_with(&fake, ok_runner()));
    unlock(&service, &root(&tmp)).unwrap();
    assert_eq!(code(service.prepare_idle_restart()), "operator_broker_busy");
    service.lock(&root(&tmp), None, false).unwrap();
    assert_eq!(
        text(&service.prepare_idle_restart().unwrap(), "restart"),
        "accepted"
    );
}

#[test]
fn test_native_unlock_rejects_unexpected_database_identity() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let service = Closing(service_with(&fake, ok_runner()));
    let expected = "f".repeat(32);
    let result = service.unlock(
        &root(&tmp),
        b"passphrase",
        UnlockRequest {
            authentication_method: "touch_id",
            expected_database_identity: Some(&expected),
            ..UnlockRequest::default()
        },
    );
    assert_eq!(code(result), "operator_project_replaced");
    assert!(service.state().leases.is_empty());
}

#[test]
fn test_native_unlock_rejects_project_replacement_before_open() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let service = Closing(service_with(&fake, ok_runner()));
    let expected = "f".repeat(64);
    let result = service.unlock(
        &root(&tmp),
        b"passphrase",
        UnlockRequest {
            authentication_method: "touch_id",
            expected_project_identity: Some(&expected),
            ..UnlockRequest::default()
        },
    );
    assert_eq!(code(result), "operator_project_replaced");
    assert_eq!(fake.opened(), 0);
    assert!(service.state().leases.is_empty());
}

#[test]
fn test_admin_cannot_be_granted_as_a_standing_lease() {
    let fake = Fake::new();
    let service = Closing(service_with(&fake, ok_runner()));
    let result = service.unlock(
        "/not-opened",
        b"passphrase",
        UnlockRequest {
            capability: Capability::Admin,
            ..UnlockRequest::default()
        },
    );
    assert_eq!(code(result), "operator_invalid_lease_capability");
    assert!(service.state().leases.is_empty());
}

#[test]
fn test_auth_backoff_cache_is_bounded_and_lru() {
    let fake = Fake::new();
    let service = Closing(service_with(&fake, ok_runner()));
    {
        let mut state = service.state();
        for index in 0..=MAX_CACHED_AUTH_BACKOFFS {
            service
                .auth_backoff_locked(
                    &mut state,
                    &format!("project-{index}"),
                    &format!("/not-opened/project-{index}"),
                )
                .unwrap();
        }
    }
    let state = service.state();
    assert_eq!(state.auth_backoffs.len(), MAX_CACHED_AUTH_BACKOFFS);
    assert!(!state.auth_backoffs.contains("project-0"));
    assert!(state
        .auth_backoffs
        .contains(&format!("project-{MAX_CACHED_AUTH_BACKOFFS}")));
}

#[test]
fn test_unlock_retains_owner_until_failed_verification_close_retries() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let script = Arc::new(DbScript::default());
    locked(&script.close_failures).extend(["close failed".into(), "still failed".into()]);
    let opened = Arc::clone(&script);
    fake.open_with(move |_, _, _| Ok(Box::new(FakeDb(Arc::clone(&opened)))));
    let owner = fake.scripted_owner();
    let service = Closing(service_with(&fake, ok_runner()));
    let error = unlock(&service, &root(&tmp)).unwrap_err();
    assert_eq!(error.to_string(), "still failed");
    assert_eq!(owner.releases.load(Ordering::SeqCst), 0);
    assert!(!service.state().pending_owner_releases.is_empty());
    service.release_pending_owners(None).unwrap();
    assert_eq!(script.closes.load(Ordering::SeqCst), 3);
    assert_eq!(owner.releases.load(Ordering::SeqCst), 1);
    assert!(service.state().pending_owner_releases.is_empty());
}

#[test]
fn test_unlock_retains_owner_when_failed_open_release_needs_retry() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    fake.open_with(|_, _, _| {
        Err(Error::App(AppError::new(
            "unlock_failed",
            "wrong passphrase",
        )))
    });
    let owner = fake.scripted_owner();
    locked(&owner.release_failures).extend(["release failed".into(), "still failed".into()]);
    let service = Closing(service_with(&fake, ok_runner()));
    let error = service
        .unlock(&root(&tmp), b"wrong-passphrase", UnlockRequest::default())
        .unwrap_err();
    assert_eq!(error.to_string(), "still failed");
    assert_eq!(owner.releases.load(Ordering::SeqCst), 2);
    assert!(!service.state().pending_owner_releases.is_empty());
    service.release_pending_owners(None).unwrap();
    assert_eq!(owner.releases.load(Ordering::SeqCst), 3);
    assert!(service.state().pending_owner_releases.is_empty());
}

#[test]
fn test_fresh_auth_retains_owner_until_failed_connection_close_retries() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let script = Arc::new(DbScript::default());
    locked(&script.close_failures).extend(["close failed".into(), "still failed".into()]);
    let opened = Arc::clone(&script);
    fake.open_with(move |_, _, _| Ok(Box::new(FakeDb(Arc::clone(&opened)))));
    let owner = fake.scripted_owner();
    let service = Closing(service_with(&fake, ok_runner()));
    let error = service
        .authenticate_database(&root(&tmp), b"passphrase", "operator_mode", false, |_| {
            Ok(())
        })
        .unwrap_err();
    assert_eq!(error.to_string(), "still failed");
    assert_eq!(owner.releases.load(Ordering::SeqCst), 0);
    assert!(!service.state().pending_owner_releases.is_empty());
    service.release_pending_owners(None).unwrap();
    assert_eq!(script.closes.load(Ordering::SeqCst), 3);
    assert_eq!(owner.releases.load(Ordering::SeqCst), 1);
    assert!(service.state().pending_owner_releases.is_empty());
}

#[test]
fn test_lifecycle_telemetry_is_bounded_ram_only_and_public_safe() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let ring = Arc::new(LogRing::new(3));
    let service = Closing(OperatorService::with_options(
        "generation",
        Arc::clone(&fake) as Arc<dyn Backend>,
        ok_runner(),
        Arc::clone(&ring) as Arc<dyn Telemetry>,
        Limits::default(),
    ));
    service
        .unlock(
            &root(&tmp),
            b"blinding_key=private-value",
            UnlockRequest::default(),
        )
        .unwrap();
    let accepted = submit(&service, &root(&tmp), argv(&["status"])).unwrap();
    wait_terminal(&service, &id(&accepted));
    service.lock(&root(&tmp), None, false).unwrap();
    let records: Vec<Record> = ring.snapshot();
    assert!(records.len() <= 3);
    let rendered = format!("{records:?}");
    assert!(!rendered.contains(&root(&tmp)));
    assert!(!rendered.contains("private-value"));
    assert!(records
        .iter()
        .any(|record| record.message.contains("operator operation")));
}

#[test]
fn test_fresh_database_authentication_does_not_require_a_lease() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let owner = fake.scripted_owner();
    let service = Closing(service_with(&fake, ok_runner()));
    let checked = Arc::clone(&owner);
    let result = service
        .authenticate_database(
            &root(&tmp),
            b"fresh-passphrase",
            "operator_mode",
            false,
            |_| {
                assert_eq!(checked.releases.load(Ordering::SeqCst), 0);
                Ok("configured")
            },
        )
        .unwrap();
    assert_eq!(result, "configured");
    assert_eq!(
        text(&service.status(Some(&root(&tmp))).unwrap(), "lease"),
        "locked"
    );
    assert_eq!(owner.releases.load(Ordering::SeqCst), 1);
}

#[test]
fn test_fresh_authentication_releases_owner_when_backoff_rejects() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let backoff = fake.auth_backoff_path(&root(&tmp)).unwrap().unwrap();
    std::fs::create_dir_all(backoff.parent().unwrap()).unwrap();
    std::fs::write(&backoff, "{\"failures\": 5, \"locked_until\": 1e300}").unwrap();
    let owner = fake.scripted_owner();
    let service = Closing(service_with(&fake, ok_runner()));
    let result = service.authenticate_database(
        &root(&tmp),
        b"fresh-passphrase",
        "operator_mode",
        false,
        |_| Ok(()),
    );
    assert_eq!(code(result), "local_auth_rate_limited");
    assert_eq!(owner.releases.load(Ordering::SeqCst), 1);
    assert_eq!(fake.opened(), 0);
}

#[cfg(unix)]
#[test]
fn test_mode_authentication_and_continuation_use_canonical_root() {
    let tmp = tempfile::tempdir().unwrap();
    let parent = tempfile::tempdir().unwrap();
    let alias = parent.path().join("project");
    std::os::unix::fs::symlink(tmp.path(), &alias).unwrap();
    let fake = Fake::new();
    let owner = fake.scripted_owner();
    let roots = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&roots);
    fake.open_with(move |data_root, _, _| {
        locked(&seen).push(data_root.to_owned());
        Ok(Box::new(FakeDb(Arc::new(DbScript::default()))))
    });
    *locked(&fake.fixed_mode) = Some("manual".into());
    let service = Closing(service_with(&fake, ok_runner()));
    let result = service
        .set_mode_authenticated(&alias.to_string_lossy(), b"passphrase", "manual")
        .unwrap();
    let canonical = std::fs::canonicalize(tmp.path())
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_eq!(text(&result, "mode"), "manual");
    assert_eq!(locked(&roots)[0], canonical);
    let identity = identity(&fake, &canonical);
    assert_eq!(
        *locked(&fake.mode_calls),
        vec![(
            canonical,
            "manual".to_owned(),
            Some(identity[..32].to_owned()),
            Some(identity.clone())
        )]
    );
    assert_eq!(owner.releases.load(Ordering::SeqCst), 1);
}

#[test]
fn test_project_worker_serializes_operations_and_retains_results() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let started = Arc::new(Mutex::new(Vec::<String>::new()));
    let first_started = Arc::new(Event::default());
    let release_first = Arc::new(Event::default());
    let (log, first, release) = (
        Arc::clone(&started),
        Arc::clone(&first_started),
        Arc::clone(&release_first),
    );
    let runner = Arc::new(move |dispatch: &Dispatch, _: &[u8]| {
        let count = {
            let mut log = locked(&log);
            log.push(dispatch.id.clone());
            log.len()
        };
        if count == 1 {
            first.set();
            release.wait(Duration::from_secs(2));
        }
        Ok(OperationResult::new(
            0,
            format!("{}\n", dispatch.command_path),
            "",
        ))
    });
    let service = Closing(service_with(&fake, runner));
    service
        .unlock(
            &root(&tmp),
            b"correct horse battery staple",
            UnlockRequest::default(),
        )
        .unwrap();
    let one = submit(&service, &root(&tmp), argv(&["status"])).unwrap();
    assert!(first_started.wait(Duration::from_secs(1)));
    let two = submit(&service, &root(&tmp), scoped("health")).unwrap();
    assert_eq!(state_of(&service, &id(&two)), "queued");
    release_first.set();
    let first_result = wait(&service, &id(&one));
    let second_result = wait(&service, &id(&two));
    assert_eq!(text(&first_result, "state"), "completed");
    assert_eq!(text(&first_result, "stdout"), "status\n");
    assert_eq!(text(&second_result, "state"), "completed");
    assert_eq!(*locked(&started), vec![id(&one), id(&two)]);
}

#[test]
fn test_project_worker_is_created_once_and_stops_with_the_service() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let service = service_with(&fake, ok_runner());
    unlock(&service, &root(&tmp)).unwrap();
    let ids: Vec<String> = ["status", "health", "next-actions"]
        .iter()
        .map(|command| {
            let command = if *command == "status" {
                argv(&["status"])
            } else {
                scoped(command)
            };
            id(&submit(&service, &root(&tmp), command).unwrap())
        })
        .collect();
    for operation in &ids {
        assert_eq!(
            text(&wait_terminal(&service, operation), "state"),
            "completed"
        );
    }
    let worker = Arc::clone(service.state().workers.values().next().unwrap());
    unlock(&service, &root(&tmp)).unwrap();
    let workers: Vec<Arc<Worker>> = service.state().workers.values().cloned().collect();
    assert_eq!(workers.len(), 1);
    assert!(Arc::ptr_eq(&workers[0], &worker));
    assert!(!worker.finished());
    service.close().unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    while !worker.finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(worker.finished());
}

#[test]
fn test_admin_never_inherits_standing_lease() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let service = Closing(service_with(&fake, ok_runner()));
    unlock(&service, &root(&tmp)).unwrap();
    assert_eq!(
        code(submit(&service, &root(&tmp), argv(&["secrets", "verify"]))),
        "operator_admin_auth_required"
    );
}

#[test]
fn test_admin_authorization_is_single_use_and_bound_to_lease_epoch() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let service = Closing(service_with(&fake, ok_runner()));
    let data_root = root(&tmp);
    unlock(&service, &data_root).unwrap();
    let stale = service.verify_admin(&data_root, b"passphrase").unwrap();
    service.lock(&data_root, None, false).unwrap();
    unlock(&service, &data_root).unwrap();
    let admin = |authorization| {
        service.submit(
            &data_root,
            &argv(&["secrets", "verify"]),
            SubmitRequest {
                admin_authorization: Some(authorization),
                ..SubmitRequest::default()
            },
        )
    };
    assert_eq!(code(admin(&stale)), "operator_admin_auth_required");
    let fresh = service.verify_admin(&data_root, b"passphrase").unwrap();
    let accepted = admin(&fresh).unwrap();
    assert_eq!(
        text(&wait_terminal(&service, &id(&accepted)), "state"),
        "completed"
    );
    assert_eq!(code(admin(&fresh)), "operator_admin_auth_required");
}

#[test]
fn test_rejected_submissions_wipe_every_staged_secret() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let service = Closing(service_with(&fake, ok_runner()));
    let data_root = root(&tmp);
    let rejected = |command: Vec<String>, expected: &str, operation_id: Option<&str>| {
        let result = service.submit(
            &data_root,
            &command,
            SubmitRequest {
                operation_id: operation_id.map(str::to_owned),
                secret_arguments: vec![(
                    "broker-secret-test".into(),
                    Zeroizing::new(b"staged-secret".to_vec()),
                )],
                ..SubmitRequest::default()
            },
        );
        assert_eq!(code(result), expected);
        // The staged secret was dropped (and so wiped) with the request.
        assert!(service
            .state()
            .operations
            .values()
            .all(|operation| operation.secret_arguments.is_empty()));
    };
    rejected(
        argv(&["--definitely-invalid"]),
        "operator_invalid_command",
        None,
    );
    rejected(
        argv(&["operator", "status"]),
        "operator_command_not_brokerable",
        None,
    );
    unlock(&service, &data_root).unwrap();
    rejected(
        argv(&["transactions", "list"]),
        "operator_scope_required",
        None,
    );
    rejected(argv(&["health"]), "operator_scope_required", None);
    rejected(
        argv(&["status"]),
        "operator_protocol_error",
        Some("invalid-operation-id"),
    );
    rejected(
        argv(&["secrets", "verify"]),
        "operator_admin_auth_required",
        None,
    );
    with_lease(&service, &fake, &data_root, |lease| {
        lease.capability = Capability::Read
    });
    rejected(
        argv(&[
            "journals",
            "process",
            "--workspace",
            "workspace-a",
            "--profile",
            "book-a",
        ]),
        "operator_capability_denied",
        None,
    );
    assert!(service.state().operations.is_empty());
}

#[test]
fn test_unclassified_command_is_a_public_safe_terminal_rejection() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    fake.unclassified.store(true, Ordering::SeqCst);
    let service = Closing(service_with(&fake, ok_runner()));
    let error = service
        .submit(
            &root(&tmp),
            &argv(&["status"]),
            SubmitRequest {
                secret_arguments: vec![(
                    "broker-secret-test".into(),
                    Zeroizing::new(b"staged-secret".to_vec()),
                )],
                ..SubmitRequest::default()
            },
        )
        .unwrap_err();
    let error = error.app().unwrap();
    assert_eq!(error.code, "operator_unclassified_command");
    assert!(!error.retryable);
}

#[test]
fn test_generation_change_reports_result_unknown() {
    let fake = Fake::new();
    let service = Closing(OperatorService::new(
        "new-generation",
        Arc::clone(&fake) as Arc<dyn Backend>,
        ok_runner(),
    ));
    let status = service.operation_status("old-generation.client.123", true);
    assert_eq!(text(&status, "state"), "result_unknown");
    assert_eq!(text(&status, "reason"), "broker_generation_changed");
}

#[test]
fn test_lock_wipes_retained_passphrase() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let service = service_with(&fake, ok_runner());
    unlock(&service, &root(&tmp)).unwrap();
    service.lock(&root(&tmp), None, false).unwrap();
    // The lease (and with it the zeroizing passphrase buffer) is gone.
    assert!(service.state().leases.is_empty());
    assert!(service.state().workers.is_empty());
    service.close().unwrap();
}

#[test]
fn test_lock_cancels_queued_work_but_running_work_finishes() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let blocking = Blocking::new(false);
    let service = Closing(service_with(
        &fake,
        Arc::clone(&blocking) as Arc<dyn Runner>,
    ));
    let data_root = root(&tmp);
    unlock(&service, &data_root).unwrap();
    let running = submit(&service, &data_root, argv(&["status"])).unwrap();
    assert!(blocking.started.wait(Duration::from_secs(1)));
    let queued = submit(&service, &data_root, scoped("health")).unwrap();
    let locked_status = service.lock(&data_root, None, false).unwrap();
    assert_eq!(
        kassiber_envelope::get(&locked_status, "running_operations_finishing"),
        Some(&int(1u64))
    );
    assert_eq!(state_of(&service, &id(&queued)), "cancelled");
    assert_eq!(
        code(submit(&service, &data_root, scoped("next-actions"))),
        "interaction_required"
    );
    blocking.release.set();
    assert_eq!(text(&wait(&service, &id(&running)), "state"), "completed");
    assert_eq!(
        text(&wait_terminal(&service, &id(&queued)), "state"),
        "cancelled"
    );
}

#[test]
fn test_expiry_between_admission_and_dispatch_cancels_queued_work() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let blocking = Blocking::new(false);
    let service = Closing(service_with(
        &fake,
        Arc::clone(&blocking) as Arc<dyn Runner>,
    ));
    let data_root = root(&tmp);
    unlock(&service, &data_root).unwrap();
    let running = submit(&service, &data_root, argv(&["status"])).unwrap();
    assert!(blocking.started.wait(Duration::from_secs(1)));
    let queued = submit(&service, &data_root, scoped("health")).unwrap();
    with_lease(&service, &fake, &data_root, expire);
    blocking.release.set();
    assert_eq!(
        text(&wait_terminal(&service, &id(&running)), "state"),
        "completed"
    );
    assert_eq!(
        text(&wait_terminal(&service, &id(&queued)), "state"),
        "cancelled"
    );
}

#[test]
fn test_janitor_survives_double_owner_release_failure() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let first_owner = fake.scripted_owner();
    locked(&first_owner.release_failures).extend([
        "simulated owner release failure".into(),
        "simulated owner release failure".into(),
    ]);
    let second_owner = fake.scripted_owner();
    let service = Closing(service_with(&fake, ok_runner()));
    unlock(&service, &root(&first)).unwrap();
    unlock(&service, &root(&second)).unwrap();
    with_lease(&service, &fake, &root(&first), expire);
    let until = |check: &dyn Fn() -> bool| {
        let deadline = Instant::now() + Duration::from_millis(3500);
        while !check() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        check()
    };
    assert!(until(&|| first_owner.releases.load(Ordering::SeqCst) >= 2));
    assert!(!locked(&service.inner.janitor)
        .as_ref()
        .unwrap()
        .is_finished());
    with_lease(&service, &fake, &root(&second), expire);
    assert!(until(&|| second_owner.releases.load(Ordering::SeqCst) >= 1));
    let second_identity = identity(&fake, &root(&second));
    assert!(!service.state().leases.contains(&second_identity));
    assert!(!locked(&service.inner.janitor)
        .as_ref()
        .unwrap()
        .is_finished());
}

#[test]
fn test_expired_worker_queue_cannot_revive_after_reunlock() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let blocking = Blocking::new(true);
    let service = Closing(service_with(
        &fake,
        Arc::clone(&blocking) as Arc<dyn Runner>,
    ));
    let data_root = root(&tmp);
    unlock(&service, &data_root).unwrap();
    let running = submit(&service, &data_root, argv(&["status"])).unwrap();
    assert!(blocking.started.wait(Duration::from_secs(1)));
    let stale = submit(&service, &data_root, scoped("health")).unwrap();
    with_lease(&service, &fake, &data_root, expire);
    blocking.release.set();
    assert_eq!(
        text(&wait_terminal(&service, &id(&running)), "state"),
        "completed"
    );
    assert_eq!(
        text(&wait_terminal(&service, &id(&stale)), "state"),
        "cancelled"
    );
    service
        .unlock(&data_root, b"second-passphrase", UnlockRequest::default())
        .unwrap();
    let fresh = submit(&service, &data_root, scoped("health")).unwrap();
    assert_eq!(
        text(&wait_terminal(&service, &id(&fresh)), "state"),
        "completed"
    );
    assert_eq!(blocking.calls(), ["status", "health"]);
}

/// Pauses a worker after it popped an operation and before it enters the
/// project gate (`mock.patch.object(ProjectWorker, "_run_one", ...)`).
fn pause_dequeued(service: &OperatorService, worker_identity: &str) -> (Arc<Event>, Arc<Event>) {
    let dequeued = Arc::new(Event::default());
    let release = Arc::new(Event::default());
    let worker = Arc::clone(&service.state().workers[worker_identity]);
    // Hold the worker's next pop: take the queue's head ourselves and feed
    // it to run_one from a helper thread once released.
    let paused = service.clone();
    let (entered, go) = (Arc::clone(&dequeued), Arc::clone(&release));
    std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(2);
        let operation_id = loop {
            if let Some(id) = locked(&worker.queue).0.pop_front() {
                break id;
            }
            assert!(Instant::now() < deadline, "nothing was queued");
            std::thread::yield_now();
        };
        entered.set();
        assert!(
            go.wait(Duration::from_secs(2)),
            "timed out releasing dequeued operation"
        );
        let _ = paused.run_one(&worker, &operation_id);
    });
    (dequeued, release)
}

/// A worker that has not been started yet, so tests can pop for it.
fn unlock_with_paused_worker(
    service: &OperatorService,
    fake: &Fake,
    data_root: &str,
    passphrase: &[u8],
) {
    service
        .unlock(data_root, passphrase, UnlockRequest::default())
        .unwrap();
    let identity = identity(fake, data_root);
    // Stop the real worker thread; the pause helper runs operations itself.
    let worker = Arc::clone(&service.state().workers[&identity]);
    worker.stop();
    let deadline = Instant::now() + Duration::from_secs(1);
    while !worker.finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    locked(&worker.queue).1 = false;
}

#[test]
fn test_expired_dequeued_operation_cannot_run_under_a_new_lease() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = Arc::clone(&calls);
    let runner = Arc::new(move |dispatch: &Dispatch, _: &[u8]| {
        locked(&log).push(dispatch.command_path.clone());
        Ok(OperationResult::new(0, "", ""))
    });
    let service = Closing(service_with(&fake, runner));
    let data_root = root(&tmp);
    unlock_with_paused_worker(&service, &fake, &data_root, b"first-passphrase");
    let (dequeued, release) = pause_dequeued(&service, &identity(&fake, &data_root));
    let stale = submit(&service, &data_root, argv(&["status"])).unwrap();
    assert!(dequeued.wait(Duration::from_secs(1)));
    with_lease(&service, &fake, &data_root, expire);
    assert_eq!(
        code(submit(&service, &data_root, scoped("health"))),
        "interaction_required"
    );
    assert_eq!(state_of(&service, &id(&stale)), "cancelled");
    service
        .unlock(&data_root, b"second-passphrase", UnlockRequest::default())
        .unwrap();
    release.set();
    let fresh = submit(&service, &data_root, scoped("health")).unwrap();
    assert_eq!(
        text(&wait_terminal(&service, &id(&fresh)), "state"),
        "completed"
    );
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(*locked(&calls), ["health"]);
}

#[test]
fn test_direct_reunlock_cancels_dequeued_work_from_expired_lease() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = Arc::clone(&calls);
    let runner = Arc::new(move |dispatch: &Dispatch, _: &[u8]| {
        locked(&log).push(dispatch.command_path.clone());
        Ok(OperationResult::new(0, "", ""))
    });
    let service = Closing(service_with(&fake, runner));
    let data_root = root(&tmp);
    unlock_with_paused_worker(&service, &fake, &data_root, b"first-passphrase");
    let (dequeued, release) = pause_dequeued(&service, &identity(&fake, &data_root));
    let stale = submit(&service, &data_root, argv(&["status"])).unwrap();
    assert!(dequeued.wait(Duration::from_secs(1)));
    with_lease(&service, &fake, &data_root, expire);
    service
        .unlock(&data_root, b"second-passphrase", UnlockRequest::default())
        .unwrap();
    assert_eq!(state_of(&service, &id(&stale)), "cancelled");
    release.set();
    let fresh = submit(&service, &data_root, scoped("health")).unwrap();
    assert_eq!(
        text(&wait_terminal(&service, &id(&fresh)), "state"),
        "completed"
    );
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(*locked(&calls), ["health"]);
}

#[test]
fn test_queued_cancellation_is_immediately_terminal_and_wipes_secrets() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let blocking = Blocking::new(false);
    let service = Closing(service_with(
        &fake,
        Arc::clone(&blocking) as Arc<dyn Runner>,
    ));
    let data_root = root(&tmp);
    unlock(&service, &data_root).unwrap();
    submit(&service, &data_root, argv(&["status"])).unwrap();
    assert!(blocking.started.wait(Duration::from_secs(1)));
    let queued = service
        .submit(
            &data_root,
            &argv(&["status"]),
            SubmitRequest {
                secret_arguments: vec![(
                    "broker-secret-test".into(),
                    Zeroizing::new(b"token".to_vec()),
                )],
                ..SubmitRequest::default()
            },
        )
        .unwrap();
    assert!(!service.state().operations[&id(&queued)]
        .secret_arguments
        .is_empty());
    let cancelled = service.cancel(&id(&queued));
    assert_eq!(text(&cancelled, "state"), "cancelled");
    assert!(service.state().operations[&id(&queued)]
        .secret_arguments
        .is_empty());
    blocking.release.set();
}

#[test]
fn test_running_cancel_is_truthfully_not_cancellable() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let blocking = Blocking::new(false);
    let service = Closing(service_with(
        &fake,
        Arc::clone(&blocking) as Arc<dyn Runner>,
    ));
    unlock(&service, &root(&tmp)).unwrap();
    let accepted = submit(&service, &root(&tmp), argv(&["status"])).unwrap();
    assert!(blocking.started.wait(Duration::from_secs(1)));
    let cancelled = service.cancel(&id(&accepted));
    assert_eq!(text(&cancelled, "state"), "running");
    assert_eq!(text(&cancelled, "cancellation"), "not_cancellable");
    blocking.release.set();
}

#[test]
fn test_capability_is_rechecked_immediately_before_dispatch() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let blocking = Blocking::new(false);
    let service = Closing(service_with(
        &fake,
        Arc::clone(&blocking) as Arc<dyn Runner>,
    ));
    let data_root = root(&tmp);
    unlock(&service, &data_root).unwrap();
    let running = submit(&service, &data_root, argv(&["status"])).unwrap();
    assert!(blocking.started.wait(Duration::from_secs(1)));
    let queued = submit(
        &service,
        &data_root,
        argv(&[
            "journals",
            "process",
            "--workspace",
            "workspace-a",
            "--profile",
            "book-a",
        ]),
    )
    .unwrap();
    with_lease(&service, &fake, &data_root, |lease| {
        lease.capability = Capability::Read
    });
    blocking.release.set();
    assert_eq!(
        text(&wait_terminal(&service, &id(&running)), "state"),
        "completed"
    );
    assert_eq!(
        text(&wait_terminal(&service, &id(&queued)), "state"),
        "cancelled"
    );
}

#[test]
fn test_different_projects_run_concurrently() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let starts = Arc::new(Mutex::new(Vec::<String>::new()));
    let both = Arc::new(Event::default());
    let release = Arc::new(Event::default());
    let (log, started, go) = (Arc::clone(&starts), Arc::clone(&both), Arc::clone(&release));
    let runner = Arc::new(move |dispatch: &Dispatch, _: &[u8]| {
        {
            let mut log = locked(&log);
            log.push(dispatch.data_root.clone());
            if log.len() == 2 {
                started.set();
            }
        }
        go.wait(Duration::from_secs(2));
        Ok(OperationResult::new(0, "", ""))
    });
    let service = Closing(service_with(&fake, runner));
    service
        .unlock(&root(&first), b"first-passphrase", UnlockRequest::default())
        .unwrap();
    service
        .unlock(
            &root(&second),
            b"second-passphrase",
            UnlockRequest::default(),
        )
        .unwrap();
    let one = submit(&service, &root(&first), argv(&["status"])).unwrap();
    let two = submit(&service, &root(&second), argv(&["status"])).unwrap();
    assert!(both.wait(Duration::from_secs(1)));
    release.set();
    assert_eq!(text(&wait(&service, &id(&one)), "state"), "completed");
    assert_eq!(text(&wait(&service, &id(&two)), "state"), "completed");
    let starts = locked(&starts).clone();
    assert_ne!(starts[0], starts[1]);
}

fn recording_runner() -> (Seen, Arc<dyn Runner>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    let runner = Arc::new(move |dispatch: &Dispatch, _: &[u8]| {
        locked(&log).push((dispatch.data_root.clone(), dispatch.argv.clone()));
        Ok(OperationResult::new(0, "", ""))
    });
    (seen, runner)
}

#[test]
fn test_explicit_book_scope_survives_queue_admission() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let (seen, runner) = recording_runner();
    let service = Closing(service_with(&fake, runner));
    unlock(&service, &root(&tmp)).unwrap();
    let accepted = submit(
        &service,
        &root(&tmp),
        argv(&[
            "transactions",
            "list",
            "--workspace",
            "workspace-a",
            "--profile",
            "book-b",
        ]),
    )
    .unwrap();
    assert_eq!(text(&wait(&service, &id(&accepted)), "state"), "completed");
    let canonical = std::fs::canonicalize(tmp.path())
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        locked(&seen)[0].1[..8],
        argv(&[
            "--data-root",
            &canonical,
            "transactions",
            "list",
            "--workspace",
            "workspace-a",
            "--profile",
            "book-b"
        ])
    );
}

#[test]
fn test_child_project_locator_cannot_borrow_another_projects_lease() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let (seen, runner) = recording_runner();
    let service = Closing(service_with(&fake, runner));
    service
        .unlock(
            &root(&first),
            b"shared-passphrase",
            UnlockRequest::default(),
        )
        .unwrap();
    let error = submit(
        &service,
        &root(&first),
        argv(&["--data-root", &root(&second), "--machine", "status"]),
    )
    .unwrap_err();
    let error = error.app().unwrap();
    assert_eq!(error.code, "operator_project_mismatch");
    assert!(locked(&seen).is_empty());
    let details = format!("{:?}", error.details);
    assert!(!details.contains(&root(&first)));
    assert!(!details.contains(&root(&second)));
}

#[test]
fn test_operation_id_cannot_be_reused_across_projects() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let service = Closing(service_with(&fake, ok_runner()));
    service
        .unlock(&root(&first), b"first", UnlockRequest::default())
        .unwrap();
    service
        .unlock(&root(&second), b"second", UnlockRequest::default())
        .unwrap();
    let fixed = || SubmitRequest {
        operation_id: Some("generation.client.fixed".into()),
        ..SubmitRequest::default()
    };
    service
        .submit(&root(&first), &argv(&["status"]), fixed())
        .unwrap();
    assert_eq!(
        code(service.submit(&root(&second), &argv(&["status"]), fixed())),
        "operator_operation_id_conflict"
    );
}

#[test]
fn test_operation_id_replay_must_repeat_the_callers_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let caller = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let blocking = Blocking::new(false);
    let service = Closing(service_with(
        &fake,
        Arc::clone(&blocking) as Arc<dyn Runner>,
    ));
    unlock(&service, &root(&tmp)).unwrap();
    let request = |directory: &str, no_egress: bool| SubmitRequest {
        operation_id: Some("generation.client.fixed".into()),
        working_directory: Some(directory.to_owned()),
        no_egress,
        ..SubmitRequest::default()
    };
    service
        .submit(
            &root(&tmp),
            &argv(&["status"]),
            request(&root(&caller), false),
        )
        .unwrap();
    let replay = service
        .submit(
            &root(&tmp),
            &argv(&["status"]),
            request(&root(&caller), false),
        )
        .unwrap();
    assert_eq!(id(&replay), "generation.client.fixed");
    for (directory, no_egress) in [(root(&tmp), false), (root(&caller), true)] {
        assert_eq!(
            code(service.submit(
                &root(&tmp),
                &argv(&["status"]),
                request(&directory, no_egress)
            )),
            "operator_operation_id_conflict"
        );
    }
    blocking.release.set();
}

#[test]
fn test_replaced_database_path_keeps_prior_lease_revocable() {
    let tmp = tempfile::tempdir().unwrap();
    let database = tmp.path().join("kassiber.sqlite3");
    std::fs::write(&database, b"first").unwrap();
    let fake = Fake::new();
    let service = Closing(service_with(&fake, ok_runner()));
    unlock(&service, &root(&tmp)).unwrap();
    std::fs::rename(&database, tmp.path().join("old.sqlite3")).unwrap();
    std::fs::write(&database, b"replacement").unwrap();
    let status = service.status(Some(&root(&tmp))).unwrap();
    assert_eq!(
        kassiber_envelope::get(&status, "project_file_changed"),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        code(submit(&service, &root(&tmp), argv(&["status"]))),
        "operator_project_replaced"
    );
    let locked_status = service.lock(&root(&tmp), None, false).unwrap();
    assert_eq!(
        kassiber_envelope::get(&locked_status, "lease_existed"),
        Some(&Value::Bool(true))
    );
    assert!(service.state().leases.is_empty());
    assert!(service.state().workers.is_empty());
}

#[test]
fn test_bounded_queue_rejects_before_false_acceptance() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let blocking = Blocking::new(true);
    let service = Closing(service_with(
        &fake,
        Arc::clone(&blocking) as Arc<dyn Runner>,
    ));
    let data_root = root(&tmp);
    unlock(&service, &data_root).unwrap();
    submit(&service, &data_root, argv(&["status"])).unwrap();
    assert!(blocking.started.wait(Duration::from_secs(1)));
    let accepted: Vec<Value> = (0..64)
        .map(|_| submit(&service, &data_root, argv(&["status"])).unwrap())
        .collect();
    let full = submit(&service, &data_root, argv(&["status"])).unwrap_err();
    let full = full.app().unwrap();
    assert_eq!(full.code, "operator_queue_full");
    assert!(full.retryable);
    assert!(accepted.iter().all(|item| text(item, "state") == "queued"));
    service.cancel(&id(&accepted[0]));
    let replacement = submit(&service, &data_root, argv(&["status"])).unwrap();
    assert_eq!(text(&replacement, "state"), "queued");
    blocking.release.set();
}

#[test]
fn test_queued_operation_is_cancelled_if_database_identity_changes() {
    let tmp = tempfile::tempdir().unwrap();
    let database = tmp.path().join("kassiber.sqlite3");
    std::fs::write(&database, b"original").unwrap();
    let fake = Fake::new();
    let blocking = Blocking::new(false);
    let service = Closing(service_with(
        &fake,
        Arc::clone(&blocking) as Arc<dyn Runner>,
    ));
    let data_root = root(&tmp);
    unlock(&service, &data_root).unwrap();
    let first = submit(&service, &data_root, argv(&["status"])).unwrap();
    assert!(blocking.started.wait(Duration::from_secs(1)));
    let queued = submit(&service, &data_root, scoped("health")).unwrap();
    std::fs::rename(&database, tmp.path().join("kassiber.old")).unwrap();
    std::fs::write(&database, b"replacement").unwrap();
    blocking.release.set();
    wait_terminal(&service, &id(&first));
    assert_eq!(
        text(&wait_terminal(&service, &id(&queued)), "state"),
        "cancelled"
    );
    assert_eq!(blocking.calls(), ["status"]);
}

#[test]
fn test_queued_admin_authentication_expires_before_dispatch() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let blocking = Blocking::new(true);
    let service = Closing(service_with(
        &fake,
        Arc::clone(&blocking) as Arc<dyn Runner>,
    ));
    let data_root = root(&tmp);
    unlock(&service, &data_root).unwrap();
    let authorization = service.verify_admin(&data_root, b"passphrase").unwrap();
    let first = submit(&service, &data_root, argv(&["status"])).unwrap();
    assert!(blocking.started.wait(Duration::from_secs(1)));
    let queued = service
        .submit(
            &data_root,
            &argv(&["secrets", "verify"]),
            SubmitRequest {
                admin_authorization: Some(&authorization),
                ..SubmitRequest::default()
            },
        )
        .unwrap();
    service
        .state()
        .operations
        .get_mut(&id(&queued))
        .unwrap()
        .admin_authorized_until = Some(Instant::now() - Duration::from_secs(1));
    blocking.release.set();
    wait_terminal(&service, &id(&first));
    assert_eq!(
        text(&wait_terminal(&service, &id(&queued)), "state"),
        "cancelled"
    );
    assert_eq!(blocking.calls(), ["status"]);
}

#[test]
fn test_queued_operation_cannot_cross_a_refreshed_lease_epoch() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = Arc::clone(&calls);
    let runner = Arc::new(move |dispatch: &Dispatch, _: &[u8]| {
        locked(&log).push(dispatch.command_path.clone());
        Ok(OperationResult::new(0, "", ""))
    });
    let service = Closing(service_with(&fake, runner));
    let data_root = root(&tmp);
    unlock_with_paused_worker(&service, &fake, &data_root, b"first-passphrase");
    let (dequeued, release) = pause_dequeued(&service, &identity(&fake, &data_root));
    let queued = submit(&service, &data_root, argv(&["status"])).unwrap();
    assert!(dequeued.wait(Duration::from_secs(1)));
    service
        .unlock(&data_root, b"second-passphrase", UnlockRequest::default())
        .unwrap();
    release.set();
    assert_eq!(
        text(&wait_terminal(&service, &id(&queued)), "state"),
        "cancelled"
    );
    assert_eq!(
        text(&service.status(Some(&data_root)).unwrap(), "lease"),
        "unlocked"
    );
    // The paused worker was stopped; restart one for the replacement.
    {
        let identity = identity(&fake, &data_root);
        let mut state = service.state();
        let worker = service.start_worker(&identity, &identity[..16]);
        state.workers.insert(identity, worker);
    }
    let replacement = submit(&service, &data_root, scoped("health")).unwrap();
    assert_eq!(
        text(&wait_terminal(&service, &id(&replacement)), "state"),
        "completed"
    );
    assert_eq!(*locked(&calls), ["health"]);
}

#[test]
fn test_fresh_auth_never_opens_database_during_same_project_work() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let blocking = Blocking::new(false);
    let service = Closing(service_with(
        &fake,
        Arc::clone(&blocking) as Arc<dyn Runner>,
    ));
    let data_root = root(&tmp);
    unlock(&service, &data_root).unwrap();
    submit(&service, &data_root, argv(&["status"])).unwrap();
    assert!(blocking.started.wait(Duration::from_secs(1)));
    let opened = fake.opened();
    assert_eq!(
        code(service.verify_admin(&data_root, b"passphrase")),
        "operator_project_busy"
    );
    assert_eq!(
        code(service.authenticate_database(
            &data_root,
            b"passphrase",
            "operator_mode",
            false,
            |_| Ok(())
        )),
        "operator_project_busy"
    );
    assert_eq!(fake.opened(), opened);
    blocking.release.set();
}

#[test]
fn test_admin_authentication_is_rechecked_after_owner_inheritance() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let owner = fake.scripted_owner();
    let entered = Arc::new(Event::default());
    let release = Arc::new(Event::default());
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = Arc::clone(&calls);
    let runner = Arc::new(move |dispatch: &Dispatch, _: &[u8]| {
        locked(&log).push(dispatch.command_path.clone());
        Ok(OperationResult::new(0, "", ""))
    });
    let service = Closing(service_with(&fake, runner));
    let data_root = root(&tmp);
    unlock(&service, &data_root).unwrap();
    let authorization = service.verify_admin(&data_root, b"passphrase").unwrap();
    *locked(&owner.duplicate_gate) = Some((Arc::clone(&entered), Arc::clone(&release)));
    let queued = service
        .submit(
            &data_root,
            &argv(&["secrets", "verify"]),
            SubmitRequest {
                admin_authorization: Some(&authorization),
                ..SubmitRequest::default()
            },
        )
        .unwrap();
    assert!(entered.wait(Duration::from_secs(1)));
    service
        .state()
        .operations
        .get_mut(&id(&queued))
        .unwrap()
        .admin_authorized_until = Some(Instant::now() - Duration::from_secs(1));
    release.set();
    assert_eq!(
        text(&wait_terminal(&service, &id(&queued)), "state"),
        "cancelled"
    );
    assert!(locked(&calls).is_empty());
    assert_eq!(owner.child_closes.load(Ordering::SeqCst), 1);
}

fn completed_operation(service: &OperatorService, index: usize, prefix: &str) -> String {
    let mut operation = Operation::new(
        format!("generation.{prefix}.{index}"),
        "project".into(),
        "identity".into(),
        "database-identity".into(),
        "/unused".into(),
        argv(&["status"]),
        "status".into(),
        Capability::Read,
    );
    operation.state = OpState::Completed;
    let id = operation.id.clone();
    service.state().operations.insert(id.clone(), operation);
    id
}

fn queued_operation(service: &OperatorService, id: &str, command: Vec<String>) {
    let operation = Operation::new(
        id.into(),
        "project".into(),
        "identity".into(),
        "database-identity".into(),
        "/unused".into(),
        command,
        "status".into(),
        Capability::Read,
    );
    service.state().operations.insert(id.into(), operation);
}

fn finish(service: &OperatorService, id: &str, result: OperationResult) {
    let mut state = service.state();
    service.finish_operation_locked(&mut state, id, OpState::Completed, Some(result));
}

#[test]
fn test_terminal_transition_prunes_retained_results() {
    let fake = Fake::new();
    let service = Closing(service_with(&fake, ok_runner()));
    let mut last = String::new();
    for index in 0..MAX_RETAINED_RESULTS + 25 {
        last = completed_operation(&service, index, "test");
    }
    service.state().operations.get_mut(&last).unwrap().state = OpState::Queued;
    finish(&service, &last, OperationResult::new(0, "", ""));
    assert_eq!(service.state().operations.len(), MAX_RETAINED_RESULTS);
}

#[test]
fn test_oversized_result_has_a_small_truthful_status_contract() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let oversized = "x".repeat(MAX_JSON_FRAME + 1);
    let runner =
        Arc::new(move |_: &Dispatch, _: &[u8]| Ok(OperationResult::new(0, oversized.clone(), "")));
    let service = Closing(service_with(&fake, runner));
    unlock(&service, &root(&tmp)).unwrap();
    let accepted = submit(&service, &root(&tmp), argv(&["status"])).unwrap();
    let completed = wait_terminal(&service, &id(&accepted));
    assert_eq!(text(&completed, "state"), "completed");
    assert_eq!(
        kassiber_envelope::get(&completed, "exit_code"),
        Some(&int(0u64))
    );
    assert_eq!(
        kassiber_envelope::get(&completed, "output_available"),
        Some(&Value::Bool(false))
    );
    let error = kassiber_envelope::get(&completed, "output_error").unwrap();
    assert_eq!(text(error, "code"), "operator_result_too_large");
    assert!(kassiber_envelope::get(&completed, "stdout").is_none());
    assert!(kassiber_envelope::get(&completed, "stderr").is_none());
    {
        let state = service.state();
        let retained = &state.operations[&id(&accepted)];
        assert_eq!(retained.result, Some(OperationResult::new(0, "", "")));
        assert_eq!(retained.retained_result_bytes, 0);
    }
    let encoded =
        crate::protocol::encode_json_frame(&dict([("ok", Value::Bool(true)), ("data", completed)]))
            .unwrap();
    assert!(encoded.len() < MAX_JSON_FRAME);
}

#[test]
fn test_oversized_result_is_measured_on_the_escaped_length() {
    // Six escaped bytes per character: well under 8 MiB of UTF-8, over the
    // frame once escaped.
    let wide = "\u{e4}".repeat((MAX_JSON_FRAME - OPERATION_STATUS_FRAME_HEADROOM) / 6 + 1);
    assert!(wide.len() < MAX_JSON_FRAME - OPERATION_STATUS_FRAME_HEADROOM);
    assert!(result_exceeds_protocol_frame(&OperationResult::new(
        0, wide, ""
    )));
    // The boundary: {"stderr":"","stdout":"..."} is 25 bytes of structure.
    let limit = MAX_JSON_FRAME - OPERATION_STATUS_FRAME_HEADROOM;
    let fits = "x".repeat(limit - 25);
    assert!(!result_exceeds_protocol_frame(&OperationResult::new(
        0,
        fits.clone(),
        ""
    )));
    assert!(result_exceeds_protocol_frame(&OperationResult::new(
        0,
        fits + "x",
        ""
    )));
}

#[test]
fn test_result_byte_budget_evicts_to_bounded_fingerprint_tombstones() {
    let fake = Fake::new();
    let service = Closing(service_with_limits(
        &fake,
        ok_runner(),
        Limits {
            max_retained_result_bytes: 20,
            ..Limits::default()
        },
    ));
    let sensitive = format!("private-nonsecret-argument-{}", "z".repeat(4096));
    let mut ids = Vec::new();
    for index in 0..3 {
        let id = format!("generation.byte-budget.{index}");
        queued_operation(
            &service,
            &id,
            argv(&["status", &sensitive, &index.to_string()]),
        );
        finish(&service, &id, OperationResult::new(0, "twelve-bytes", ""));
        ids.push(id);
    }
    let state = service.state();
    assert_eq!(
        state.operations.keys().cloned().collect::<Vec<_>>(),
        [ids[2].clone()]
    );
    let tombstone = state.tombstones.get(&ids[0]).unwrap();
    assert_eq!(tombstone.0, "project");
    assert_eq!(tombstone.1.len(), 64);
    assert!(!format!("{:?}", state.tombstones.values().collect::<Vec<_>>()).contains(&sensitive));
}

#[test]
fn test_active_operations_do_not_evict_the_first_completed_result() {
    let fake = Fake::new();
    let service = Closing(service_with(&fake, ok_runner()));
    for index in 0..MAX_RETAINED_RESULTS + 25 {
        queued_operation(
            &service,
            &format!("generation.active.{index}"),
            argv(&["status"]),
        );
    }
    queued_operation(&service, "generation.completed", argv(&["status"]));
    finish(
        &service,
        "generation.completed",
        OperationResult::new(0, "done", ""),
    );
    assert!(service.state().operations.contains("generation.completed"));
    assert_eq!(state_of(&service, "generation.completed"), "completed");
}

#[test]
fn test_terminal_retention_uses_completion_order() {
    let fake = Fake::new();
    let service = Closing(service_with(&fake, ok_runner()));
    let ids: Vec<String> = (0..=MAX_RETAINED_RESULTS)
        .map(|index| {
            let id = format!("generation.completion-order.{index}");
            queued_operation(&service, &id, argv(&["status"]));
            id
        })
        .collect();
    for id in ids.iter().rev() {
        finish(&service, id, OperationResult::new(0, "", ""));
    }
    let state = service.state();
    assert!(!state.operations.contains(ids.last().unwrap()));
    assert!(state.operations.contains(&ids[0]));
    assert_eq!(state.operations.len(), MAX_RETAINED_RESULTS);
}

#[test]
fn test_tombstones_are_bounded_oldest_first() {
    let fake = Fake::new();
    let service = Closing(service_with_limits(
        &fake,
        ok_runner(),
        Limits {
            max_retained_results: 1,
            max_tombstones: 2,
            ..Limits::default()
        },
    ));
    for index in 0..4 {
        let id = format!("generation.tomb.{index}");
        queued_operation(&service, &id, argv(&["status"]));
        finish(&service, &id, OperationResult::new(0, "", ""));
    }
    let state = service.state();
    assert_eq!(
        state.tombstones.keys().cloned().collect::<Vec<_>>(),
        ["generation.tomb.1", "generation.tomb.2"]
    );
    assert_eq!(
        state.operations.keys().cloned().collect::<Vec<_>>(),
        ["generation.tomb.3"]
    );
}

#[test]
fn test_evicted_operation_id_is_not_reexecuted() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = Arc::clone(&calls);
    let runner = Arc::new(move |dispatch: &Dispatch, _: &[u8]| {
        locked(&log).push(dispatch.command_path.clone());
        Ok(OperationResult::new(0, "", ""))
    });
    let service = Closing(service_with_limits(
        &fake,
        runner,
        Limits {
            max_retained_results: 1,
            ..Limits::default()
        },
    ));
    let data_root = root(&tmp);
    unlock(&service, &data_root).unwrap();
    let first = submit(&service, &data_root, argv(&["status"])).unwrap();
    wait_terminal(&service, &id(&first));
    let second = submit(&service, &data_root, scoped("health")).unwrap();
    wait_terminal(&service, &id(&second));
    let replay = service
        .submit(
            &data_root,
            &argv(&["status"]),
            SubmitRequest {
                operation_id: Some(id(&first)),
                ..SubmitRequest::default()
            },
        )
        .unwrap();
    assert_eq!(text(&replay, "state"), "result_unknown");
    assert_eq!(text(&replay, "reason"), "result_not_retained");
    // A tombstoned id with another request is a conflict, not a replay.
    assert_eq!(
        code(service.submit(
            &data_root,
            &scoped("health"),
            SubmitRequest {
                operation_id: Some(id(&first)),
                ..SubmitRequest::default()
            },
        )),
        "operator_operation_id_conflict"
    );
    assert_eq!(*locked(&calls), ["status", "health"]);
}

#[test]
fn test_worker_crash_marks_result_unknown() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let runner = Arc::new(|_: &Dispatch, _: &[u8]| -> Result<OperationResult> {
        Err(Error::Value(
            "simulated crash after unknown commit point".into(),
        ))
    });
    let service = Closing(service_with(&fake, runner));
    unlock(&service, &root(&tmp)).unwrap();
    let accepted = submit(&service, &root(&tmp), argv(&["status"])).unwrap();
    assert_eq!(
        text(&wait_terminal(&service, &id(&accepted)), "state"),
        "result_unknown"
    );
}

#[test]
fn test_runner_panic_marks_result_unknown() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let runner = Arc::new(|_: &Dispatch, _: &[u8]| -> Result<OperationResult> {
        panic!("simulated runner panic")
    });
    let service = Closing(service_with(&fake, runner));
    unlock(&service, &root(&tmp)).unwrap();
    let accepted = submit(&service, &root(&tmp), argv(&["status"])).unwrap();
    let status = wait_terminal(&service, &id(&accepted));
    assert_eq!(text(&status, "state"), "result_unknown");
    assert_eq!(
        text(&status, "stderr"),
        "operator child failed: the runner panicked\n"
    );
    // The worker survives for the next operation.
    let next = submit(&service, &root(&tmp), argv(&["status"])).unwrap();
    assert_eq!(
        text(&wait_terminal(&service, &id(&next)), "state"),
        "result_unknown"
    );
}

#[test]
fn test_owner_inheritance_failure_before_launch_is_failed() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let owner = fake.scripted_owner();
    locked(&owner.duplicate_failures).push_back("passphrase=pre-launch-secret".into());
    let (seen, runner) = recording_runner();
    let service = Closing(service_with(&fake, runner));
    unlock(&service, &root(&tmp)).unwrap();
    let accepted = submit(&service, &root(&tmp), argv(&["status"])).unwrap();
    let terminal = wait_terminal(&service, &id(&accepted));
    assert_eq!(text(&terminal, "state"), "failed");
    assert_eq!(
        kassiber_envelope::get(&terminal, "exit_code"),
        Some(&int(1u64))
    );
    assert!(!text(&terminal, "stderr").contains("pre-launch-secret"));
    assert!(locked(&seen).is_empty());
}

#[test]
fn test_owner_cleanup_failure_preserves_result_and_revokes_lease() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let owner = fake.scripted_owner();
    locked(&owner.child_close_failures).push_back("passphrase=cleanup-secret".into());
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = Arc::clone(&calls);
    let runner = Arc::new(move |dispatch: &Dispatch, _: &[u8]| {
        locked(&log).push(dispatch.command_path.clone());
        Ok(OperationResult::new(0, "", ""))
    });
    let service = Closing(service_with(&fake, runner));
    let data_root = root(&tmp);
    unlock(&service, &data_root).unwrap();
    let first = submit(&service, &data_root, argv(&["status"])).unwrap();
    let first_status = wait_terminal(&service, &id(&first));
    assert_eq!(text(&first_status, "state"), "completed");
    assert!(!text(&first_status, "stderr").contains("cleanup-secret"));
    let deadline = Instant::now() + Duration::from_secs(1);
    while text(&service.status(Some(&data_root)).unwrap(), "lease") != "locked"
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        text(&service.status(Some(&data_root)).unwrap(), "lease"),
        "locked"
    );
    unlock(&service, &data_root).unwrap();
    let second = submit(&service, &data_root, scoped("health")).unwrap();
    assert_eq!(
        text(&wait_terminal(&service, &id(&second)), "state"),
        "completed"
    );
    assert_eq!(*locked(&calls), ["status", "health"]);
    service.lock(&data_root, None, false).unwrap();
}

#[test]
fn test_transient_owner_release_logs_preserved_terminal_state() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let owner = fake.scripted_owner();
    locked(&owner.release_failures).push_back("transient close failure".into());
    let ring = Arc::new(LogRing::new(100));
    let blocking = Blocking::new(false);
    let service = Closing(OperatorService::with_options(
        "generation",
        Arc::clone(&fake) as Arc<dyn Backend>,
        Arc::clone(&blocking) as Arc<dyn Runner>,
        Arc::clone(&ring) as Arc<dyn Telemetry>,
        Limits::default(),
    ));
    let data_root = root(&tmp);
    unlock(&service, &data_root).unwrap();
    let accepted = submit(&service, &data_root, argv(&["status"])).unwrap();
    assert!(blocking.started.wait(Duration::from_secs(1)));
    service.lock(&data_root, None, false).unwrap();
    blocking.release.set();
    assert_eq!(
        text(&wait_terminal(&service, &id(&accepted)), "state"),
        "completed"
    );
    let deadline = Instant::now() + Duration::from_secs(1);
    let recovered = loop {
        let found = ring
            .snapshot()
            .into_iter()
            .find(|record| record.message == "operator worker recovered from an internal failure");
        if let Some(found) = found {
            break found;
        }
        assert!(Instant::now() < deadline, "no recovery record");
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(recovered
        .fields
        .contains(&("state".to_owned(), s("completed"))));
    let deadline = Instant::now() + Duration::from_secs(1);
    while owner.releases.load(Ordering::SeqCst) < 2 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(owner.releases.load(Ordering::SeqCst), 2);
}

#[test]
fn test_runner_exception_is_secret_floor_redacted() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let runner = Arc::new(|_: &Dispatch, _: &[u8]| -> Result<OperationResult> {
        Err(Error::Value(
            "blinding_key=private-value passphrase=hunter2".into(),
        ))
    });
    let service = Closing(service_with(&fake, runner));
    unlock(&service, &root(&tmp)).unwrap();
    let accepted = submit(&service, &root(&tmp), argv(&["status"])).unwrap();
    let status = wait_terminal(&service, &id(&accepted));
    assert_eq!(text(&status, "state"), "result_unknown");
    assert!(!text(&status, "stderr").contains("private-value"));
    assert!(!text(&status, "stderr").contains("hunter2"));
}

#[test]
fn test_completed_child_stderr_is_secret_floor_redacted() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let runner = Arc::new(|_: &Dispatch, _: &[u8]| {
        Ok(OperationResult::new(
            1,
            "",
            "warning {\"blinding_key\":\"private-value\"} token=btcpay-secret\n",
        ))
    });
    let service = Closing(service_with(&fake, runner));
    unlock(&service, &root(&tmp)).unwrap();
    let accepted = submit(&service, &root(&tmp), argv(&["status"])).unwrap();
    let status = wait_terminal(&service, &id(&accepted));
    assert_eq!(text(&status, "state"), "failed");
    let stderr = text(&status, "stderr");
    assert!(!stderr.contains("private-value"));
    assert!(!stderr.contains("btcpay-secret"));
    assert!(stderr.contains("[redacted]"));
}

#[test]
fn test_unproven_nonzero_mutation_is_unknown_but_read_failure_is_failed() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let runner =
        Arc::new(|_: &Dispatch, _: &[u8]| Ok(OperationResult::new(1, "", "child exited\n")));
    let service = Closing(service_with(&fake, runner));
    let data_root = root(&tmp);
    unlock(&service, &data_root).unwrap();
    let read = submit(&service, &data_root, argv(&["status"])).unwrap();
    let mutation = submit(
        &service,
        &data_root,
        argv(&[
            "journals",
            "process",
            "--workspace",
            "workspace-a",
            "--profile",
            "book-a",
        ]),
    )
    .unwrap();
    assert_eq!(
        text(&wait_terminal(&service, &id(&read)), "state"),
        "failed"
    );
    assert_eq!(
        text(&wait_terminal(&service, &id(&mutation)), "state"),
        "result_unknown"
    );
}

#[test]
fn test_signalled_read_child_is_result_unknown() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let runner = Arc::new(|_: &Dispatch, _: &[u8]| Ok(OperationResult::new(-9, "", "")));
    let service = Closing(service_with(&fake, runner));
    unlock(&service, &root(&tmp)).unwrap();
    let read = submit(&service, &root(&tmp), argv(&["status"])).unwrap();
    assert_eq!(
        text(&wait_terminal(&service, &id(&read)), "state"),
        "result_unknown"
    );
}

fn admin_submit(
    service: &OperatorService,
    data_root: &str,
    command: Vec<String>,
    secrets: Vec<(String, Zeroizing<Vec<u8>>)>,
) -> Value {
    let authorization = service.verify_admin(data_root, b"passphrase").unwrap();
    service
        .submit(
            data_root,
            &command,
            SubmitRequest {
                secret_arguments: secrets,
                admin_authorization: Some(&authorization),
                ..SubmitRequest::default()
            },
        )
        .unwrap()
}

#[test]
fn test_successful_passphrase_rotation_revokes_the_lease() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let service = Closing(service_with(&fake, ok_runner()));
    let data_root = root(&tmp);
    unlock(&service, &data_root).unwrap();
    let accepted = admin_submit(
        &service,
        &data_root,
        argv(&[
            "secrets",
            "change-passphrase",
            "--new-passphrase-fd",
            "broker-secret-test",
        ]),
        vec![(
            "broker-secret-test".into(),
            Zeroizing::new(b"new-passphrase".to_vec()),
        )],
    );
    assert_eq!(
        text(&wait_terminal(&service, &id(&accepted)), "state"),
        "completed"
    );
    assert_eq!(
        text(&service.status(Some(&data_root)).unwrap(), "lease"),
        "locked"
    );
}

#[test]
fn test_credential_mode_commands_revoke_lease_after_success() {
    for command in ["remember-unlock", "forget-unlock"] {
        let tmp = tempfile::tempdir().unwrap();
        let fake = Fake::new();
        let service = Closing(service_with(&fake, ok_runner()));
        let data_root = root(&tmp);
        unlock(&service, &data_root).unwrap();
        let accepted = admin_submit(&service, &data_root, argv(&["secrets", command]), vec![]);
        assert_eq!(
            text(&wait_terminal(&service, &id(&accepted)), "state"),
            "completed"
        );
        assert_eq!(
            text(&service.status(Some(&data_root)).unwrap(), "lease"),
            "locked"
        );
        assert!(service.state().leases.is_empty());
    }
}

#[test]
fn test_credential_mode_partial_outcomes_revoke_and_drain_lease() {
    for command in ["remember-unlock", "forget-unlock"] {
        let tmp = tempfile::tempdir().unwrap();
        let fake = Fake::new();
        let started = Arc::new(Event::default());
        let release = Arc::new(Event::default());
        let (entered, go) = (Arc::clone(&started), Arc::clone(&release));
        let expected = format!("secrets.{command}");
        let runner = Arc::new(move |dispatch: &Dispatch, _: &[u8]| {
            if dispatch.command_path == expected {
                entered.set();
                assert!(go.wait(Duration::from_secs(2)));
                return Ok(OperationResult::new(1, "", "partial credential update\n"));
            }
            Ok(OperationResult::new(0, "", ""))
        });
        let service = Closing(service_with(&fake, runner));
        let data_root = root(&tmp);
        unlock(&service, &data_root).unwrap();
        let credential = admin_submit(&service, &data_root, argv(&["secrets", command]), vec![]);
        assert!(started.wait(Duration::from_secs(1)));
        let queued = submit(&service, &data_root, argv(&["status"])).unwrap();
        release.set();
        assert_eq!(
            text(&wait_terminal(&service, &id(&credential)), "state"),
            "result_unknown"
        );
        assert_eq!(
            text(&wait_terminal(&service, &id(&queued)), "state"),
            "cancelled"
        );
        assert_eq!(
            text(&service.status(Some(&data_root)).unwrap(), "lease"),
            "locked"
        );
    }
}

#[test]
fn test_cross_project_backup_install_is_rejected_before_admission() {
    let source = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let (seen, runner) = recording_runner();
    let service = Closing(service_with(&fake, runner));
    unlock(&service, &root(&source)).unwrap();
    let result = service.submit(
        &root(&source),
        &argv(&[
            "backup",
            "import",
            "archive.kassiber",
            "--install",
            "--target-data-root",
            &root(&target),
            "--backup-passphrase-fd",
            "broker-secret-backup",
        ]),
        SubmitRequest {
            secret_arguments: vec![(
                "broker-secret-backup".into(),
                Zeroizing::new(b"backup-passphrase".to_vec()),
            )],
            ..SubmitRequest::default()
        },
    );
    assert_eq!(code(result), "operator_command_not_brokerable");
    assert!(locked(&seen).is_empty());
    assert!(service.state().operations.is_empty());
}

#[test]
fn test_same_project_backup_install_requires_manual_locked_workflow() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let (seen, runner) = recording_runner();
    let service = Closing(service_with(&fake, runner));
    unlock(&service, &root(&tmp)).unwrap();
    let error = submit(
        &service,
        &root(&tmp),
        argv(&["backup", "import", "archive.kassiber", "--install"]),
    )
    .unwrap_err();
    let error = error.app().unwrap();
    assert_eq!(error.code, "operator_command_not_brokerable");
    assert_eq!(
        error.hint.as_deref(),
        Some("Lock the broker lease, select manual mode, and run the restore explicitly for its destination.")
    );
    assert!(locked(&seen).is_empty());
    assert!(service.state().operations.is_empty());
    assert_eq!(
        text(&service.status(Some(&root(&tmp))).unwrap(), "lease"),
        "unlocked"
    );
}

#[cfg(unix)]
#[test]
fn test_queued_child_is_pinned_to_canonical_root_not_caller_alias() {
    let source = tempfile::tempdir().unwrap();
    let replacement = tempfile::tempdir().unwrap();
    let parent = tempfile::tempdir().unwrap();
    let alias = parent.path().join("project");
    std::os::unix::fs::symlink(source.path(), &alias).unwrap();
    let alias_text = alias.to_string_lossy().into_owned();
    let fake = Fake::new();
    let seen = Arc::new(Mutex::new(Vec::<(String, Vec<String>)>::new()));
    let started = Arc::new(Event::default());
    let release = Arc::new(Event::default());
    let (log, entered, go) = (
        Arc::clone(&seen),
        Arc::clone(&started),
        Arc::clone(&release),
    );
    let runner = Arc::new(move |dispatch: &Dispatch, _: &[u8]| {
        let first = {
            let mut log = locked(&log);
            log.push((dispatch.data_root.clone(), dispatch.argv.clone()));
            log.len() == 1
        };
        if first {
            entered.set();
            go.wait(Duration::from_secs(2));
        }
        Ok(OperationResult::new(0, "", ""))
    });
    let service = Closing(service_with(&fake, runner));
    unlock(&service, &alias_text).unwrap();
    let first = submit(&service, &alias_text, argv(&["status"])).unwrap();
    assert!(started.wait(Duration::from_secs(1)));
    let second = submit(&service, &alias_text, argv(&["status"])).unwrap();
    let canonical = std::fs::canonicalize(source.path())
        .unwrap()
        .to_string_lossy()
        .into_owned();
    {
        let state = service.state();
        let queued = &state.operations[&id(&second)];
        assert_eq!(queued.data_root, canonical);
        assert_eq!(queued.argv[..2], argv(&["--data-root", &canonical]));
    }
    std::fs::remove_file(&alias).unwrap();
    std::os::unix::fs::symlink(replacement.path(), &alias).unwrap();
    release.set();
    assert_eq!(
        text(&wait_terminal(&service, &id(&first)), "state"),
        "completed"
    );
    assert_eq!(
        text(&wait_terminal(&service, &id(&second)), "state"),
        "completed"
    );
    for (data_root, command) in locked(&seen).iter() {
        assert_eq!(*data_root, canonical);
        assert_eq!(command[..2], argv(&["--data-root", &canonical]));
    }
}

#[test]
fn test_explicit_scope_is_pinned_at_admission() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    fake.open_with(|_, _, _| {
        Ok(Box::new(FakeDb(Arc::new(DbScript {
            identity: Some("d".repeat(32)),
            context: (Some("workspace-a".into()), Some("book-a".into())),
            ..DbScript::default()
        }))))
    });
    let seen = Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
    let started = Arc::new(Event::default());
    let release = Arc::new(Event::default());
    let (log, entered, go) = (
        Arc::clone(&seen),
        Arc::clone(&started),
        Arc::clone(&release),
    );
    let runner = Arc::new(move |dispatch: &Dispatch, _: &[u8]| {
        let first = {
            let mut log = locked(&log);
            log.push(dispatch.argv.clone());
            log.len() == 1
        };
        if first {
            entered.set();
            go.wait(Duration::from_secs(2));
        }
        Ok(OperationResult::new(0, "", ""))
    });
    let service = Closing(service_with(&fake, runner));
    let data_root = root(&tmp);
    unlock(&service, &data_root).unwrap();
    let first = submit(&service, &data_root, argv(&["status"])).unwrap();
    assert!(started.wait(Duration::from_secs(1)));
    let queued = submit(
        &service,
        &data_root,
        argv(&[
            "transactions",
            "list",
            "--workspace",
            "workspace-a",
            "--profile",
            "book-a",
        ]),
    )
    .unwrap();
    with_lease(&service, &fake, &data_root, |lease| {
        lease.workspace = Some("workspace-b".into());
        lease.profile = Some("book-b".into());
    });
    release.set();
    wait_terminal(&service, &id(&first));
    wait_terminal(&service, &id(&queued));
    let canonical = std::fs::canonicalize(tmp.path())
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        locked(&seen)[1][..8],
        argv(&[
            "--data-root",
            &canonical,
            "transactions",
            "list",
            "--workspace",
            "workspace-a",
            "--profile",
            "book-a"
        ])
    );
}

#[test]
fn test_scoped_command_without_explicit_book_fails_closed() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let service = Closing(service_with(&fake, ok_runner()));
    unlock(&service, &root(&tmp)).unwrap();
    let error = submit(&service, &root(&tmp), argv(&["transactions", "list"])).unwrap_err();
    let error = error.app().unwrap();
    assert_eq!(error.code, "operator_scope_required");
    let details = error.details.as_ref().unwrap();
    assert_eq!(
        kassiber_envelope::get(details, "missing"),
        Some(&Value::List(vec![s("workspace"), s("profile")]))
    );
}

#[test]
fn test_context_set_needs_one_scope_flag() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let service = Closing(service_with(&fake, ok_runner()));
    unlock(&service, &root(&tmp)).unwrap();
    let error = submit(&service, &root(&tmp), argv(&["context", "set"])).unwrap_err();
    let details = error.app().unwrap().details.clone().unwrap();
    assert_eq!(
        kassiber_envelope::get(&details, "missing"),
        Some(&Value::List(vec![s("workspace_or_profile")]))
    );
    submit(
        &service,
        &root(&tmp),
        argv(&["context", "set", "--profile", "book-a"]),
    )
    .unwrap();
}

#[test]
fn test_missing_lease_precedes_scope_validation() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let service = Closing(service_with(&fake, ok_runner()));
    assert_eq!(
        code(submit(
            &service,
            &root(&tmp),
            argv(&["transactions", "list"])
        )),
        "interaction_required"
    );
}

#[test]
fn test_context_workspace_change_does_not_inject_the_old_profile() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let seen = Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
    let log = Arc::clone(&seen);
    let runner = Arc::new(move |dispatch: &Dispatch, _: &[u8]| {
        locked(&log).push(dispatch.argv.clone());
        Ok(OperationResult::new(1, "", "validation stop\n"))
    });
    let service = Closing(service_with(&fake, runner));
    let data_root = root(&tmp);
    unlock(&service, &data_root).unwrap();
    with_lease(&service, &fake, &data_root, |lease| {
        lease.workspace = Some("workspace-a".into());
        lease.profile = Some("book-a".into());
    });
    let accepted = submit(
        &service,
        &data_root,
        argv(&["context", "set", "--workspace", "workspace-b"]),
    )
    .unwrap();
    wait_terminal(&service, &id(&accepted));
    let seen = locked(&seen)[0].clone();
    assert!(seen.contains(&"workspace-b".to_owned()));
    assert!(!seen.contains(&"--profile".to_owned()));
    assert!(!seen.contains(&"book-a".to_owned()));
}

#[test]
fn test_completed_context_set_refreshes_the_lease_scope() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let service = Closing(service_with(&fake, ok_runner()));
    let data_root = root(&tmp);
    unlock(&service, &data_root).unwrap();
    fake.open_with(|_, _, expected| {
        assert!(expected.is_some());
        Ok(Box::new(FakeDb(Arc::new(DbScript {
            context: (Some("workspace-b".into()), None),
            ..DbScript::default()
        }))))
    });
    let accepted = submit(
        &service,
        &data_root,
        argv(&["context", "set", "--workspace", "workspace-b"]),
    )
    .unwrap();
    wait_terminal(&service, &id(&accepted));
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let status = service.status(Some(&data_root)).unwrap();
        let scope = kassiber_envelope::get(&status, "default_scope")
            .unwrap()
            .clone();
        if kassiber_envelope::get(&scope, "workspace") == Some(&s("workspace-b")) {
            assert_eq!(
                kassiber_envelope::get(&scope, "profile"),
                Some(&Value::Null)
            );
            break;
        }
        assert!(Instant::now() < deadline, "scope was not refreshed");
        std::thread::sleep(Duration::from_millis(5));
    }
    // A book that cannot be read ends the lease rather than keep a stale scope.
    fake.open_with(|_, _, _| {
        Err(Error::App(AppError::new(
            "unlock_failed",
            "wrong passphrase",
        )))
    });
    let accepted = submit(
        &service,
        &data_root,
        argv(&["context", "set", "--workspace", "workspace-c"]),
    )
    .unwrap();
    wait_terminal(&service, &id(&accepted));
    let deadline = Instant::now() + Duration::from_secs(1);
    while text(&service.status(Some(&data_root)).unwrap(), "lease") != "locked" {
        assert!(
            Instant::now() < deadline,
            "the lease survived a failed refresh"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Opens block for one project root until released.
fn blocking_opener(fake: &Fake, blocked_root: &str) -> (Arc<Event>, Arc<Event>) {
    let entered = Arc::new(Event::default());
    let release = Arc::new(Event::default());
    let blocked = std::fs::canonicalize(blocked_root).unwrap();
    let (signal, go) = (Arc::clone(&entered), Arc::clone(&release));
    fake.open_with(move |data_root, _, _| {
        if std::fs::canonicalize(data_root).ok().as_ref() == Some(&blocked) {
            signal.set();
            assert!(
                go.wait(Duration::from_secs(2)),
                "timed out releasing a blocked open"
            );
        }
        Ok(Box::new(FakeDb(Arc::new(DbScript::default()))))
    });
    (entered, release)
}

#[test]
fn test_blocked_unlock_does_not_block_another_project() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    *locked(&fake.fixed_mode) = Some("brokered".into());
    let service = Closing(service_with(&fake, ok_runner()));
    service
        .unlock(&root(&second), b"second", UnlockRequest::default())
        .unwrap();
    let (entered, release) = blocking_opener(&fake, &root(&first));
    let unlocker = {
        let (service, data_root) = (service.clone(), root(&first));
        std::thread::spawn(move || {
            service
                .unlock(&data_root, b"first", UnlockRequest::default())
                .map(|_| ())
        })
    };
    assert!(entered.wait(Duration::from_secs(1)));
    let lock_finished = Arc::new(Event::default());
    let locker = {
        let (service, data_root, finished) =
            (service.clone(), root(&first), Arc::clone(&lock_finished));
        std::thread::spawn(move || {
            let result = service.lock(&data_root, None, false).map(|_| ());
            finished.set();
            result
        })
    };
    assert!(!lock_finished.wait(Duration::from_millis(50)));
    assert_eq!(
        text(&service.status(Some(&root(&second))).unwrap(), "lease"),
        "unlocked"
    );
    let accepted = submit(&service, &root(&second), argv(&["status"])).unwrap();
    assert_eq!(
        text(&wait_terminal(&service, &id(&accepted)), "state"),
        "completed"
    );
    release.set();
    unlocker.join().unwrap().unwrap();
    locker.join().unwrap().unwrap();
    assert!(lock_finished.is_set());
}

#[test]
fn test_blocked_fresh_auth_does_not_block_another_project() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    *locked(&fake.fixed_mode) = Some("brokered".into());
    let service = Closing(service_with(&fake, ok_runner()));
    service
        .unlock(&root(&first), b"first", UnlockRequest::default())
        .unwrap();
    service
        .unlock(&root(&second), b"second", UnlockRequest::default())
        .unwrap();
    let (entered, release) = blocking_opener(&fake, &root(&first));
    let authenticating = {
        let (service, data_root) = (service.clone(), root(&first));
        std::thread::spawn(move || {
            service.authenticate_database(&data_root, b"first", "operator_admin", true, |_| Ok(()))
        })
    };
    assert!(entered.wait(Duration::from_secs(1)));
    assert_eq!(
        text(&service.status(Some(&root(&second))).unwrap(), "lease"),
        "unlocked"
    );
    let accepted = submit(&service, &root(&second), argv(&["status"])).unwrap();
    assert_eq!(
        text(&wait_terminal(&service, &id(&accepted)), "state"),
        "completed"
    );
    release.set();
    authenticating.join().unwrap().unwrap();
}

#[test]
fn test_blocked_authenticated_continuation_does_not_block_other_project() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    *locked(&fake.fixed_mode) = Some("brokered".into());
    let service = Closing(service_with(&fake, ok_runner()));
    service
        .unlock(&root(&first), b"first", UnlockRequest::default())
        .unwrap();
    service
        .unlock(&root(&second), b"second", UnlockRequest::default())
        .unwrap();
    let entered = Arc::new(Event::default());
    let release = Arc::new(Event::default());
    let authenticating = {
        let (service, data_root) = (service.clone(), root(&first));
        let (signal, go) = (Arc::clone(&entered), Arc::clone(&release));
        std::thread::spawn(move || {
            service.authenticate_database(
                &data_root,
                b"first",
                "operator_native_enrollment",
                true,
                |_| {
                    signal.set();
                    assert!(go.wait(Duration::from_secs(2)));
                    Ok(())
                },
            )
        })
    };
    assert!(entered.wait(Duration::from_secs(1)));
    assert_eq!(
        text(&service.status(Some(&root(&second))).unwrap(), "lease"),
        "unlocked"
    );
    let accepted = submit(&service, &root(&second), argv(&["status"])).unwrap();
    assert_eq!(
        text(&wait_terminal(&service, &id(&accepted)), "state"),
        "completed"
    );
    release.set();
    authenticating.join().unwrap().unwrap();
}

#[test]
fn test_close_waits_for_an_authenticated_continuation() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    *locked(&fake.fixed_mode) = Some("brokered".into());
    let service = service_with(&fake, ok_runner());
    unlock(&service, &root(&tmp)).unwrap();
    let entered = Arc::new(Event::default());
    let release = Arc::new(Event::default());
    let authenticating = {
        let (service, data_root) = (service.clone(), root(&tmp));
        let (signal, go) = (Arc::clone(&entered), Arc::clone(&release));
        std::thread::spawn(move || {
            service.authenticate_database(
                &data_root,
                b"passphrase",
                "operator_native_enrollment",
                true,
                |_| {
                    signal.set();
                    assert!(go.wait(Duration::from_secs(2)));
                    Ok(())
                },
            )
        })
    };
    assert!(entered.wait(Duration::from_secs(1)));
    let close_finished = Arc::new(Event::default());
    let closer = {
        let (service, finished) = (service.clone(), Arc::clone(&close_finished));
        std::thread::spawn(move || {
            let result = service.close();
            finished.set();
            result
        })
    };
    assert!(!close_finished.wait(Duration::from_millis(50)));
    release.set();
    authenticating.join().unwrap().unwrap();
    closer.join().unwrap().unwrap();
    assert!(close_finished.is_set());
    assert!(service.state().leases.is_empty());
    assert!(service.state().workers.is_empty());
    service.close().unwrap();
}

#[test]
fn test_close_during_unlock_prevents_lease_installation() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    *locked(&fake.fixed_mode) = Some("brokered".into());
    let (entered, release) = blocking_opener(&fake, &root(&tmp));
    let service = service_with(&fake, ok_runner());
    let unlocker = {
        let (service, data_root) = (service.clone(), root(&tmp));
        std::thread::spawn(move || unlock(&service, &data_root).map(|_| ()))
    };
    assert!(entered.wait(Duration::from_secs(1)));
    let close_finished = Arc::new(Event::default());
    let closer = {
        let (service, finished) = (service.clone(), Arc::clone(&close_finished));
        std::thread::spawn(move || {
            service.close().unwrap();
            finished.set();
        })
    };
    assert!(!close_finished.wait(Duration::from_millis(50)));
    release.set();
    let result = unlocker.join().unwrap();
    closer.join().unwrap();
    assert!(close_finished.is_set());
    assert_eq!(code(result), "operator_broker_stopped");
    assert!(service.state().leases.is_empty());
    assert!(service.state().workers.is_empty());
    service.close().unwrap();
}

#[test]
fn test_close_allows_a_running_atomic_operation_to_finish() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let started = Arc::new(Event::default());
    let release = Arc::new(Event::default());
    let (entered, go) = (Arc::clone(&started), Arc::clone(&release));
    let runner = Arc::new(move |_: &Dispatch, _: &[u8]| {
        entered.set();
        assert!(go.wait(Duration::from_secs(2)));
        Ok(OperationResult::new(0, "finished\n", ""))
    });
    let service = service_with(&fake, runner);
    unlock(&service, &root(&tmp)).unwrap();
    let accepted = submit(&service, &root(&tmp), argv(&["status"])).unwrap();
    assert!(started.wait(Duration::from_secs(1)));
    service.close().unwrap();
    assert_eq!(state_of(&service, &id(&accepted)), "running");
    release.set();
    let terminal = wait_terminal(&service, &id(&accepted));
    assert_eq!(text(&terminal, "state"), "completed");
    assert_eq!(text(&terminal, "stdout"), "finished\n");
    let deadline = Instant::now() + Duration::from_secs(1);
    while !service.state().leases.is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(service.state().leases.is_empty());
    service.close().unwrap();
}

#[test]
fn test_close_retries_failed_owner_release_without_repreparing() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let owner = fake.scripted_owner();
    locked(&owner.release_failures).push_back("transient release failure".into());
    let service = service_with(&fake, ok_runner());
    unlock(&service, &root(&tmp)).unwrap();
    let error = service.close().unwrap_err();
    assert_eq!(error.to_string(), "transient release failure");
    assert_eq!(owner.releases.load(Ordering::SeqCst), 1);
    assert!(service.state().close_prepared);
    service.close().unwrap();
    assert_eq!(owner.releases.load(Ordering::SeqCst), 2);
    assert!(service.state().pending_owner_releases.is_empty());
    // Complete: a third close does nothing.
    service.close().unwrap();
    assert_eq!(owner.releases.load(Ordering::SeqCst), 2);
}

#[test]
fn test_concurrent_close_waiter_retries_failed_owner_release() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let owner = fake.scripted_owner();
    locked(&owner.release_failures).push_back("transient release failure".into());
    let service = service_with(&fake, ok_runner());
    unlock(&service, &root(&tmp)).unwrap();
    let closers: Vec<_> = (0..2)
        .map(|_| {
            let service = service.clone();
            std::thread::spawn(move || service.close())
        })
        .collect();
    let results: Vec<Result<()>> = closers
        .into_iter()
        .map(|closer| closer.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);
    assert_eq!(owner.releases.load(Ordering::SeqCst), 2);
    assert!(service.state().pending_owner_releases.is_empty());
}

#[test]
fn test_blocked_scope_refresh_does_not_block_another_project() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    *locked(&fake.fixed_mode) = Some("brokered".into());
    let service = Closing(service_with(&fake, ok_runner()));
    service
        .unlock(&root(&first), b"first", UnlockRequest::default())
        .unwrap();
    service
        .unlock(&root(&second), b"second", UnlockRequest::default())
        .unwrap();
    let (entered, release) = blocking_opener(&fake, &root(&first));
    submit(
        &service,
        &root(&first),
        argv(&["context", "set", "--workspace", "workspace-a"]),
    )
    .unwrap();
    assert!(entered.wait(Duration::from_secs(1)));
    assert_eq!(
        text(&service.status(Some(&root(&second))).unwrap(), "lease"),
        "unlocked"
    );
    let accepted = submit(&service, &root(&second), argv(&["status"])).unwrap();
    assert_eq!(
        text(&wait_terminal(&service, &id(&accepted)), "state"),
        "completed"
    );
    release.set();
    service.status(Some(&root(&first))).unwrap();
}

#[test]
fn test_authentication_attempts_are_serialized_per_project() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let active = Arc::new(AtomicUsize::new(0));
    let most = Arc::new(AtomicUsize::new(0));
    let (now, peak) = (Arc::clone(&active), Arc::clone(&most));
    fake.open_with(move |_, passphrase, _| {
        if passphrase == "correct" {
            return Ok(Box::new(FakeDb(Arc::new(DbScript::default()))));
        }
        let current = now.fetch_add(1, Ordering::SeqCst) + 1;
        peak.fetch_max(current, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(50));
        now.fetch_sub(1, Ordering::SeqCst);
        Err(Error::App(AppError::new("unlock_failed", "wrong")))
    });
    let service = Closing(service_with(&fake, ok_runner()));
    service
        .unlock(&root(&tmp), b"correct", UnlockRequest::default())
        .unwrap();
    let barrier = Arc::new(Barrier::new(3));
    let attempts: Vec<_> = (0..2)
        .map(|_| {
            let (service, data_root, barrier) = (service.clone(), root(&tmp), Arc::clone(&barrier));
            std::thread::spawn(move || {
                barrier.wait();
                code(service.authenticate_database(&data_root, b"wrong", "test", true, |_| Ok(())))
            })
        })
        .collect();
    barrier.wait();
    let codes: Vec<String> = attempts
        .into_iter()
        .map(|attempt| attempt.join().unwrap())
        .collect();
    assert_eq!(codes, ["unlock_failed", "unlock_failed"]);
    assert_eq!(most.load(Ordering::SeqCst), 1);
}

// ----- beyond the Python suite: pinned contracts -----

#[test]
fn test_lease_status_names_capabilities_and_scope() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let service = Closing(service_with(&fake, ok_runner()));
    let status = service
        .unlock(
            &root(&tmp),
            b"passphrase",
            UnlockRequest {
                duration_seconds: Some(3600),
                capability: Capability::Operator,
                ..UnlockRequest::default()
            },
        )
        .unwrap();
    assert_eq!(text(&status, "lease"), "unlocked");
    assert_eq!(
        kassiber_envelope::get(&status, "granted_capabilities"),
        Some(&Value::List(vec![s("read"), s("operator")]))
    );
    assert_eq!(
        kassiber_envelope::get(&status, "duration_seconds"),
        Some(&int(3600u64))
    );
    assert_eq!(
        kassiber_envelope::get(&status, "until_lock"),
        Some(&Value::Bool(false))
    );
    assert_eq!(
        kassiber_envelope::get(&status, "previous_mode"),
        Some(&s("manual"))
    );
    assert_eq!(
        kassiber_envelope::get(&status, "agent_scope"),
        Some(&Value::Bool(false))
    );
    let remaining = kassiber_envelope::get(&status, "remaining_seconds").unwrap();
    assert!(
        matches!(remaining, Value::Int(n) if *n == BigInt::from(3599) || *n == BigInt::from(3600))
    );
    assert_eq!(text(&status, "lease_id").len(), 24);
    assert_eq!(
        locked(&fake.modes).values().cloned().collect::<Vec<_>>(),
        ["brokered"]
    );
}

#[test]
fn test_lease_durations_have_a_floor_and_the_timestamp_range() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let service = Closing(service_with(&fake, ok_runner()));
    let with = |seconds| {
        service.unlock(
            &root(&tmp),
            b"passphrase",
            UnlockRequest {
                duration_seconds: Some(seconds),
                ..UnlockRequest::default()
            },
        )
    };
    let error = with(59).unwrap_err();
    assert_eq!(
        error.to_string(),
        "operator lease duration must be at least 1 minute"
    );
    let error = with(i64::MAX).unwrap_err();
    assert_eq!(
        error.to_string(),
        "operator lease duration exceeds the timestamp range"
    );
    assert_eq!(
        code(with(crate::clock::MAX_UNIX_SECONDS - unix_seconds() + 10)),
        "operator_invalid_duration"
    );
    with(60).unwrap();
}

#[test]
fn test_lease_expires_by_duration_and_status_reports_locked() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let service = Closing(service_with(&fake, ok_runner()));
    unlock(&service, &root(&tmp)).unwrap();
    with_lease(&service, &fake, &root(&tmp), |lease| {
        lease.expires_at = Some(Instant::now() + Duration::from_millis(30));
    });
    assert_eq!(
        text(&service.status(Some(&root(&tmp))).unwrap(), "lease"),
        "unlocked"
    );
    std::thread::sleep(Duration::from_millis(40));
    assert_eq!(
        text(&service.status(Some(&root(&tmp))).unwrap(), "lease"),
        "locked"
    );
    assert!(service.state().leases.is_empty());
}

#[test]
fn test_lock_keeps_a_lease_it_was_not_asked_to_end() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let service = Closing(service_with(&fake, ok_runner()));
    let data_root = root(&tmp);
    let status = unlock(&service, &data_root).unwrap();
    let kept = service.lock(&data_root, Some("other-lease"), true).unwrap();
    assert_eq!(
        kassiber_envelope::get(&kept, "kept"),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        text(&service.status(Some(&data_root)).unwrap(), "lease"),
        "unlocked"
    );
    let ended = service
        .lock(&data_root, Some(&text(&status, "lease_id")), true)
        .unwrap();
    assert_eq!(
        kassiber_envelope::get(&ended, "locked"),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        kassiber_envelope::get(&ended, "mode_restored"),
        Some(&Value::Bool(true))
    );
    assert_eq!(fake.effective_unlock_mode(&data_root).unwrap(), "manual");
    // Restoring needs the mode still brokered.
    let again = service.lock(&data_root, None, true).unwrap();
    assert_eq!(
        kassiber_envelope::get(&again, "mode_restored"),
        Some(&Value::Bool(false))
    );
    assert_eq!(
        kassiber_envelope::get(&again, "lease_existed"),
        Some(&Value::Bool(false))
    );
}

#[test]
fn test_unlock_only_if_locked_refuses_an_existing_lease() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let service = Closing(service_with(&fake, ok_runner()));
    unlock(&service, &root(&tmp)).unwrap();
    let result = service.unlock(
        &root(&tmp),
        b"passphrase",
        UnlockRequest {
            only_if_locked: true,
            ..UnlockRequest::default()
        },
    );
    assert_eq!(code(result), "operator_lease_exists");
}

#[test]
fn test_wrong_passphrases_feed_the_shared_backoff() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    fake.open_with(|_, _, _| {
        Err(Error::App(
            AppError::new(
                "unlock_failed",
                "wrong passphrase or unsupported database format",
            )
            .retryable(true),
        ))
    });
    let service = Closing(service_with(&fake, ok_runner()));
    for _ in 0..3 {
        assert_eq!(code(unlock(&service, &root(&tmp))), "unlock_failed");
    }
    assert_eq!(
        code(unlock(&service, &root(&tmp))),
        "local_auth_rate_limited"
    );
    assert_eq!(fake.opened(), 3);
    assert!(fake
        .auth_backoff_path(&root(&tmp))
        .unwrap()
        .unwrap()
        .exists());
}

#[test]
fn test_pinning_strips_every_project_locator() {
    assert_eq!(
        pin_project_arguments(
            &argv(&[
                "--project",
                "a",
                "--data-root=/x",
                "--machine",
                "--project=b",
                "--data-root",
                "/y",
                "status"
            ]),
            "/canonical"
        ),
        argv(&["--data-root", "/canonical", "--machine", "status"])
    );
    // A trailing locator without a value is dropped too.
    assert_eq!(
        pin_project_arguments(&argv(&["status", "--project"]), "/c"),
        argv(&["--data-root", "/c", "status"])
    );
}

#[test]
fn test_classification_reads_secret_labels_as_descriptor_zero() {
    assert_eq!(
        classification_argv(&argv(&[
            "backends",
            "create",
            "--token-fd",
            "broker-secret-ab",
            "--name",
            "broker-secret-cd",
            "broker-secret-ef"
        ])),
        argv(&[
            "backends",
            "create",
            "--token-fd",
            "0",
            "--name",
            "broker-secret-cd",
            "broker-secret-ef"
        ])
    );
}

#[test]
fn test_fingerprints_follow_python_json() {
    // json.dumps([["status"], null, false], separators=(",", ":"))
    assert_eq!(
        request_fingerprint(&argv(&["status"]), None, false),
        sha256_hex(b"[[\"status\"],null,false]")
    );
    assert_eq!(
        request_fingerprint(&argv(&["\u{e4}"]), Some("/w"), true),
        sha256_hex(b"[[\"\\u00e4\"],\"/w\",true]")
    );
}

#[test]
fn test_capabilities_are_cumulative_and_never_admin() {
    use Capability::*;
    for (granted, required, allowed) in [
        (Read, Read, true),
        (Read, Operator, false),
        (Operator, Read, true),
        (AccountingDecisions, Operator, true),
        (Operator, AccountingDecisions, false),
        (AccountingDecisions, Admin, false),
        (Admin, Read, false),
    ] {
        assert_eq!(
            Capability::allows(granted, required),
            allowed,
            "{granted:?} {required:?}"
        );
    }
}

#[test]
fn test_admin_authorization_lives_sixty_seconds() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let service = Closing(service_with(&fake, ok_runner()));
    unlock(&service, &root(&tmp)).unwrap();
    let before = Instant::now();
    let mut authorization = service.verify_admin(&root(&tmp), b"passphrase").unwrap();
    let lifetime = authorization.expires_at - before;
    assert!(lifetime >= ADMIN_AUTH_TTL && lifetime < ADMIN_AUTH_TTL + Duration::from_secs(1));
    authorization.expire_now();
    assert_eq!(
        code(service.submit(
            &root(&tmp),
            &argv(&["secrets", "verify"]),
            SubmitRequest {
                admin_authorization: Some(&authorization),
                ..SubmitRequest::default()
            },
        )),
        "operator_admin_auth_required"
    );
}

#[test]
fn test_idle_timeout_ends_an_agent_lease() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let service = Closing(service_with(&fake, ok_runner()));
    let control = vec![b'c'; MIN_AGENT_CONTROL_BYTES];
    let grant = |timeout: i64| {
        service.unlock(
            &root(&tmp),
            b"passphrase",
            UnlockRequest {
                capability: Capability::Read,
                agent_scope: true,
                idle_timeout_seconds: Some(timeout),
                agent_control: Some(&control),
                ..UnlockRequest::default()
            },
        )
    };
    assert_eq!(code(grant(59)), "operator_invalid_duration");
    let status = grant(60).unwrap();
    let agent = kassiber_envelope::get(&status, "agent").unwrap();
    assert_eq!(
        kassiber_envelope::get(agent, "idle_timeout_seconds"),
        Some(&int(60u64))
    );
    with_lease(&service, &fake, &root(&tmp), |lease| {
        lease.last_used = Instant::now() - Duration::from_secs(59);
    });
    assert_eq!(
        text(&service.status(Some(&root(&tmp))).unwrap(), "lease"),
        "unlocked"
    );
    with_lease(&service, &fake, &root(&tmp), |lease| {
        lease.last_used = Instant::now() - Duration::from_secs(60);
    });
    assert_eq!(
        text(&service.status(Some(&root(&tmp))).unwrap(), "lease"),
        "locked"
    );
    // A terminal lease cannot carry an idle timeout or control secret.
    assert_eq!(
        code(service.unlock(
            &root(&tmp),
            b"passphrase",
            UnlockRequest {
                idle_timeout_seconds: Some(600),
                ..UnlockRequest::default()
            },
        )),
        "operator_protocol_error"
    );
    // An agent lease admits only MCP tool calls.
    grant(600).unwrap();
    assert_eq!(
        code(submit(&service, &root(&tmp), argv(&["status"]))),
        "agent_session_scope"
    );
}
