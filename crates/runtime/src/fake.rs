//! An in-memory [`BookStore`] for the shell's tests (feature `fake`).
//!
//! It never touches the file system except an optional event log, so the
//! CLI and daemon protocol can be exercised end to end before the storage
//! crates exist. Release builds do not include it.

use std::sync::{Arc, Mutex};

use kassiber_envelope::{dict, get, s, AppError};
use kassiber_pyjson::Value;

use crate::store::{
    Book, BookContext, BookStore, Bootstrap, JournalFailure, JournalReply, OwnerKind, ProjectOwner,
    ProjectRef, RuntimeConfig, RuntimePaths, Secret, Selection, StatusCounts, StatusFacts, Unlock,
    UnlockMode,
};

/// What the fake book looks like.
#[derive(Clone, Debug)]
pub struct FakeConfig {
    pub data_root: String,
    pub encrypted: bool,
    /// The passphrase that unlocks an encrypted fake book.
    pub passphrase: Option<String>,
    pub initialized: bool,
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub project_root: Option<String>,
    pub workspace: Option<(String, String)>,
    pub profile: Option<(String, String)>,
    pub counts: StatusCounts,
    pub default_backend: Option<String>,
    pub unlock_mode: UnlockMode,
    /// Appends one line per store operation, for black-box tests.
    pub events_file: Option<String>,
}

impl Default for FakeConfig {
    fn default() -> Self {
        FakeConfig {
            data_root: "/fake/kassiber/data".to_owned(),
            encrypted: false,
            passphrase: None,
            initialized: true,
            project_id: None,
            project_name: None,
            project_root: None,
            workspace: Some(("ws-1".to_owned(), "Personal".to_owned())),
            profile: Some(("pf-1".to_owned(), "Main".to_owned())),
            counts: StatusCounts::default(),
            default_backend: Some("mempool".to_owned()),
            unlock_mode: UnlockMode::Manual,
            events_file: None,
        }
    }
}

fn text_field(config: &Value, key: &str) -> Option<String> {
    match get(config, key) {
        Some(Value::Str(text)) => text.as_str().map(str::to_owned),
        _ => None,
    }
}

fn pair_field(config: &Value, key: &str) -> Option<Option<(String, String)>> {
    match get(config, key)? {
        Value::Null => Some(None),
        value => Some(Some((
            text_field(value, "id")?,
            text_field(value, "label")?,
        ))),
    }
}

impl FakeConfig {
    /// Reads a JSON object (as `json.loads` returns it); absent keys keep
    /// their defaults.
    pub fn from_value(config: &Value) -> FakeConfig {
        let mut fake = FakeConfig::default();
        if let Some(root) = text_field(config, "data_root") {
            fake.data_root = root;
        }
        if let Some(Value::Bool(encrypted)) = get(config, "encrypted") {
            fake.encrypted = *encrypted;
        }
        fake.passphrase = text_field(config, "passphrase");
        if let Some(Value::Bool(initialized)) = get(config, "initialized") {
            fake.initialized = *initialized;
        }
        fake.project_id = text_field(config, "project_id");
        fake.project_name = text_field(config, "project_name");
        fake.project_root = text_field(config, "project_root");
        if let Some(workspace) = pair_field(config, "workspace") {
            fake.workspace = workspace;
        }
        if let Some(profile) = pair_field(config, "profile") {
            fake.profile = profile;
        }
        if let Some(Value::Int(n)) = get(config, "transactions") {
            fake.counts.transactions = i64::try_from(n.clone()).unwrap_or(0);
        }
        if get(config, "default_backend").is_some() {
            fake.default_backend = text_field(config, "default_backend");
        }
        if let Some(Value::Bool(true)) = get(config, "brokered") {
            fake.unlock_mode = UnlockMode::Brokered;
        }
        fake.events_file = text_field(config, "events_file");
        fake
    }
}

/// The in-memory store.
#[derive(Clone, Debug)]
pub struct FakeStore {
    config: Arc<FakeConfig>,
    events: Arc<Mutex<Vec<String>>>,
}

