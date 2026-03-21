//! TLS certificate generation for the HTTP/3 (QUIC) server.
//!
//! Generates a self-signed certificate valid for `127.0.0.1` and `localhost`.
//! The certificate DER bytes are exported so test clients can add them to their
//! trust stores via `reqwest::ClientBuilder::add_root_certificate()`.

use anyhow::{Context, Result};
use std::sync::Arc;

/// Self-signed TLS certificate and private key for the QUIC server.
pub struct SelfSignedCert {
    /// DER-encoded certificate bytes (for rustls and for exporting to clients).
    pub cert_der: Vec<u8>,
    /// DER-encoded private key bytes (for rustls).
    pub key_der: Vec<u8>,
}

/// Generate a self-signed TLS certificate valid for `127.0.0.1` and `localhost`.
pub fn generate_self_signed_cert() -> Result<SelfSignedCert> {
    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_string(), "localhost".to_string()])
            .context("Failed to generate self-signed certificate")?;

    let cert_der = cert.der().to_vec();
    let key_der = key_pair.serialize_der();

    Ok(SelfSignedCert { cert_der, key_der })
}

/// Build a rustls `ServerConfig` suitable for HTTP/3 (QUIC) with the given certificate.
pub fn build_quic_tls_config(cert: &SelfSignedCert) -> Result<Arc<rustls::ServerConfig>> {
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};

    let cert_der = CertificateDer::from(cert.cert_der.clone());
    let key_der = PrivateKeyDer::try_from(cert.key_der.clone())
        .map_err(|e| anyhow::anyhow!("Invalid private key DER: {}", e))?;

    // Explicitly select the ring crypto provider to avoid ambiguity when both
    // `ring` and `aws-lc-rs` features are enabled by transitive dependencies.
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut tls_config = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .context("Failed to build TLS server config with TLS 1.3")?
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der)
        .context("Failed to build TLS server config")?;

    // HTTP/3 requires the h3 ALPN token; 0-RTT reduces latency on reconnect.
    tls_config.max_early_data_size = u32::MAX;
    tls_config.alpn_protocols = vec![b"h3".to_vec()];

    Ok(Arc::new(tls_config))
}
