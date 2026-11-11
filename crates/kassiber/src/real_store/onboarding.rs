//! Desktop wallet onboarding: script-type detection, ownership identification
//! and the ledger preview. Only detection contacts a backend, and only for
//! the user's explicit request.
use super::{lightning::network_gate, RealBook};
use kassiber_envelope::{dict, get, py_str, s, set, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;
use kassiber_wallets::catalog_detection::{self, Detection, Grant};
use kassiber_wallets::identify::{
    self,
    onchain::{Start, Verification, Verifier},
};
use std::time::{Duration, Instant};

impl RealBook {
    /// `_with_local_journal_refresh` after the import committed: a refresh
    /// already running elsewhere is reported as deferred, never as an
    /// import failure.
    pub(super) fn finish_import(
        &mut self,
        mut payload: Value,
        profile_id: &str,
    ) -> Result<Value, AppError> {
        if !matches!(payload, Value::Dict(_)) || profile_id.is_empty() {
            return Ok(payload);
        }
        let journals = match self.local_journal_refresh(profile_id) {
            Ok(summary) => summary,
            Err(error) if error.code == "project_operation_in_progress" => {
                let mut summary = kassiber_wallets::freshness::ui::journal_step_summary(&[], &[]);
                set(&mut summary, "status", s("deferred"));
                set(
                    &mut summary,
                    "error",
                    dict([
                        ("code", s(error.code.as_str())),
                        ("message", s(error.message.as_str())),
                    ]),
                );
                summary
            }
            Err(error) => return Err(error),
        };
        set(&mut payload, "journals", journals);
        Ok(payload)
    }

    fn local_journal_refresh(&mut self, profile_id: &str) -> Result<Value, AppError> {
        let cancel = kassiber_wallets::scan::Cancellation::default();
        let conn = self
            .book
            .as_mut()
            .ok_or_else(|| AppError::new("internal_error", "book is closed"))?
            .connection_mut();
        let mut handler = super::journals::Refresh {
            cancel: &cancel,
            consent: None,
            context: kassiber_setup::context::snapshot(conn)?,
            deadline: Instant::now() + Duration::from_secs(900),
        };
        let (mut sources, _) = super::sync_sources();
        kassiber_wallets::freshness::planning::local_journal_refresh(
            conn,
            profile_id,
            sources.as_mut(),
            &mut handler,
        )
    }

    /// `build_wallet_identify_snapshot`: cache-only, so safe without consent.
    pub(super) fn identify_wallets(&self, args: &Value) -> Result<Value, AppError> {
        let context = self.context()?;
        if context.workspace_id.is_empty() || context.profile_id.is_empty() {
            return Ok(identify::empty());
        }
        let inputs = identify::Inputs::from_args(Some(args))?;
        let mut payload = identify::identify(self.conn()?, &context.profile_id, &inputs)?;
        let label = |text: &str| {
            if text.is_empty() {
                Value::Null
            } else {
                s(text)
            }
        };
        set(
            &mut payload,
            "context",
            dict([
                ("workspace", label(&context.workspace_label)),
                ("profile", label(&context.profile_label)),
            ]),
        );
        Ok(payload)
    }

    /// `wallets identify` on the CLI: the scope, then Python's argument
    /// checks and report. `--verify-on-chain` is the explicit trigger for
    /// its lookups on the resolved Esplora or Electrum backend.
    pub(super) fn identify_cli(
        &self,
        workspace: Option<&Value>,
        profile: Option<&Value>,
        args: identify::CliArgs,
    ) -> Result<Value, AppError> {
        let (_, profile_id) = self.resolve_scope(workspace, profile)?;
        let conn = self.conn()?;
        identify::cli_report(conn, &profile_id, args, |name| {
            let backend = self.backends.resolve_configured(conn, name)?;
            Ok((Verifier::new(&backend)?, network_gate()))
        })
    }

    /// `prepare_wallet_identify_onchain`: inputs, then the verification
    /// backend, then the local tiers; txids local data cannot settle stay
    /// pending for the worker.
    pub(super) fn prepare_identify_onchain(&self, args: &Value) -> Result<Start, AppError> {
        let context = self.context()?;
        if context.workspace_id.is_empty() || context.profile_id.is_empty() {
            return Ok(Start::Report(identify::empty()));
        }
        let inputs = identify::Inputs::from_args(Some(args))?;
        let name = get(args, "backend")
            .filter(|value| !matches!(value, Value::Null))
            .map(py_str);
        let backend = self
            .backends
            .resolve_configured(self.conn()?, name.as_deref())?;
        let verifier = Verifier::new(&backend)?;
        let plan = identify::prepare(self.conn()?, &context.profile_id, &inputs, true)?;
        let label = |text: &str| {
            if text.is_empty() {
                Value::Null
            } else {
                s(text)
            }
        };
        let scope = dict([
            ("workspace", label(&context.workspace_label)),
            ("profile", label(&context.profile_label)),
        ]);
        Ok(Verification::start(plan, verifier, network_gate(), scope))
    }

    /// The daemon has already validated the material and argument types.
    pub(super) fn detect_script_types(&mut self, args: &Value) -> Result<Value, AppError> {
        let text = |key| {
            get(args, key)
                .filter(|value| !matches!(value, Value::Null))
                .map(py_str)
        };
        let material = text("wallet_material").unwrap_or_default();
        let backend = match self
            .backends
            .resolve_configured(self.conn()?, text("backend").as_deref())
        {
            Ok(backend) => backend,
            Err(error) => return Ok(catalog_detection::fallback(&error.message)),
        };
        let chain = text("chain").unwrap_or_else(|| "bitcoin".into());
        let action = match Detection::new(&material, &chain, text("network").as_deref(), &backend) {
            Ok(action) => action,
            Err(error) => return Ok(catalog_detection::fallback(&error.message)),
        };
        // This request is the user's explicit "detect" action; the grant is
        // bound to it and spent by the one run below.
        let grant = Grant::explicit(&action);
        Ok(action
            .run(
                Some(&grant),
                network_gate(),
                Instant::now() + Duration::from_secs(300),
            )
            .unwrap_or_else(|error| catalog_detection::fallback(&error.message)))
    }
}