impl FakeStore {
    pub fn new(config: FakeConfig) -> Self {
        FakeStore {
            config: Arc::new(config),
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Every operation so far, in order.
    pub fn events(&self) -> Vec<String> {
        self.events.lock().map(|e| e.clone()).unwrap_or_default()
    }

    fn record(&self, event: String) {
        if let Some(path) = &self.config.events_file {
            use std::io::Write;
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let _ = writeln!(file, "{event}");
            }
        }
        if let Ok(mut events) = self.events.lock() {
            events.push(event);
        }
    }

    fn paths(&self, data_root: &str, env_file: Option<&str>) -> RuntimePaths {
        let config_root = format!("{data_root}/config");
        RuntimePaths {
            data_root: data_root.to_owned(),
            env_file: env_file
                .map(str::to_owned)
                .unwrap_or_else(|| format!("{config_root}/backends.env")),
            state_root: data_root.to_owned(),
            settings_file: format!("{config_root}/settings.json"),
            config_root,
            exports_root: format!("{data_root}/exports"),
            attachments_root: format!("{data_root}/attachments"),
            database: format!("{data_root}/data/kassiber.sqlite3"),
            project_id: self.config.project_id.clone(),
            project_name: self.config.project_name.clone(),
            project_root: self.config.project_root.clone(),
        }
    }
}

struct FakeOwner {
    store: FakeStore,
    project: ProjectRef,
    released: bool,
}

impl ProjectOwner for FakeOwner {
    fn project(&self) -> &ProjectRef {
        &self.project
    }

    fn release(&mut self) -> Result<(), AppError> {
        if !self.released {
            self.released = true;
            self.store
                .record(format!("release {}", self.project.public_id));
        }
        Ok(())
    }
}

struct FakeBook {
    store: FakeStore,
    paths: RuntimePaths,
    closed: bool,
}

struct FakeJournalReply {
    store: FakeStore,
    payload: Value,
}
impl JournalReply for FakeJournalReply {
    fn payload(&self) -> &Value {
        &self.payload
    }
    fn after_reply(self: Box<Self>) -> Result<(), AppError> {
        self.store.record("journals checkpoint".into());
        Ok(())
    }
}

impl Book for FakeBook {
    fn process_journals(
        &mut self,
        workspace: Option<&Value>,
        profile: Option<&Value>,
        auto_pair: bool,
        progress: &mut dyn FnMut(&'static str),
    ) -> Result<Box<dyn JournalReply + '_>, JournalFailure> {
        self.store.record("journals process".into());
        progress("complete");
        Ok(Box::new(FakeJournalReply {
            store: self.store.clone(),
            payload: dict([
                ("workspace", workspace.cloned().unwrap_or(Value::Null)),
                ("profile", profile.cloned().unwrap_or(Value::Null)),
                ("auto_pair", Value::Bool(auto_pair)),
            ]),
        }))
    }

    fn is_initialized(&self) -> Result<bool, AppError> {
        Ok(self.store.config.initialized)
    }

    fn context(&self) -> Result<BookContext, AppError> {
        let config = &self.store.config;
        let part = |pair: &Option<(String, String)>, label: bool| {
            pair.as_ref()
                .map(|(id, name)| if label { name.clone() } else { id.clone() })
                .unwrap_or_default()
        };
        Ok(BookContext {
            workspace_id: part(&config.workspace, false),
            workspace_label: part(&config.workspace, true),
            profile_id: part(&config.profile, false),
            profile_label: part(&config.profile, true),
        })
    }

    fn status_facts(&self) -> Result<StatusFacts, AppError> {
        let config = &self.store.config;
        Ok(StatusFacts {
            paths: self.paths.clone(),
            database_encrypted: config.encrypted,
            project_id: config.project_id.clone(),
            project_name: config.project_name.clone(),
            project_root: config.project_root.clone(),
            context: self.context()?,
            counts: config.counts,
        })
    }

    fn resolve_scope(
        &self,
        workspace: Option<&Value>,
        profile: Option<&Value>,
    ) -> Result<(String, String), AppError> {
        let config = &self.store.config;
        let pick = |wanted: Option<&Value>, have: &Option<(String, String)>, what: &str| {
            let (id, label) = have
                .clone()
                .ok_or_else(|| AppError::new("not_found", format!("No {what} is selected")))?;
            match wanted {
                None | Some(Value::Null) => Ok(id),
                Some(Value::Str(text))
                    if text.as_str() == Some(&id) || text.as_str() == Some(&label) =>
                {
                    Ok(id)
                }
                Some(_) => Err(AppError::new("not_found", format!("{what} not found"))),
            }
        };
        Ok((
            pick(workspace, &config.workspace, "Workspace")?,
            pick(profile, &config.profile, "Profile")?,
        ))
    }

    fn rollback(&mut self) -> Result<(), AppError> {
        Ok(())
    }

    fn close(&mut self) -> Result<(), AppError> {
        if !self.closed {
            self.closed = true;
            self.store.record("close".to_owned());
        }
        Ok(())
    }
}

impl BookStore for FakeStore {
    fn migrate_default_state_root(&self) -> Result<(), AppError> {
        self.record("migrate default state root".to_owned());
        Ok(())
    }

