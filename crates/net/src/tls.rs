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
        Self::select_python_roots(
            std::env::var_os("SSL_CERT_FILE")
                .filter(|p| !p.is_empty())
                .as_deref()
                .map(Path::new),
            if cfg!(target_os = "linux") {
                &[
                    Path::new("/etc/pki/ca-trust/extracted/pem/tls-ca-bundle.pem"),
                    Path::new("/etc/pki/tls/certs/ca-bundle.crt"),
                    Path::new("/etc/ssl/certs/ca-certificates.crt"),
                    Path::new("/etc/ssl/cert.pem"),
                ]
            } else {
                &[]
            },
        )
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

pub(crate) fn config(trust: &TlsTrust, http: bool) -> Result<Option<Arc<ClientConfig>>, Error> {
    if *trust == TlsTrust::Plaintext {
        return Ok(None);
    }
    // Fix the provider explicitly: workspace feature unification cannot change it.
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|_| Error::Tls)?;
    let mut config = if *trust == TlsTrust::ExplicitInsecure {
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(InsecureVerifier(provider)))
            .with_no_client_auth()
    } else {
        let mut roots = RootCertStore::empty();
        match trust {
            TlsTrust::BundledRoots => roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned()),
            TlsTrust::Roots(snapshot) => {
                for certificate in snapshot.certificates.iter() {
                    roots
                        .add(certificate.clone())
                        .map_err(|_| Error::TrustInvalid)?;
                }
            }
            _ => unreachable!(),
        }
        builder.with_root_certificates(roots).with_no_client_auth()
    };
    if http {
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
    }
    // No key logger, client certificate, early data or cross-route session cache.
    config.enable_early_data = false;
    config.resumption = rustls::client::Resumption::disabled();
    Ok(Some(Arc::new(config)))
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
        transport.prepare()?;
        client.complete_io(&mut transport)?;
    }
    Ok(Connection {
        channel: Channel::Tls(Box::new(rustls::StreamOwned::new(client, transport))),
    })
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
