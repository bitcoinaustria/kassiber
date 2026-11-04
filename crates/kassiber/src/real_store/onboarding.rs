//! Desktop wallet onboarding: script-type detection, ownership identification
//! and the ledger preview. Only detection contacts a backend, and only for
//! the user's explicit request.
use super::{lightning::network_gate, RealBook};
use kassiber_envelope::{dict, get, py_str, s, set, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;
use kassiber_wallets::catalog_detection::{self, Detection, Grant};
use kassiber_wallets::identify;
use std::time::{Duration, Instant};

impl RealBook {
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
