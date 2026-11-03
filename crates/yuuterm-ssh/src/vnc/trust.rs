//! Pinning is additional identity policy, never a replacement for rustls validation.
use super::*;
use crate::trust::TrustStore;
use rustls::{
    DigitallySignedStruct, Error, SignatureScheme,
    client::{
        WebPkiServerVerifier,
        danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    },
    pki_types::{CertificateDer, ServerName, UnixTime},
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fmt,
    path::PathBuf,
    sync::{Mutex, RwLock},
};

/// A rejected, name/time-valid certificate requiring independent user review.
/// Approval authorizes this exact leaf as an endpoint-specific trust anchor.
#[derive(Clone, Debug)]
pub struct CertificateReview {
    pub endpoint: String,
    pub sha256: String,
    pub certificate_der: Vec<u8>,
}

#[derive(Clone)]
pub struct VncTrust {
    store: Arc<Mutex<TrustStore>>,
    pins: Arc<RwLock<BTreeMap<String, String>>>,
}
impl fmt::Debug for VncTrust {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("VncTrust")
    }
}
impl VncTrust {
    /// Use a dedicated VNC pin file, never the SSH host-key file. Load once,
    /// then clone this owner: independently loaded stores are not coordinated.
    pub async fn load(path: PathBuf) -> Result<Self> {
        tokio::task::spawn_blocking(move || {
            let store = TrustStore::load(path)?;
            let pins = store.snapshot();
            Ok(Self {
                store: Arc::new(Mutex::new(store)),
                pins: Arc::new(RwLock::new(pins)),
            })
        })
        .await?
    }

    /// Call only after an explicit independent trust decision. A saved entry
    /// cannot be replaced, including by a stale concurrent review.
    pub async fn approve(&self, review: CertificateReview) -> Result<()> {
        anyhow::ensure!(
            review.certificate_der.len() <= 65536,
            "VNC certificate exceeds limit"
        );
        anyhow::ensure!(
            !review.endpoint.is_empty() && review.endpoint.len() <= 270,
            "Invalid VNC review endpoint"
        );
        anyhow::ensure!(
            fingerprint(&review.certificate_der) == review.sha256,
            "VNC review changed"
        );
        let store = self.store.clone();
        let pins = self.pins.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = store
                .lock()
                .map_err(|_| anyhow::anyhow!("VNC trust lock failed"))?;
            store.remember(&review.endpoint, &review.sha256)?;
            *pins
                .write()
                .map_err(|_| anyhow::anyhow!("VNC trust cache failed"))? = store.snapshot();
            Ok(())
        })
        .await?
    }
}

pub(super) fn fingerprint(cert: &[u8]) -> String {
    crate::hex::lower(Sha256::digest(cert))
}

#[derive(Debug)]
pub(super) struct Verifier {
    pub roots: rustls::RootCertStore,
    pub trust: VncTrust,
    pub endpoint: String,
    pub review: Mutex<Option<CertificateReview>>,
    pub changed: Mutex<bool>,
}
impl Verifier {
    fn strict(
        roots: rustls::RootCertStore,
    ) -> std::result::Result<Arc<WebPkiServerVerifier>, Error> {
        WebPkiServerVerifier::builder_with_provider(
            Arc::new(roots),
            Arc::new(rustls::crypto::ring::default_provider()),
        )
        .build()
        .map_err(|_| Error::General("Invalid VNC trust anchors".into()))
    }
}
impl ServerCertVerifier for Verifier {
    fn verify_server_cert(
        &self,
        cert: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, Error> {
        if cert.len() > 65536
            || intermediates.len() > 16
            || intermediates.iter().map(|c| c.len()).sum::<usize>() > 262144
        {
            return Err(Error::General("VNC certificate chain exceeds limit".into()));
        }
        let sha256 = fingerprint(cert.as_ref());
        let pins = self
            .trust
            .pins
            .read()
            .map_err(|_| Error::General("VNC trust cache failed".into()))?;
        let pinned = match pins.get(&self.endpoint) {
            Some(value) if value == &sha256 => true,
            None => false,
            Some(_) => {
                *self.changed.lock().unwrap() = true;
                return Err(Error::General(
                    "VNC CERTIFICATE CHANGED; connection blocked".into(),
                ));
            }
        };
        drop(pins);
        // A valid chain/name is accepted normally, unless an existing pin changed.
        if !self.roots.is_empty() {
            let result = Self::strict(self.roots.clone())?.verify_server_cert(
                cert,
                intermediates,
                name,
                ocsp,
                now,
            );
            match result {
                Ok(valid) => return Ok(valid),
                Err(Error::InvalidCertificate(rustls::CertificateError::UnknownIssuer)) => {}
                Err(error) => return Err(error),
            }
        }
        // Even a manually pinned leaf must pass WebPKI name, time, usage and
        // algorithm checks. TLS handshake signatures are verified below too.
        let mut anchors = rustls::RootCertStore::empty();
        anchors.add(cert.clone().into_owned())?;
        let valid =
            Self::strict(anchors)?.verify_server_cert(cert, intermediates, name, ocsp, now)?;
        if pinned {
            return Ok(valid);
        }
        *self.review.lock().unwrap() = Some(CertificateReview {
            endpoint: self.endpoint.clone(),
            sha256,
            certificate_der: cert.as_ref().to_vec(),
        });
        Err(Error::General(
            "VNC certificate requires explicit trust approval".into(),
        ))
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}
