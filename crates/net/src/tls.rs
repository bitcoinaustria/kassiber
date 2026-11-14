//! rustls is a protocol engine only: it never receives an unguarded socket.
use crate::{Channel, Connection, Error, Transport};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{
    ClientConfig, ClientConnection, DigitallySignedStruct, RootCertStore, SignatureScheme,
};
use std::fmt;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

const MAX_CA_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RootSource {
    Host,
    Operator,
    Backend,
}

/// An immutable snapshot, binding consent to the actual selected CA certificates.
/// Debug omits paths and certificate contents.
#[derive(Clone, PartialEq, Eq)]
pub struct RootCertificates {
    source: RootSource,
    certificates: Arc<[CertificateDer<'static>]>,
}

impl fmt::Debug for RootCertificates {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RootCertificates")
            .field("source", &self.source)
            .field("count", &self.certificates.len())
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TlsTrust {
    Plaintext,
    /// Embedded Mozilla roots; no runtime download or discovery.
    BundledRoots,
    Roots(RootCertificates),
    /// Only a reviewed backend may select this. Handshake signatures still verify.
    ExplicitInsecure,
}

impl TlsTrust {
    /// Match the frozen Python bootstrap: explicit SSL_CERT_FILE (no fallback),
    /// then the first loadable Linux host bundle, then bundled Mozilla roots.
    /// This performs bounded local reads only. Resolve before minting consent.
    pub fn python_default() -> Result<Self, Error> {
        const LINUX_HOST_BUNDLES: [&str; 4] = [
            "/etc/pki/ca-trust/extracted/pem/tls-ca-bundle.pem",
            "/etc/pki/tls/certs/ca-bundle.crt",
            "/etc/ssl/certs/ca-certificates.crt",
            "/etc/ssl/cert.pem",
        ];
        let host_bundles: Vec<&Path> = if cfg!(target_os = "linux") {
            LINUX_HOST_BUNDLES.iter().map(Path::new).collect()
        } else {
            Vec::new()
        };
        let explicit = std::env::var_os("SSL_CERT_FILE").filter(|p| !p.is_empty());
        Self::select_python_roots(explicit.as_deref().map(Path::new), &host_bundles)
    }

    pub(crate) fn select_python_roots(
        explicit: Option<&Path>,
        host_bundles: &[&Path],
    ) -> Result<Self, Error> {
        if let Some(path) = explicit {
            return Self::ca_file(path);
        }
        for path in host_bundles {
            if let Ok(trust) = Self::from_file(path, RootSource::Host) {
                return Ok(trust);
            }
        }
        Ok(Self::BundledRoots)
    }

    /// Native platform trust for non-frozen/source behavior. Empty/failed stores
    /// refuse; never silently substitute Mozilla roots for an operator's store.
    ///
    /// `rustls_native_certs` reads two environment variables first: when
    /// `SSL_CERT_FILE` is set (even to an empty value) or `SSL_CERT_DIR` names
    /// at least one directory, the roots come only from that file and those
    /// directories, never from the platform store. A missing or unreadable
    /// file or directory refuses, as does a set without a certificate.
    /// `python_default` instead ignores an empty `SSL_CERT_FILE` and never
    /// reads `SSL_CERT_DIR`. No product path calls this yet.
    pub fn platform_roots() -> Result<Self, Error> {
        let result = rustls_native_certs::load_native_certs();
        if !result.errors.is_empty() {
            return Err(Error::TrustInvalid);
        }
        Self::from_certificates(result.certs, RootSource::Host)
    }

    pub fn ca_file(path: &Path) -> Result<Self, Error> {
        Self::from_file(path, RootSource::Operator)
    }

    pub fn backend_certificate(path: &Path) -> Result<Self, Error> {
        Self::from_file(path, RootSource::Backend)
    }

    /// Accept PEM contents separately from the path for stored backend settings.
    pub fn backend_pem(pem: &[u8]) -> Result<Self, Error> {
        Self::from_pem(pem, RootSource::Backend)
    }

