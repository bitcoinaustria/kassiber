//! The operator broker's backend: the storage crates, the unlock-mode
//! policy and the CLI grammar behind `kassiber_operator::service::Backend`,
//! for `kassiber --operator-broker-server` (plan 21, R6-2).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use kassiber_envelope::{dict, s, AppError};
use kassiber_operator::error::{Error, Result};
use kassiber_operator::service::{Backend, Capability, ChildOwner, Classified, Database, Owner};
use kassiber_ownership::{CanonicalProject, OwnerLease, OwnerNamespace};
use kassiber_runtime::store::{BookStore, Selection, UnlockMode};
use kassiber_store::rusqlite::OptionalExtension;
use kassiber_store::{map_sqlite_error, Book, OpenOptions};

use crate::real_store::RealStore;

fn app(error: AppError) -> Error {
    Error::App(error)
}

fn paths(error: kassiber_paths::Error) -> Error {
    kassiber_operator::error::from_paths(error)
}

fn namespace() -> Result<OwnerNamespace> {
    crate::real_store::owner_namespace().map_err(app)
}

pub struct RealBackend {
    store: RealStore,
}

impl RealBackend {
    pub fn new() -> Self {
        RealBackend {
            store: RealStore::default(),
        }
    }
}

struct BrokerOwner(Option<OwnerLease>);

impl Owner for BrokerOwner {
    fn add_alias(&mut self, project: &CanonicalProject) -> Result<()> {
        match &mut self.0 {
            Some(lease) => lease.add_alias(project).map_err(paths),
            None => Err(Error::Value("the broker owner was released".into())),
        }
    }
    fn duplicate_for_child(&self) -> Result<Box<dyn ChildOwner>> {
        match &self.0 {
            Some(lease) => Ok(Box::new(Inherited(Some(
                lease.duplicate_for_child().map_err(paths)?,
            )))),
            None => Err(Error::Value("the broker owner was released".into())),
        }
    }
    fn release(&mut self) -> Result<()> {
        self.0.take();
        Ok(())
    }
}

struct Inherited(Option<kassiber_ownership::ChildHandles>);

impl ChildOwner for Inherited {
    fn tokens(&self) -> Vec<kassiber_ipc::process::Inheritable> {
        self.0
            .as_ref()
            .map(|handles| handles.tokens())
            .unwrap_or_default()
    }
    fn close(mut self: Box<Self>) -> Result<()> {
        if let Some(handles) = self.0.take() {
            handles.close();
        }
        Ok(())
    }
}

struct Opened(Option<Book>);