    fn bootstrap(&self, selection: &Selection) -> Result<Bootstrap, AppError> {
        if selection.data_root.is_some() && selection.project.is_some() {
            return Err(AppError::new(
                "invalid_flag_combination",
                "--project and --data-root cannot be used together",
            )
            .with_hint(
                "Use --project for a catalog project or --data-root for an explicit project data directory, not both.",
            ));
        }
        let data_root = selection
            .data_root
            .clone()
            .unwrap_or_else(|| self.config.data_root.clone());
        let paths = self.paths(&data_root, selection.env_file.as_deref());
        self.record(format!("bootstrap {data_root}"));
        Ok(Bootstrap {
            // The book's default backend arrives with `merge_db_backends`
            // when the book opens.
            runtime_config: RuntimeConfig {
                default_backend: None,
                env_file: paths.env_file.clone(),
            },
            paths,
            selection_explicit: selection.data_root.is_some() || selection.project.is_some(),
        })
    }

    fn effective_unlock_mode(&self, _: &Selection) -> Result<UnlockMode, AppError> {
        self.record("unlock mode".to_owned());
        Ok(self.config.unlock_mode)
    }

    fn selected_data_root(&self, selection: &Selection) -> Result<String, AppError> {
        if selection.data_root.is_some() && selection.project.is_some() {
            return Err(AppError::new(
                "invalid_flag_combination",
                "--project and --data-root cannot be used together",
            )
            .with_hint(
                "Use --project for a catalog project or --data-root for an explicit project data directory, not both.",
            ));
        }
        Ok(selection
            .data_root
            .clone()
            .unwrap_or_else(|| self.config.data_root.clone()))
    }

    fn unlock_mode_status(&self, _: &str) -> Result<Value, AppError> {
        let mode = match self.config.unlock_mode {
            UnlockMode::Manual => "manual",
            UnlockMode::Brokered => "brokered",
            UnlockMode::Unattended => "unattended",
        };
        Ok(dict([
            ("configured", s(mode)),
            ("effective", s(mode)),
            ("legacy_inferred", Value::Bool(false)),
            ("binding_state", s("valid")),
        ]))
    }

    fn database_is_encrypted(&self, _: &str) -> Result<bool, AppError> {
        Ok(self.config.encrypted)
    }

    fn canonical_project(&self, data_root: &str) -> Result<ProjectRef, AppError> {
        Ok(ProjectRef {
            identity: format!("fake:{data_root}"),
            public_id: self
                .config
                .project_id
                .clone()
                .unwrap_or_else(|| "fake".to_owned()),
            data_root: data_root.to_owned(),
        })
    }

    fn acquire_ownership(
        &self,
        project: &ProjectRef,
        owner_kind: OwnerKind,
        _generation: &str,
    ) -> Result<Box<dyn ProjectOwner>, AppError> {
        self.record(format!("acquire {} {owner_kind:?}", project.public_id));
        Ok(Box::new(FakeOwner {
            store: self.clone(),
            project: project.clone(),
            released: false,
        }))
    }

    fn open_book(
        &self,
        data_root: &str,
        unlock: Unlock<'_>,
        runtime_config: &mut RuntimeConfig,
    ) -> Result<Box<dyn Book>, AppError> {
        if self.config.encrypted {
            let supplied = match &unlock {
                Unlock::Daemon { passphrase, .. } => passphrase.map(|p| p.expose().to_owned()),
                Unlock::Cli { passphrase_fd, .. } => {
                    passphrase_fd.and(self.config.passphrase.clone())
                }
            };
            match (supplied, &self.config.passphrase) {
                (None, _) => {
                    return Err(match unlock {
                        Unlock::Daemon { .. } => AppError::new(
                            "passphrase_required",
                            "database is encrypted; a passphrase is required",
                        ),
                        Unlock::Cli { .. } => AppError::new(
                            "interaction_required",
                            "database authorization requires local interaction",
                        )
                        .with_details(dict([
                            ("reason", s("database_passphrase")),
                            ("unlock_mode", Value::Null),
                            ("user_command", s("kassiber operator unlock")),
                        ])),
                    })
                }
                (Some(given), Some(expected)) if &given == expected => {}
                _ => {
                    return Err(AppError::new(
                        "unlock_failed",
                        "could not unlock the database",
                    ))
                }
            }
        }
        self.record(format!("open {data_root}"));
        if self.config.default_backend.is_some() {
            runtime_config.default_backend = self.config.default_backend.clone();
        }
        Ok(Box::new(FakeBook {
            store: self.clone(),
            paths: self.paths(data_root, None),
            closed: false,
        }))
    }

    fn verify_passphrase(
        &self,
        _: &str,
        scope: &str,
        passphrase: &Secret,
    ) -> Result<bool, AppError> {
        let ok = self.config.passphrase.as_deref() == Some(passphrase.expose());
        self.record(format!("verify {scope} {ok}"));
        Ok(ok)
    }

    fn mark_project_opened(&self, project_id: &str, _: &str, select: bool) -> Result<(), AppError> {
        self.record(format!("opened {project_id} select={select}"));
        Ok(())
    }
}