    fn from_file(path: &Path, source: RootSource) -> Result<Self, Error> {
        let mut pem = Vec::new();
        std::fs::File::open(path)
            .and_then(|f| f.take(MAX_CA_BYTES + 1).read_to_end(&mut pem))
            .map_err(|_| Error::TrustInvalid)?;
        Self::from_pem(&pem, source)
    }

    fn from_pem(pem: &[u8], source: RootSource) -> Result<Self, Error> {
        if pem.len() as u64 > MAX_CA_BYTES {
            return Err(Error::TrustInvalid);
        }
        let certificates = CertificateDer::pem_slice_iter(pem)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| Error::TrustInvalid)?;
        Self::from_certificates(certificates, source)
    }

    fn from_certificates(
        certificates: Vec<CertificateDer<'static>>,
        source: RootSource,
    ) -> Result<Self, Error> {
        if certificates.is_empty() {
            return Err(Error::TrustInvalid);
        }
        let mut roots = RootCertStore::empty();
        for certificate in &certificates {
            roots
                .add(certificate.clone())
                .map_err(|_| Error::TrustInvalid)?;
        }
        Ok(Self::Roots(RootCertificates {
            source,
            certificates: certificates.into(),
        }))
    }
}

fn provider() -> Arc<CryptoProvider> {
    // Fix the provider explicitly: workspace feature unification cannot change it.
    Arc::new(rustls::crypto::ring::default_provider())
}

/// The server-certificate verifier for one trust selection. Plaintext has none.
fn verifier(
    trust: &TlsTrust,
    provider: Arc<CryptoProvider>,
) -> Result<Arc<dyn ServerCertVerifier>, Error> {
    let mut roots = RootCertStore::empty();
    let snapshot = match trust {
        TlsTrust::Plaintext => return Err(Error::TrustInvalid),
        TlsTrust::ExplicitInsecure => return Ok(Arc::new(InsecureVerifier(provider))),
        TlsTrust::BundledRoots => {
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            None
        }
        TlsTrust::Roots(snapshot) => {
            for certificate in snapshot.certificates.iter() {
                roots
                    .add(certificate.clone())
                    .map_err(|_| Error::TrustInvalid)?;
            }
            Some(snapshot)
        }
    };
    match snapshot {
        Some(snapshot) if snapshot.source == RootSource::Backend => {
            backend_verifier(roots, provider, snapshot.certificates.clone())
        }
        _ => Ok(Arc::new(ClassifyingVerifier(webpki_verifier(
            roots, provider,
        )?))),
    }
}

fn webpki_verifier(
    roots: RootCertStore,
    provider: Arc<CryptoProvider>,
) -> Result<Arc<rustls::client::WebPkiServerVerifier>, Error> {
    // The same verifier `with_root_certificates` builds: no revocation lists.
    rustls::client::WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider)
        .build()
        .map_err(|_| Error::TrustInvalid)
}

fn client_config(
    verifier: Arc<dyn ServerCertVerifier>,
    provider: Arc<CryptoProvider>,
    http: bool,
) -> Result<Arc<ClientConfig>, Error> {
    let mut config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|_| Error::Tls)?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    if http {
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
    }
    // No key logger, client certificate, early data or cross-route session cache.
    config.enable_early_data = false;
    config.resumption = rustls::client::Resumption::disabled();
    Ok(Arc::new(config))
}

pub(crate) fn config(trust: &TlsTrust, http: bool) -> Result<Option<Arc<ClientConfig>>, Error> {
    if *trust == TlsTrust::Plaintext {
        return Ok(None);
    }
    let provider = provider();
    client_config(verifier(trust, provider.clone())?, provider, http).map(Some)
}