impl Database for Opened {
    fn instance_id(&self) -> Result<Option<String>> {
        Ok(self.0.as_ref().map(|book| book.instance_id().to_owned()))
    }
    /// `current_context_snapshot(connection)`: the selected workspace, and
    /// the selected profile when it belongs to it.
    fn context(&self) -> Result<(Option<String>, Option<String>)> {
        let Some(book) = &self.0 else {
            return Ok((None, None));
        };
        let conn = book.connection();
        let workspace: Option<String> = conn
            .query_row(
                "SELECT id FROM workspaces WHERE id=(SELECT value FROM settings WHERE key='context_workspace')",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(map_sqlite_error)
            .map_err(paths)?;
        let profile: Option<(String, String)> = conn
            .query_row(
                "SELECT id,workspace_id FROM profiles WHERE id=(SELECT value FROM settings WHERE key='context_profile')",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(map_sqlite_error)
            .map_err(paths)?;
        let profile = profile
            .filter(|(_, owner)| {
                workspace
                    .as_ref()
                    .is_none_or(|workspace| workspace == owner)
            })
            .map(|(id, _)| id);
        Ok((
            workspace.filter(|id| !id.is_empty()),
            profile.filter(|id| !id.is_empty()),
        ))
    }
    fn close(&mut self) -> Result<()> {
        let Some(book) = self.0.take() else {
            return Ok(());
        };
        book.close().map_err(|failure| {
            let error = paths(failure.error);
            self.0 = Some(*failure.resource);
            error
        })
    }
}

fn selection(data_root: &str) -> Selection {
    Selection {
        data_root: Some(data_root.to_owned()),
        ..Selection::default()
    }
}

impl Backend for RealBackend {
    fn canonical_project(&self, data_root: &str) -> Result<CanonicalProject> {
        namespace()?
            .canonical_project(Path::new(data_root))
            .map_err(paths)
    }

    fn acquire_owner(
        &self,
        project: &CanonicalProject,
        generation: &str,
    ) -> Result<Box<dyn Owner>> {
        let lease = namespace()?
            .acquire(
                project.clone(),
                kassiber_ownership::OwnerKind::Broker,
                generation,
            )
            .map_err(paths)?;
        Ok(Box::new(BrokerOwner(Some(lease))))
    }

    fn open_database(
        &self,
        data_root: &str,
        passphrase: &str,
        expected_database_identity: Option<&str>,
    ) -> Result<Box<dyn Database>> {
        // open_db(require_existing_schema=True) refuses a missing or empty
        // file before keying anything.
        let database = kassiber_paths::resolve_database_path(Path::new(data_root));
        let present =
            std::fs::metadata(&database).is_ok_and(|info| info.is_file() && info.len() > 0);
        if !present {
            return Err(app(AppError::new(
                "invalid_project_database",
                "database does not contain a Kassiber project schema",
            )
            .with_hint(
                "Choose an existing Kassiber project database, not an empty or missing database file.",
            )
            .with_details(dict([("database", s(database.to_string_lossy().as_ref()))]))));
        }
        let book = Book::open_migrating(
            Path::new(data_root),
            OpenOptions {
                passphrase: Some(passphrase),
                expected_database_identity,
                expected_project: None,
            },
            true,
        )
        .map_err(paths)?;
        Ok(Box::new(Opened(Some(book))))
    }

    fn auth_backoff_path(&self, data_root: &str) -> Result<Option<PathBuf>> {
        let state = kassiber_paths::Context::from_env()
            .and_then(|context| context.state_root_for_data(Path::new(data_root)))
            .map_err(paths)?;
        Ok(Some(
            state
                .join("config")
                .join(kassiber_operator::backoff::AUTH_BACKOFF_FILENAME),
        ))
    }

    fn effective_unlock_mode(&self, data_root: &str) -> Result<String> {
        let mode = self
            .store
            .effective_unlock_mode(&selection(data_root))
            .map_err(app)?;
        Ok(match mode {
            UnlockMode::Manual => "manual",
            UnlockMode::Brokered => "brokered",
            UnlockMode::Unattended => "unattended",
        }
        .to_owned())
    }

    fn set_unlock_mode(
        &self,
        data_root: &str,
        mode: &str,
        database_identity: Option<&str>,
        expected_project_identity: Option<&str>,
    ) -> Result<String> {
        kassiber_secrets::modes::set_unlock_mode(
            &namespace()?,
            Path::new(data_root),
            mode,
            database_identity,
            expected_project_identity,
        )
        .map_err(app)
    }

    fn classify(&self, argv: &[String]) -> Result<Classified> {
        let parsed = kassiber_cli::operator::classify(argv).map_err(app)?;
        use kassiber_runtime::capabilities::Capability as Cli;
        Ok(Classified {
            command_path: parsed.command_path,
            capability: match parsed.capability {
                Cli::Read => Capability::Read,
                Cli::Operator => Capability::Operator,
                Cli::AccountingDecisions => Capability::AccountingDecisions,
                Cli::Admin => Capability::Admin,
            },
            workspace: parsed.workspace,
            profile: parsed.profile,
            install: parsed.install,
            data_root: parsed.data_root,
            project: parsed.project,
            env_file: parsed.env_file,
        })
    }

    fn resolve_data_root(
        &self,
        data_root: Option<&str>,
        env_file: Option<&str>,
        project: Option<&str>,
    ) -> Result<String> {
        self.store
            .resolve_data_root(&Selection {
                data_root: data_root.map(str::to_owned),
                env_file: env_file.map(str::to_owned),
                project: project.map(str::to_owned),
            })
            .map_err(app)
    }
}

/// The session guard: the system one, or in test builds the integration
/// fixture's (`KASSIBER_RUST_TEST_OPERATOR_SESSION=fixture`, the
/// equivalent of `tests/operator_server_fixture.py`).
fn session_guard() -> kassiber_operator::server::SessionGuard {
    #[cfg(feature = "test-harness")]
    if std::env::var("KASSIBER_RUST_TEST_OPERATOR_SESSION").as_deref() == Ok("fixture") {
        if let Some(guard) = kassiber_operator::server::SessionGuard::isolated_test_runtime() {
            return guard;
        }
    }
    kassiber_operator::server::SessionGuard::system()
}

/// `kassiber --operator-broker-server`.
pub fn serve(build: kassiber_runtime::BuildInfo) -> i32 {
    let backend: Arc<dyn Backend> = Arc::new(RealBackend::new());
    kassiber_operator::server::main(
        backend,
        kassiber_operator::build::BuildInputs {
            version: build.version,
            channel: build.channel,
            commit: build.commit,
        },
        session_guard(),
        &mut std::io::stderr(),
    )
}
