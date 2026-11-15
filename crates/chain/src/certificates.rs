//! Confirmed trust-on-first-use for a backend's TLS certificate. Fetching is
//! an explicit user action only: never on launch, display, status or a timer.
//! The fetched certificate is shown for a decision and trusts nothing; the
//! caller pins it only after the user confirms its SHA-256 fingerprint and a
//! second fetch presents the same certificate.
use super::*;
use kassiber_net::{Gate, PinRefusal, PresentedCertificate, TlsTrust};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::Path;

/// Backend kinds whose `certificate` setting pins the TLS server certificate.
pub const KINDS: &[&str] = &[
    "bitcoinrpc",
    "electrum",
    "esplora",
    "liquid-esplora",
    "mempool",
];

/// A grant for one explicit certificate fetch of one named backend.
pub struct CertificateConsent {
    backend: String,
}

impl CertificateConsent {
    /// Trusted explicit user-action boundary only (a CLI command or one
    /// desktop request). Never construct from a read, listing or refresh.
    pub fn explicit(backend: &str) -> Self {
        Self {
            backend: backend.trim().to_lowercase(),
        }
    }
}

/// The certificate a backend's server presented, with what pinning it means.
pub struct Fetched {
    backend: String,
    kind: String,
    port: u16,
    presented: PresentedCertificate,
    fingerprint: String,
    previous: Option<String>,
    clears_insecure: bool,
}