pub(crate) fn wrap(
    mut transport: Transport,
    config: Option<Arc<ClientConfig>>,
) -> Result<Connection, Error> {
    let Some(config) = config else {
        return Ok(Connection {
            channel: Channel::Plain(transport),
        });
    };
    let name = ServerName::try_from(transport.route.request.endpoint.host.clone())
        .map_err(|_| Error::InvalidEndpoint)?;
    let mut client = ClientConnection::new(config, name).map_err(|_| Error::Tls)?;
    while client.is_handshaking() {
        transport.route.check()?;
        client
            .complete_io(&mut transport)
            .map_err(handshake_error)?;
    }
    Ok(Connection {
        channel: Channel::Tls(Box::new(rustls::StreamOwned::new(client, transport))),
    })
}

fn backend_verifier(
    roots: RootCertStore,
    provider: Arc<CryptoProvider>,
    pins: Arc<[CertificateDer<'static>]>,
) -> Result<Arc<dyn ServerCertVerifier>, Error> {
    let fallback = webpki_verifier(roots, provider)?;
    Ok(Arc::new(PinnedVerifier { pins, fallback }))
}

/// Why a server certificate failed, when the reason lets the user act on it.
/// It travels inside rustls's error and never carries certificate contents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TrustFailure {
    /// No configured certificate: the chain reaches no trusted root.
    Untrusted,
    /// This backend has a certificate, and the server presented another one.
    Changed,
}

impl fmt::Display for TrustFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Untrusted => "certificate not trusted",
            Self::Changed => "certificate changed",
        })
    }
}

impl std::error::Error for TrustFailure {}

/// Mark a chain failure as untrusted or changed. Only an unknown issuer, or a
/// self-issued certificate webpki refuses before looking for an issuer (a
/// self-signed CA:TRUE leaf), qualifies; expiry, name and other faults keep
/// rustls's error and stay generic TLS failures.
fn classify(error: rustls::Error, cert: &CertificateDer<'_>, pinned: bool) -> rustls::Error {
    use rustls::CertificateError;
    let issuer = match &error {
        rustls::Error::InvalidCertificate(CertificateError::UnknownIssuer) => true,
        rustls::Error::InvalidCertificate(CertificateError::Other(_)) => {
            crate::pinned::self_issued(cert)
        }
        _ => false,
    };
    if !issuer {
        return error;
    }
    let failure = if pinned {
        TrustFailure::Changed
    } else {
        TrustFailure::Untrusted
    };
    rustls::Error::InvalidCertificate(CertificateError::Other(rustls::OtherError(Arc::new(
        failure,
    ))))
}

/// A handshake failure as a typed error: a classified certificate failure,
/// else the usual I/O mapping (which recovers gate refusals).
fn handshake_error(error: std::io::Error) -> Error {
    let failure = error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>())
        .and_then(|inner| match inner {
            rustls::Error::InvalidCertificate(rustls::CertificateError::Other(other)) => {
                other.0.downcast_ref::<TrustFailure>().copied()
            }
            _ => None,
        });
    match failure {
        Some(TrustFailure::Untrusted) => Error::CertificateUntrusted,
        Some(TrustFailure::Changed) => Error::CertificateChanged,
        None => Error::from(error),
    }
}

/// Webpki verification that reports an untrusted chain as such.
#[derive(Debug)]
struct ClassifyingVerifier(Arc<rustls::client::WebPkiServerVerifier>);

impl ServerCertVerifier for ClassifyingVerifier {
    fn verify_server_cert(
        &self,
        cert: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        self.0
            .verify_server_cert(cert, intermediates, name, ocsp, now)
            .map_err(|error| classify(error, cert, false))
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls12_signature(message, cert, dss)
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls13_signature(message, cert, dss)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_verify_schemes()
    }
}

