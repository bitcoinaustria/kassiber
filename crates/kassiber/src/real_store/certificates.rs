//! Confirmed trust for a saved backend's TLS certificate
//! (docs/reference/backends.md#self-signed-certificates).
//!
//! Each kind is one explicit user action and the only path that fetches a
//! certificate: no read, status, listing or refresh reaches it. A fetch shows
//! the certificate and trusts nothing. Trusting requires the fingerprint the
//! user confirmed, fetches again, writes the pin owner-only into the data
//! root, and points the backend's `certificate` at it. A changed pin is only
//! ever replaced this way, never on its own.
use super::*;
use kassiber_chain::certificates::{self, CertificateConsent, Fetched};
use kassiber_envelope::{get, py_str};
use kassiber_setup::backend_certificates;

pub(super) const COMMANDS: &[&str] = &[
    "backends.trust-certificate",
    "ui.backends.certificate.fetch",
    "ui.backends.certificate.trust",
];

fn text(args: &Value, key: &str) -> Option<String> {
    get(args, key)
        .filter(|v| !matches!(v, Value::Null))
        .map(py_str)
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

/// What a fetch shows, plus the transport revision it was fetched for: the
/// desktop hands that back when trusting, binding the trust to the backend
/// setup the user reviewed.
fn payload(fetched: &Fetched, revision: &str) -> Result<Value, AppError> {
    let mut shown = fetched.payload();
    shown["revision"] = serde_json::Value::String(revision.to_owned());
    kassiber_pytext::loads(&shown.to_string())
        .map_err(|_| AppError::new("internal_error", "certificate payload"))
}

fn changed_since_review(name: &str) -> AppError {
    AppError::new(
        "backend_certificate_config_changed",
        format!("Backend '{name}' changed since its certificate was checked"),
    )
    .with_hint("Nothing was trusted. Check the certificate again for the backend as it is now.")
}

impl RealBook {
    pub(super) fn backend_certificate(
        &mut self,
        operation: &str,
        args: &Value,
    ) -> Result<Value, AppError> {
        let name = text(args, "name")
            .ok_or_else(|| AppError::new("validation", "Backend name is required"))?
            .to_lowercase();
        let fingerprint = text(args, "fingerprint");
        let reviewed = text(args, "revision");
        if operation == "ui.backends.certificate.trust" && fingerprint.is_none() {
            return Err(AppError::new(
                "validation",
                "The confirmed certificate fingerprint is required",
            ));
        }
        if operation == "ui.backends.certificate.trust" && reviewed.is_none() {
            return Err(AppError::new(
                "validation",
                "The revision of the reviewed certificate is required",
            ));
        }
        // The snapshot every fetch below uses; trusting compares the backend
        // with it again inside the write transaction.
        let config = self.backends.resolve(self.conn()?, &name)?;
        let overrides = kassiber_setup::backends::environment_overrides(&name);
        if !overrides.is_empty() {
            // Names only: a value could be a proxy credential or a path.
            return Err(AppError::new(
                "backend_certificate_overridden",
                format!(
                    "Backend '{name}' has connection settings overridden by the environment ({}), so a trusted certificate might not be used",
                    overrides.join(", ")
                ),
            )
            .with_hint("Unset those variables for Kassiber, then check the certificate again."));
        }
        let revision = kassiber_setup::backends::transport_revision(&name, &config);
        if reviewed.is_some_and(|reviewed| reviewed != revision) {
            // Refused before connecting: the endpoint, proxy or trust settings
            // are not the ones the certificate was reviewed for.
            return Err(changed_since_review(&name));
        }
        let gate = lightning::network_gate();
        // Reached only for one explicit CLI command or registered desktop
        // request naming this backend, after scope and unlock checks.
        let consent = CertificateConsent::explicit(&name);
        let Some(expected) = fingerprint.filter(|_| operation != "ui.backends.certificate.fetch")
        else {
            let fetched = certificates::fetch(&gate, &consent, &config)?;
            if operation == "ui.backends.certificate.fetch" {
                return payload(&fetched, &revision);
            }
            return Err(AppError::new(
                "confirmation_required",
                format!(
                    "{}:{} presents a certificate with SHA-256 fingerprint {}",
                    fetched.host(),
                    fetched.port(),
                    fetched.fingerprint()
                ),
            )
            .with_hint(format!(
                "Compare this fingerprint with the one your server shows (for example `openssl x509 -noout -fingerprint -sha256 -in <its certificate>`). If they match, trust it with `kassiber backends trust-certificate {name} --fingerprint {}`.",
                fetched.fingerprint()
            ))
            .with_details(payload(&fetched, &revision)?));
        };
        let fetched = certificates::confirm(&gate, &consent, &config, &expected)?;
        let data_root = PathBuf::from(&self.data_root);
        // A new immutable file: the previous pin stays until the row moves.
        let pin = backend_certificates::write_pin(&data_root, fetched.der(), &fetched.pem())?;
        let path_text = pin.path.to_string_lossy().into_owned();
        let conn = self
            .book
            .as_ref()
            .ok_or_else(|| AppError::new("internal_error", "book is closed"))?
            .connection();
        let outcome = self
            .backends
            .seed_configuration(conn)
            .and_then(|()| self.backends.merge(conn))
            .and_then(|()| {
                self.backends.pin_certificate(
                    conn,
                    &name,
                    &config,
                    &path_text,
                    &|effective| certificates::verify_pinned(effective, &fetched),
                    &SetupSources,
                )
            });
        // Check and delete under the write lock: another trust of the same leaf
        // holds that lock from its verify to its commit, so it either commits
        // first (the file is referenced and stays) or verifies after the
        // delete and refuses. A failed check keeps the file.
        let remove_if_unreferenced = |path: &str| {
            let Ok(tx) = kassiber_store::write_transaction(conn) else {
                return;
            };
            let referenced =
                kassiber_setup::backends::certificate_referenced(&tx, path).unwrap_or(true);
            backend_certificates::remove_unreferenced(&data_root, Path::new(path), referenced);
            let _ = tx.commit();
        };
        let previous = match outcome {
            Ok(previous) => previous,
            Err(error) => {
                if pin.created {
                    remove_if_unreferenced(&path_text);
                }
                return Err(error);
            }
        };
        // Committed: the long-lived desktop daemon answers settings reads from
        // this view, which must show the pin rather than the old trust setup.
        self.backends.merge(conn)?;
        // Committed: a replaced managed pin no backend uses any more can go.
        if let Some(previous) = previous.filter(|p| *p != path_text) {
            remove_if_unreferenced(&previous);
        }
        let mut result = fetched.payload();
        result["trusted"] = serde_json::Value::Bool(true);
        result["certificate"] = serde_json::Value::String(path_text);
        kassiber_pytext::loads(&result.to_string())
            .map_err(|_| AppError::new("internal_error", "certificate payload"))
    }
}
