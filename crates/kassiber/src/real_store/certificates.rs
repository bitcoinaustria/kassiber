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

fn payload(fetched: &Fetched) -> Result<Value, AppError> {
    kassiber_pytext::loads(&fetched.payload().to_string())
        .map_err(|_| AppError::new("internal_error", "certificate payload"))
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
        if operation == "ui.backends.certificate.trust" && fingerprint.is_none() {
            return Err(AppError::new(
                "validation",
                "The confirmed certificate fingerprint is required",
            ));
        }
        let config = self.backends.resolve(self.conn()?, &name)?;
        let gate = lightning::network_gate();
        // Reached only for one explicit CLI command or registered desktop
        // request naming this backend, after scope and unlock checks.
        let consent = CertificateConsent::explicit(&name);
        let Some(expected) = fingerprint.filter(|_| operation != "ui.backends.certificate.fetch")
        else {
            let fetched = certificates::fetch(&gate, &consent, &config)?;
            if operation == "ui.backends.certificate.fetch" {
                return payload(&fetched);
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
            .with_details(payload(&fetched)?));
        };
        let fetched = certificates::confirm(&gate, &consent, &config, &expected)?;
        let path = kassiber_setup::backend_certificates::write_pin(
            Path::new(&self.data_root),
            &name,
            &fetched.pem(),
        )?;
        let path_text = path.to_string_lossy().into_owned();
        let conn = self
            .book
            .as_ref()
            .ok_or_else(|| AppError::new("internal_error", "book is closed"))?
            .connection();
        self.backends.seed_configuration(conn)?;
        self.backends.merge(conn)?;
        self.backends.execute(
            conn,
            "backends.update",
            &dict([
                ("name", s(name.as_str())),
                (
                    "config",
                    dict([
                        ("certificate", s(path_text.as_str())),
                        // A pin replaces disabled verification.
                        ("insecure", Value::Null),
                    ]),
                ),
            ]),
            &SetupSources,
        )?;
        // An environment override wins over the stored setting; then the pin
        // would never be used, so say so instead of reporting success.
        let effective = self.backends.resolve(self.conn()?, &name)?;
        if effective
            .get("certificate")
            .and_then(serde_json::Value::as_str)
            != Some(path_text.as_str())
        {
            return Err(AppError::new(
                "backend_certificate_overridden",
                "The certificate was saved, but this backend's certificate setting is overridden by its environment or env file",
            )
            .with_hint("Remove the CERTIFICATE override for this backend so the trusted certificate is used."));
        }
        let mut result = fetched.payload();
        result["trusted"] = serde_json::Value::Bool(true);
        result["certificate"] = serde_json::Value::String(path_text);
        kassiber_pytext::loads(&result.to_string())
            .map_err(|_| AppError::new("internal_error", "certificate payload"))
    }
}