#[derive(Debug)]
struct PinnedVerifier {
    pins: Arc<[CertificateDer<'static>]>,
    fallback: Arc<rustls::client::WebPkiServerVerifier>,
}

impl ServerCertVerifier for PinnedVerifier {
    fn verify_server_cert(
        &self,
        cert: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if self.pins.iter().any(|pin| pin.as_ref() == cert.as_ref()) {
            crate::pinned::verify(cert, name, now)?;
            Ok(ServerCertVerified::assertion())
        } else {
            self.fallback
                .verify_server_cert(cert, intermediates, name, ocsp, now)
                .map_err(|error| classify(error, cert, true))
        }
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.fallback.verify_tls12_signature(message, cert, dss)
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.fallback.verify_tls13_signature(message, cert, dss)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.fallback.supported_verify_schemes()
    }
}

/// Why a presented certificate cannot become this backend's pin: the pin
/// path still requires it to be in date and to name the configured host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinRefusal {
    NotYetValid,
    Expired,
    NameMismatch,
    /// The certificate could not be read as a server certificate.
    Unsupported,
}

/// Display fields of a presented leaf. Times are seconds since the epoch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CertificateSummary {
    pub subject: Option<String>,
    pub issuer: Option<String>,
    pub not_before: i64,
    pub not_after: i64,
    pub self_issued: bool,
    /// DNS and IP subject alternative names, in certificate order.
    pub names: Vec<String>,
}

/// The chain a server presented during a handshake Kassiber aborted. The
/// server proved it holds the leaf's key (its handshake signature verified),
/// but nothing here is trusted: it exists for the user's decision only.
/// Debug omits the certificates.
#[derive(Clone)]
pub struct PresentedCertificate {
    host: String,
    chain: Vec<CertificateDer<'static>>,
}

impl fmt::Debug for PresentedCertificate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PresentedCertificate")
            .field("host", &self.host)
            .field("chain_len", &self.chain.len())
            .finish()
    }
}

impl PresentedCertificate {
    /// The configured host the certificate was presented for.
    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn leaf_der(&self) -> &[u8] {
        self.chain[0].as_ref()
    }

    /// The leaf alone as one PEM block, the form a backend certificate file takes.
    pub fn leaf_pem(&self) -> String {
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(self.leaf_der());
        let mut pem = String::from("-----BEGIN CERTIFICATE-----\n");
        for line in encoded.as_bytes().chunks(64) {
            pem.push_str(std::str::from_utf8(line).expect("base64 is ASCII"));
            pem.push('\n');
        }
        pem.push_str("-----END CERTIFICATE-----\n");
        pem
    }

    pub fn summary(&self) -> Option<CertificateSummary> {
        crate::pinned::describe(&self.chain[0]).ok()
    }

    /// Whether `trust` accepts this chain for the host now, as a connection
    /// would. Plaintext and explicitly insecure trust decide nothing here.
    pub fn trusted_by(&self, trust: &TlsTrust) -> Result<bool, Error> {
        if matches!(trust, TlsTrust::Plaintext | TlsTrust::ExplicitInsecure) {
            return Err(Error::TrustInvalid);
        }
        let name = ServerName::try_from(self.host.as_str()).map_err(|_| Error::InvalidEndpoint)?;
        Ok(verifier(trust, provider())?
            .verify_server_cert(
                &self.chain[0],
                &self.chain[1..],
                &name,
                &[],
                UnixTime::now(),
            )
            .is_ok())
    }

    /// `None` when a pin of this leaf would verify for the host now.
    pub fn pin_refusal(&self) -> Option<PinRefusal> {
        let Ok(name) = ServerName::try_from(self.host.as_str()) else {
            return Some(PinRefusal::Unsupported);
        };
        let now = UnixTime::now();
        if crate::pinned::verify(&self.chain[0], &name, now).is_ok() {
            return None;
        }
        let Some(summary) = self.summary() else {
            return Some(PinRefusal::Unsupported);
        };
        let now = i64::try_from(now.as_secs()).unwrap_or(i64::MAX);
        Some(if now < summary.not_before {
            PinRefusal::NotYetValid
        } else if now > summary.not_after {
            PinRefusal::Expired
        } else if rustls::server::ParsedCertificate::try_from(&self.chain[0]).is_err() {
            PinRefusal::Unsupported
        } else {
            PinRefusal::NameMismatch
        })
    }
}