impl Fetched {
    /// Uppercase, colon-separated SHA-256 of the leaf DER, as
    /// `openssl x509 -noout -fingerprint -sha256` prints it.
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    pub fn host(&self) -> &str {
        self.presented.host()
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn pem(&self) -> String {
        self.presented.leaf_pem()
    }

    /// The leaf certificate's DER bytes.
    pub fn der(&self) -> &[u8] {
        self.presented.leaf_der()
    }

    /// The display payload: no URL (it may carry credentials), no PEM.
    pub fn payload(&self) -> Value {
        let summary = self.presented.summary();
        let time = |t: Option<i64>| t.and_then(time::epoch).map_or(Value::Null, Value::from);
        json!({
            "backend": self.backend,
            "kind": self.kind,
            "host": self.presented.host(),
            "port": self.port,
            "fingerprint_sha256": self.fingerprint,
            "subject": summary.as_ref().and_then(|s| s.subject.clone()),
            "issuer": summary.as_ref().and_then(|s| s.issuer.clone()),
            "not_before": time(summary.as_ref().map(|s| s.not_before)),
            "not_after": time(summary.as_ref().map(|s| s.not_after)),
            "self_signed": summary.as_ref().is_some_and(|s| s.self_issued),
            "names": summary.map(|s| s.names).unwrap_or_default(),
            "replaces_fingerprint_sha256": self.previous,
            "clears_insecure": self.clears_insecure,
        })
    }
}

/// `AB:CD:...` for a DER certificate.
pub fn fingerprint(der: &[u8]) -> String {
    Sha256::digest(der)
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// Accept a SHA-256 fingerprint with or without colons or spaces, any case.
pub fn normalize_fingerprint(text: &str) -> Result<String> {
    let hex: String = text
        .chars()
        .filter(|c| !matches!(c, ':' | ' ' | '-'))
        .collect();
    if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(AppError::new(
            "validation",
            "Certificate fingerprint must be a SHA-256 fingerprint (64 hex digits)",
        )
        .with_hint(
            "Copy the fingerprint Kassiber showed, for example AB:CD:...; colons are optional.",
        ));
    }
    let upper = hex.to_ascii_uppercase();
    Ok(upper
        .as_bytes()
        .chunks(2)
        .map(|pair| std::str::from_utf8(pair).expect("ASCII hex"))
        .collect::<Vec<_>>()
        .join(":"))
}

fn refused(code: &str, message: impl Into<String>, hint: &str) -> AppError {
    AppError::new(code, message).with_hint(hint)
}

/// Fetch the certificate `config`'s server presents. Refuses before any
/// connection for kinds without a certificate setting and for plaintext URLs,
/// and after the fetch when there is nothing to pin: the chain already reaches
/// a trusted root or the backend's configured certificate, or a pin of this
/// certificate would not verify.
pub fn fetch(gate: &Gate, consent: &CertificateConsent, config: &Value) -> Result<Fetched> {
    let name = config::value(config, &["name"])
        .unwrap_or("")
        .trim()
        .to_lowercase();
    if name.is_empty() || name != consent.backend {
        return Err(AppError::new(
            "interaction_required",
            "Network action requires current consent",
        ));
    }
    let kind = config::value(config, &["kind"]).unwrap_or("").to_owned();
    if !KINDS.contains(&kind.as_str()) {
        return Err(refused(
            "backend_certificate_unsupported",
            format!("Backend '{name}' ({kind}) has no TLS certificate setting to pin"),
            "Certificate trust applies to Electrum, Esplora/HTTP and Bitcoin Core RPC backends.",
        ));
    }
    let electrum = kind == "electrum";
    // The capture never uses the route's trust; a stale or missing pin file
    // must not stop the user from looking at the server's certificate.
    let backend = Backend::with_trust(config, electrum, Some(TlsTrust::BundledRoots))?;
    if !backend.secure {
        return Err(refused(
            "backend_certificate_plaintext",
            format!("Backend '{name}' does not use TLS, so there is no certificate to trust"),
            if electrum {
                "Use an ssl:// URL if the server offers TLS."
            } else {
                "Use an https:// URL if the server offers TLS."
            },
        ));
    }
    let consent = Consent::explicit_trigger(backend.request());
    let route = gate
        .authorize(backend.request().clone(), Some(consent))
        .map_err(|e| e.app_error())?;
    let presented = route
        .fetch_certificate(backend.timeout(), !electrum)
        .map_err(|e| e.app_error())?;
    let trusted = |trust: &TlsTrust| presented.trusted_by(trust).map_err(|e| e.app_error());
    if trusted(&TlsTrust::python_default().map_err(|e| e.app_error())?)? {
        return Err(refused(
            "backend_certificate_already_trusted",
            "This server's certificate is signed by an authority Kassiber already trusts",
            "There is nothing to pin. If this backend still fails, clear its certificate setting.",
        ));
    }
    let configured = config::value(config, &["certificate"])
        .filter(|path| !path.is_empty())
        .and_then(|path| TlsTrust::backend_certificate(Path::new(path)).ok());
    if let Some(configured) = &configured {
        if trusted(configured)? {
            return Err(refused(
                "backend_certificate_already_trusted",
                "This server's certificate already matches the certificate trusted for this backend",
                "There is nothing to pin.",
            ));
        }
    }
    if let Some(reason) = presented.pin_refusal() {
        let (message, hint) = match reason {
            PinRefusal::Expired => (
                "This server's certificate has expired",
                "Renew the certificate on the server, then trust the new one.",
            ),
            PinRefusal::NotYetValid => (
                "This server's certificate is not valid yet",
                "Check the clock on this computer and on the server.",
            ),
            PinRefusal::Unsupported => (
                "This server's certificate cannot be used as a pinned certificate",
                "Reissue the server certificate as an ordinary TLS server certificate.",
            ),
        };
        return Err(
            refused("backend_certificate_unpinnable", message, hint).with_details(details(json!({
                "reason": match reason {
                    PinRefusal::Expired => "expired",
                    PinRefusal::NotYetValid => "not_yet_valid",
                    PinRefusal::Unsupported => "unsupported",
                },
                "host": presented.host(),
                "names": presented.summary().map(|s| s.names).unwrap_or_default(),
            }))),
        );
    }
    let previous = configured
        .as_ref()
        .and_then(|trust| trust.certificate_ders().first().map(|der| fingerprint(der)));
    let clears_insecure = config::field(config, &["insecure"])
        .map(config::boolean)
        .transpose()?
        .unwrap_or(false);
    let fingerprint = fingerprint(presented.leaf_der());
    Ok(Fetched {
        backend: name,
        kind,
        port: backend.request().endpoint().port(),
        presented,
        fingerprint,
        previous,
        clears_insecure,
    })
}

/// Require that `config` (a backend resolution after the pin was saved)
/// connects with exactly this certificate as its backend trust and with
/// verification on: no environment override, insecure flag or other
/// certificate may leave the pin unused while the trust reports success.
pub fn verify_pinned(config: &Value, fetched: &Fetched) -> Result<()> {
    let not_used = || {
        AppError::new(
            "backend_certificate_not_effective",
            "The certificate was saved, but this backend would not connect with it",
        )
        .with_hint("Check the backend's certificate and insecure settings, including environment overrides, then trust the certificate again.")
    };
    let backend = Backend::new(config, fetched.kind == "electrum").map_err(|_| not_used())?;
    let trust = backend.request().tls();
    if !backend.secure
        || matches!(trust, TlsTrust::ExplicitInsecure | TlsTrust::Plaintext)
        || trust.certificate_ders() != [fetched.der()]
    {
        return Err(not_used());
    }
    Ok(())
}

/// Fetch again and require the fingerprint the user confirmed. A mismatch
/// writes nothing: the server changed certificates between the two fetches,
/// or the user confirmed a different server's fingerprint.
pub fn confirm(
    gate: &Gate,
    consent: &CertificateConsent,
    config: &Value,
    expected: &str,
) -> Result<Fetched> {
    let expected = normalize_fingerprint(expected)?;
    let fetched = fetch(gate, consent, config)?;
    if fetched.fingerprint != expected {
        return Err(AppError::new(
            "backend_certificate_mismatch",
            "The server now presents a different certificate than the fingerprint you confirmed",
        )
        .with_hint("Nothing was trusted. Fetch the certificate again and compare its fingerprint with the server before confirming.")
        .with_details(details(json!({
            "host": fetched.host(),
            "port": fetched.port(),
            "fingerprint_sha256": fetched.fingerprint,
            "expected_fingerprint_sha256": expected,
        }))));
    }
    Ok(fetched)
}

#[cfg(test)]
mod tests;
