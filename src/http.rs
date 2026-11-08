//! The HTTPS connection to opensecmon: HTTP/1.1 over rustls, one connection per request.
//!
//! The certificate is accepted if its SHA-256 fingerprint is the one from the enrollment code, **or** if it is
//! valid for the address by the normal verification of the system (opensecmon behind a proxy with a certificate
//! from a public or the company's CA). Either way the TLS handshake proves that the server holds the private key.
//! A refused certificate is reported with its fingerprint, and nothing is sent.

use std::net::Ipv6Addr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Bytes;
use hyper::header::{self, HeaderValue};
use hyper_util::rt::TokioIo;
use sha2::{Digest, Sha256};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use tokio_rustls::rustls::crypto::{
    CryptoProvider, WebPkiSupportedAlgorithms, ring, verify_tls12_signature, verify_tls13_signature,
};
use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use tokio_rustls::rustls::{
    self, CertificateError, ClientConfig, DigitallySignedStruct, SignatureScheme,
};

use crate::code;
use crate::store::Token;

/// Time for connecting to one address.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Time for a whole request, from looking up the name to the last byte of the answer. A report of a large
/// computer is a few hundred KB compressed; opensecmon answers before it compares.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
/// Answers of the API are small JSON documents.
const RESPONSE_MAX: usize = 1024 * 1024;
/// Longest wait opensecmon can ask for with `Retry-After`.
const RETRY_AFTER_MAX: u64 = 24 * 3600;

/// Why a request did not get an answer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HttpError {
    #[error("the address of opensecmon is not valid")]
    InvalidAddress,
    #[error("the name {0} could not be looked up: {1}")]
    Resolve(String, String),
    #[error("opensecmon could not be reached: {0}")]
    Connect(String),
    #[error(
        "the certificate of opensecmon was refused ({reason}); nothing was sent. Fingerprint offered: {offered}"
    )]
    Untrusted { offered: String, reason: String },
    #[error("the secure connection failed: {0}")]
    Tls(String),
    #[error("the answer of opensecmon could not be read: {0}")]
    Protocol(String),
    #[error("opensecmon did not answer in time")]
    Timeout,
    #[error("the answer of opensecmon is too large")]
    TooLarge,
}

/// Where opensecmon is: the host to connect to and to check the certificate for, and the `Host` header, which
/// opensecmon checks against `allowed_hosts` (the address exactly as the browser used it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub host: String,
    pub port: u16,
    pub authority: String,
}

impl Target {
    /// Reads `host`, `host:port`, `192.0.2.10:8444`, `[fd00::2]` or `[fd00::2]:8444`; the port defaults to 443.
    pub fn parse(address: &str) -> Option<Self> {
        if !code::is_address(address) {
            return None;
        }
        let (host, port) = if let Some(rest) = address.strip_prefix('[') {
            let (host, after) = rest.split_once(']')?;
            host.parse::<Ipv6Addr>().ok()?;
            let port = match after {
                "" => 443,
                p => p.strip_prefix(':')?.parse().ok()?,
            };
            (host, port)
        } else if address.contains(['[', ']']) {
            return None;
        } else {
            match address.split_once(':') {
                None => (address, 443),
                // A second colon would be an IPv6 address without brackets: ambiguous.
                Some((host, port)) => (host, port.parse().ok()?),
            }
        };
        (!host.is_empty() && port != 0).then(|| Self {
            host: host.to_owned(),
            port,
            authority: address.to_owned(),
        })
    }
}

/// A request body.
pub struct Body {
    pub content_type: &'static str,
    pub gzip: bool,
    pub bytes: Vec<u8>,
}

/// An answer: status, the wait asked for with `Retry-After`, and the body.
#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub retry_after: Option<u64>,
    pub body: Vec<u8>,
}

/// A client for one opensecmon.
pub struct Client {
    target: Target,
    verifier: Arc<PinOrSystem>,
    config: Arc<ClientConfig>,
}

impl Client {
    /// A client that trusts the certificate with `pin` or one valid for the address by the system's trust store.
    pub fn new(address: &str, pin: Option<[u8; 32]>) -> Result<Self, HttpError> {
        let system = rustls_platform_verifier::Verifier::new(provider())
            .map_err(|e| HttpError::Tls(e.to_string()))?;
        Self::with_system_verifier(address, pin, Arc::new(system))
    }

    /// As [`Client::new`] with another verifier in place of the system's (tests use their own CA).
    pub fn with_system_verifier(
        address: &str,
        pin: Option<[u8; 32]>,
        system: Arc<dyn ServerCertVerifier>,
    ) -> Result<Self, HttpError> {
        let target = Target::parse(address).ok_or(HttpError::InvalidAddress)?;
        let verifier = Arc::new(PinOrSystem {
            pin,
            system,
            algorithms: provider().signature_verification_algorithms,
            refused: Mutex::new(None),
        });
        let mut config = ClientConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .map_err(|e| HttpError::Tls(e.to_string()))?
            .dangerous()
            .with_custom_certificate_verifier(verifier.clone())
            .with_no_client_auth();
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        Ok(Self {
            target,
            verifier,
            config: Arc::new(config),
        })
    }

    /// Sends one request (`GET` or `POST`) to `path` and reads the answer.
    pub async fn send(
        &self,
        method: hyper::Method,
        path: &str,
        token: Option<&Token>,
        body: Option<Body>,
    ) -> Result<Response, HttpError> {
        tokio::time::timeout(REQUEST_TIMEOUT, self.exchange(method, path, token, body))
            .await
            .unwrap_or(Err(HttpError::Timeout))
    }