/// Records the presented chain, checks the server's handshake signature over
/// it, and then aborts the handshake. It never reports a certificate as
/// trusted to a completed connection: every path ends in an error.
#[derive(Debug)]
struct CaptureVerifier {
    provider: Arc<CryptoProvider>,
    chain: std::sync::Mutex<Option<Vec<CertificateDer<'static>>>>,
    proven: std::sync::atomic::AtomicBool,
}

/// The handshake ended on purpose after the chain was recorded.
#[derive(Debug)]
struct Captured;

impl fmt::Display for Captured {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("certificate captured")
    }
}

impl std::error::Error for Captured {}

impl CaptureVerifier {
    fn prove(
        &self,
        cert: &CertificateDer<'_>,
        verified: Result<HandshakeSignatureValid, rustls::Error>,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verified?;
        let chain = self.chain.lock().unwrap_or_else(|e| e.into_inner());
        if chain
            .as_ref()
            .is_some_and(|chain| chain[0].as_ref() == cert.as_ref())
        {
            self.proven.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        Err(rustls::Error::InvalidCertificate(
            rustls::CertificateError::Other(rustls::OtherError(Arc::new(Captured))),
        ))
    }
}

impl ServerCertVerifier for CaptureVerifier {
    fn verify_server_cert(
        &self,
        cert: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let chain = std::iter::once(cert)
            .chain(intermediates)
            .map(|c| c.clone().into_owned())
            .collect();
        *self.chain.lock().unwrap_or_else(|e| e.into_inner()) = Some(chain);
        // Continue only so the server's signature over this handshake can be
        // checked; the signature callback always aborts.
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.prove(
            cert,
            verify_tls12_signature(
                message,
                cert,
                dss,
                &self.provider.signature_verification_algorithms,
            ),
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.prove(
            cert,
            verify_tls13_signature(
                message,
                cert,
                dss,
                &self.provider.signature_verification_algorithms,
            ),
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// One capture's verifier and the client configuration that uses it.
pub(crate) struct Capture(Arc<CaptureVerifier>);

impl Capture {
    pub(crate) fn new(http: bool) -> Result<(Self, Arc<ClientConfig>), Error> {
        let provider = provider();
        let verifier = Arc::new(CaptureVerifier {
            provider: provider.clone(),
            chain: std::sync::Mutex::new(None),
            proven: std::sync::atomic::AtomicBool::new(false),
        });
        let config = client_config(verifier.clone(), provider, http)?;
        Ok((Self(verifier), config))
    }

    /// The chain, only once the server's handshake signature verified with it.
    pub(crate) fn take(&self, host: &str) -> Option<PresentedCertificate> {
        if !self.0.proven.load(std::sync::atomic::Ordering::SeqCst) {
            return None;
        }
        let chain = self
            .0
            .chain
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()?;
        Some(PresentedCertificate {
            host: host.to_owned(),
            chain,
        })
    }
}

#[derive(Debug)]
struct InsecureVerifier(Arc<CryptoProvider>);

impl ServerCertVerifier for InsecureVerifier {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const PINS: [&[u8]; 3] = [
        include_bytes!("../tests/fixtures/lnd-ca.pem"),
        include_bytes!("../tests/fixtures/openssl-ca.pem"),
        include_bytes!("../tests/fixtures/cn-only.pem"),
    ];

    #[test]
    fn backend_pins_accept_python_self_signed_shapes_and_refuse_wrong_names() {
        for pem in PINS {
            let cert = CertificateDer::from_pem_slice(pem).unwrap();
            let mut roots = RootCertStore::empty();
            roots.add(cert.clone()).unwrap();
            let verifier = backend_verifier(
                roots,
                Arc::new(rustls::crypto::ring::default_provider()),
                vec![cert.clone()].into(),
            )
            .unwrap();
            let name = ServerName::try_from("localhost").unwrap();
            verifier
                .verify_server_cert(&cert, &[], &name, &[], UnixTime::now())
                .unwrap();
            for name in ["wrong.invalid", "127.0.0.1"] {
                assert!(verifier
                    .verify_server_cert(
                        &cert,
                        &[],
                        &ServerName::try_from(name).unwrap(),
                        &[],
                        UnixTime::now()
                    )
                    .is_err());
            }
            assert!(verifier
                .verify_server_cert(
                    &cert,
                    &[],
                    &name,
                    &[],
                    UnixTime::since_unix_epoch(std::time::Duration::ZERO)
                )
                .is_err());
            assert!(verifier
                .verify_server_cert(
                    &cert,
                    &[],
                    &name,
                    &[],
                    UnixTime::since_unix_epoch(std::time::Duration::from_secs(10_000_000_000))
                )
                .is_err());
        }
    }
    #[test]
    fn pin_verifier_keeps_chain_and_handshake_signature_verification() {
        use rustls::internal::msgs::codec::{Codec, Reader};
        use rustls::pki_types::PrivateKeyDer;
        let cert = CertificateDer::from_pem_slice(PINS[0]).unwrap();
        let mut roots = RootCertStore::empty();
        roots.add(cert.clone()).unwrap();
        let verifier = backend_verifier(
            roots,
            Arc::new(rustls::crypto::ring::default_provider()),
            vec![cert.clone()].into(),
        )
        .unwrap();
        let key = PrivateKeyDer::from_pem_slice(include_bytes!("../tests/fixtures/lnd-ca-key.pem"))
            .unwrap();
        let signer = rustls::crypto::ring::sign::any_supported_type(&key)
            .unwrap()
            .choose_scheme(&[SignatureScheme::RSA_PSS_SHA256])
            .unwrap();
        let signature = signer.sign(b"handshake").unwrap();
        let mut encoded = Vec::new();
        signer.scheme().encode(&mut encoded);
        encoded.extend_from_slice(&(signature.len() as u16).to_be_bytes());
        encoded.extend_from_slice(&signature);
        let signed = DigitallySignedStruct::read(&mut Reader::init(&encoded)).unwrap();
        verifier
            .verify_tls12_signature(b"handshake", &cert, &signed)
            .unwrap();
        verifier
            .verify_tls13_signature(b"handshake", &cert, &signed)
            .unwrap();
        assert!(verifier
            .verify_tls12_signature(b"tampered", &cert, &signed)
            .is_err());
        assert!(verifier
            .verify_tls13_signature(b"tampered", &cert, &signed)
            .is_err());
        let other = CertificateDer::from_pem_slice(PINS[1]).unwrap();
        assert!(verifier
            .verify_server_cert(
                &other,
                &[],
                &ServerName::try_from("localhost").unwrap(),
                &[],
                UnixTime::now()
            )
            .is_err());
        let conflict =
            CertificateDer::from_pem_slice(include_bytes!("../tests/fixtures/cn-san-conflict.pem"))
                .unwrap();
        assert!(crate::pinned::verify(
            &conflict,
            &ServerName::try_from("localhost").unwrap(),
            UnixTime::now()
        )
        .is_err());
        let ca =
            CertificateDer::from_pem_slice(include_bytes!("../tests/fixtures/ca.pem")).unwrap();
        let mut roots = RootCertStore::empty();
        roots.add(ca.clone()).unwrap();
        let verifier = backend_verifier(
            roots,
            Arc::new(rustls::crypto::ring::default_provider()),
            vec![ca].into(),
        )
        .unwrap();
        let leaf =
            CertificateDer::from_pem_slice(include_bytes!("../tests/fixtures/server.pem")).unwrap();
        verifier
            .verify_server_cert(
                &leaf,
                &[],
                &ServerName::try_from("localhost").unwrap(),
                &[],
                UnixTime::now(),
            )
            .unwrap();
    }
}