    async fn exchange(
        &self,
        method: hyper::Method,
        path: &str,
        token: Option<&Token>,
        body: Option<Body>,
    ) -> Result<Response, HttpError> {
        let tcp = self.connect().await?;
        let name = ServerName::try_from(self.target.host.clone())
            .map_err(|_| HttpError::InvalidAddress)?;
        let tls = match TlsConnector::from(self.config.clone())
            .connect(name, tcp)
            .await
        {
            Ok(tls) => tls,
            Err(e) => {
                return Err(match self.verifier.take_refused() {
                    Some((seen, reason)) => HttpError::Untrusted {
                        offered: code::format_fingerprint(&seen),
                        reason,
                    },
                    None => HttpError::Tls(e.to_string()),
                });
            }
        };
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
            .await
            .map_err(|e| HttpError::Protocol(e.to_string()))?;
        let task = AbortOnDrop(tokio::spawn(async move {
            let _ = connection.await;
        }));

        let mut request = hyper::Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, &self.target.authority)
            .header(
                header::USER_AGENT,
                concat!("opensecagent/", env!("CARGO_PKG_VERSION")),
            )
            .header(header::ACCEPT, "application/json");
        if let Some(token) = token {
            let mut value = HeaderValue::try_from(format!("Bearer {}", token.expose()))
                .map_err(|_| HttpError::Protocol("token".into()))?;
            value.set_sensitive(true);
            request = request.header(header::AUTHORIZATION, value);
        }
        let bytes = match body {
            Some(body) => {
                request = request.header(header::CONTENT_TYPE, body.content_type);
                if body.gzip {
                    request = request.header(header::CONTENT_ENCODING, "gzip");
                }
                body.bytes
            }
            None => Vec::new(),
        };
        let request = request
            .body(Full::new(Bytes::from(bytes)))
            .map_err(|e| HttpError::Protocol(e.to_string()))?;
        let response = sender
            .send_request(request)
            .await
            .map_err(|e| HttpError::Protocol(e.to_string()))?;
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get(header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map(|s| s.min(RETRY_AFTER_MAX));
        let body = Limited::new(response.into_body(), RESPONSE_MAX)
            .collect()
            .await
            .map_err(|e| {
                if e.downcast_ref::<http_body_util::LengthLimitError>()
                    .is_some()
                {
                    HttpError::TooLarge
                } else {
                    HttpError::Protocol(e.to_string())
                }
            })?
            .to_bytes()
            .to_vec();
        drop(task);
        Ok(Response {
            status,
            retry_after,
            body,
        })
    }

    /// Connects to the first address of the host that answers.
    async fn connect(&self) -> Result<TcpStream, HttpError> {
        let Target { host, port, .. } = &self.target;
        let addresses = tokio::net::lookup_host((host.as_str(), *port))
            .await
            .map_err(|e| HttpError::Resolve(host.clone(), e.to_string()))?;
        let mut last = String::from("no address");
        for address in addresses {
            match tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(address)).await {
                Ok(Ok(stream)) => return Ok(stream),
                Ok(Err(e)) => last = format!("{address}: {e}"),
                Err(_) => last = format!("{address}: timed out"),
            }
        }
        Err(HttpError::Connect(last))
    }
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(ring::default_provider())
}

/// Aborts the task of a connection when the request is done or given up.
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Accepts the certificate with the fingerprint of the enrollment code, otherwise asks the system's verifier.
#[derive(Debug)]
struct PinOrSystem {
    pin: Option<[u8; 32]>,
    system: Arc<dyn ServerCertVerifier>,
    algorithms: WebPkiSupportedAlgorithms,
    /// Fingerprint of the last refused certificate and why.
    refused: Mutex<Option<([u8; 32], String)>>,
}

impl PinOrSystem {
    fn take_refused(&self) -> Option<([u8; 32], String)> {
        self.refused.lock().map(|mut r| r.take()).unwrap_or(None)
    }
}

impl ServerCertVerifier for PinOrSystem {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let mut seen = [0u8; 32];
        seen.copy_from_slice(&Sha256::digest(end_entity.as_ref()));
        if self.pin == Some(seen) {
            return Ok(ServerCertVerified::assertion());
        }
        let result = self.system.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        );
        if let Err(e) = &result {
            let reason = if self.pin.is_some() {
                format!("not the certificate of the enrollment code, and {e}")
            } else {
                e.to_string()
            };
            if let Ok(mut refused) = self.refused.lock() {
                *refused = Some((seen, reason));
            }
            // The same error for both cases; the reason is kept above.
            return Err(rustls::Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ));
        }
        result
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_read_as_the_browser_uses_them() {
        let t = |a: &str| Target::parse(a).map(|t| (t.host, t.port, t.authority));
        assert_eq!(
            t("192.0.2.10:8444"),
            Some(("192.0.2.10".into(), 8444, "192.0.2.10:8444".into()))
        );
        assert_eq!(
            t("secmon.example.org"),
            Some((
                "secmon.example.org".into(),
                443,
                "secmon.example.org".into()
            ))
        );
        assert_eq!(
            t("[fd00::2]:8444"),
            Some(("fd00::2".into(), 8444, "[fd00::2]:8444".into()))
        );
        assert_eq!(t("[::1]"), Some(("::1".into(), 443, "[::1]".into())));
        for bad in [
            "",
            "fd00::2",
            "host:",
            "host:0",
            "host:65536",
            ":8444",
            "[fd00::2]x",
            "[nohost]:1",
            "ho[st",
            "host/path",
            "user@host",
        ] {
            assert_eq!(Target::parse(bad), None, "{bad:?}");
        }
    }
}
