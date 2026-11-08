//! The agent against a stand-in for opensecmon: HTTPS with a self-signed certificate (pinned by the enrollment
//! code) or one from a test CA (the "system" trust store of the tests), and the behaviour of the agent API.

use std::io::Read;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use sha2::{Digest, Sha256};
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::client::WebPkiServerVerifier;
use tokio_rustls::rustls::crypto::ring;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio_rustls::rustls::{RootCertStore, ServerConfig};

use super::*;
use crate::code::tests::encode;
use crate::inventory::{Inventory, Item, Os};

/// A request as the stand-in saw it (body decompressed).
#[derive(Debug, Clone)]
struct Seen {
    method: String,
    path: String,
    host: String,
    auth: Option<String>,
    content_type: String,
    gzip: bool,
    body: Vec<u8>,
}

/// The stand-in's memory.
#[derive(Default)]
struct FakeState {
    /// One-time codes that are valid, without dash.
    codes: Vec<String>,
    token: Option<String>,
    revoked: bool,
    stored_hash: Option<String>,
    /// Answer the next request with this instead (status, Retry-After, body).
    force: Option<(u16, Option<u64>, String)>,
    seen: Vec<Seen>,
}

struct Fake {
    port: u16,
    /// SHA-256 of the server certificate.
    fingerprint: [u8; 32],
    /// The test CA (empty when the certificate is self-signed).
    ca: Option<CertificateDer<'static>>,
    state: Arc<Mutex<FakeState>>,
    _task: tokio::task::JoinHandle<()>,
}

impl Fake {
    /// opensecmon with the self-signed certificate of the built-in HTTPS.
    async fn self_signed() -> Self {
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = rcgen::CertificateParams::new(vec!["opensecmon".to_owned()])
            .unwrap()
            .self_signed(&key)
            .unwrap();
        Self::start(cert.der().clone(), key.serialize_der(), None).await
    }

    /// opensecmon behind a proxy with a certificate for `localhost` from a CA the "system" trusts.
    async fn with_ca() -> Self {
        let ca_key = rcgen::KeyPair::generate().unwrap();
        let mut ca_params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let ca = rcgen::CertifiedIssuer::self_signed(ca_params, ca_key).unwrap();
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = rcgen::CertificateParams::new(vec!["localhost".to_owned()])
            .unwrap()
            .signed_by(&key, &ca)
            .unwrap();
        Self::start(
            cert.der().clone(),
            key.serialize_der(),
            Some(ca.der().clone()),
        )
        .await
    }

    async fn start(
        cert: CertificateDer<'static>,
        key: Vec<u8>,
        ca: Option<CertificateDer<'static>>,
    ) -> Self {
        let mut fingerprint = [0u8; 32];
        fingerprint.copy_from_slice(&Sha256::digest(cert.as_ref()));
        let config = ServerConfig::builder_with_provider(Arc::new(ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![cert],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key)),
            )
            .unwrap();
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let state = Arc::new(Mutex::new(FakeState {
            codes: vec!["K7FQ29XM".to_owned()],
            ..FakeState::default()
        }));
        let shared = state.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((tcp, _)) = listener.accept().await else {
                    return;
                };
                let acceptor = acceptor.clone();
                let state = shared.clone();
                tokio::spawn(async move {
                    // A refused handshake is the client's business.
                    let Ok(tls) = acceptor.accept(tcp).await else {
                        return;
                    };
                    let service = hyper::service::service_fn(move |req| {
                        let state = state.clone();
                        async move { Ok::<_, std::convert::Infallible>(handle(req, port, &state).await) }
                    });
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(tls), service)
                        .await;
                });
            }
        });
        Self {
            port,
            fingerprint,
            ca,
            state,
            _task: task,
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.state.lock().unwrap()
    }

    fn seen(&self) -> Vec<Seen> {
        self.state().seen.clone()
    }

    /// An agent in a fresh folder whose "system" trusts the test CA (or nothing at all).
    fn agent(&self) -> (Agent, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let ca = self.ca.clone();
        let connect: Connect = Box::new(move |address, pin| {
            let mut roots = RootCertStore::empty();
            if let Some(ca) = &ca {
                roots.add(ca.clone()).unwrap();
            } else {
                // A CA that signed nothing here: the "system" trusts no certificate of the tests.
                let key = rcgen::KeyPair::generate().unwrap();
                let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
                params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
                roots
                    .add(params.self_signed(&key).unwrap().der().clone())
                    .unwrap();
            }
            let system = WebPkiServerVerifier::builder_with_provider(
                Arc::new(roots),
                Arc::new(ring::default_provider()),
            )
            .build()
            .unwrap();
            Client::with_system_verifier(address, pin, system)
        });
        (
            Agent::with_connect(Store::new(dir.path().join("agent")), connect),
            dir,
        )
    }

    fn code(&self, address: &str, fingerprint: Option<[u8; 32]>) -> String {
        encode(address, fingerprint.as_ref(), "K7FQ-29XM")
    }

    fn pinned_code(&self) -> String {
        self.code(&format!("127.0.0.1:{}", self.port), Some(self.fingerprint))
    }
}

/// What opensecmon does with the requests of the agent API (only what the agent needs).
async fn handle(
    req: hyper::Request<hyper::body::Incoming>,
    port: u16,
    state: &Mutex<FakeState>,
) -> hyper::Response<Full<Bytes>> {
    let header = |name: &str| {
        req.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };
    let mut seen = Seen {
        method: req.method().to_string(),
        path: req.uri().path().to_owned(),
        host: header("host").unwrap_or_default(),
        auth: header("authorization"),
        content_type: header("content-type").unwrap_or_default(),
        gzip: header("content-encoding").as_deref() == Some("gzip"),
        body: Vec::new(),
    };
    let raw = req.into_body().collect().await.unwrap().to_bytes();
    seen.body = if seen.gzip {
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(&raw[..])
            .read_to_end(&mut out)
            .unwrap();
        out
    } else {
        raw.to_vec()
    };
    let mut s = state.lock().unwrap();
    s.seen.push(seen.clone());
    let reply = |status: u16, body: serde_json::Value| (status, None, body.to_string());
    let error = |status: u16, code: &str| {
        reply(
            status,
            serde_json::json!({ "error": code, "message": format!("{code} (test)") }),
        )
    };
    let authorized = |s: &FakeState| {
        !s.revoked
            && s.token.is_some()
            && seen.auth == s.token.as_ref().map(|t| format!("Bearer {t}"))
    };
    let (status, retry_after, body) = if let Some(forced) = s.force.take() {
        forced
    } else if seen.host != format!("127.0.0.1:{port}") && seen.host != format!("localhost:{port}") {
        error(421, "misdirected")
    } else {
        match (seen.method.as_str(), seen.path.as_str()) {
            ("POST", "/api/v1/agent/enroll") => {
                let body: serde_json::Value = serde_json::from_slice(&seen.body).unwrap();
                let code = body["code"].as_str().unwrap_or_default().replace('-', "");
                if let Some(i) = s.codes.iter().position(|c| *c == code) {
                    s.codes.remove(i);
                    let token = format!("osm1_{}", "A".repeat(43));
                    s.token = Some(token.clone());
                    s.revoked = false;
                    let interval = if body["agent"]["name"] == "opensecagent" {
                        900
                    } else {
                        21600
                    };
                    reply(
                        201,
                        serde_json::json!({ "device": "laptop", "token": token, "report_interval_s": interval }),
                    )
                } else {
                    error(401, "invalid_code")
                }
            }
            ("POST", "/api/v1/agent/report") if !authorized(&s) => error(401, "invalid_token"),
            ("POST", "/api/v1/agent/report") => {
                let body: serde_json::Value = serde_json::from_slice(&seen.body).unwrap();
                if let Some(hash) = body["unchanged"].as_str() {
                    if s.stored_hash.as_deref() == Some(hash) {
                        reply(
                            200,
                            serde_json::json!({ "status": "unchanged", "next_report_s": 900 }),
                        )
                    } else {
                        error(409, "full_report_required")
                    }
                } else {
                    let hash: String = Sha256::digest(&seen.body)
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect();
                    let status = if s.stored_hash.as_deref() == Some(hash.as_str()) {
                        "unchanged"
                    } else {
                        "stored"
                    };
                    s.stored_hash = Some(hash);
                    reply(
                        200,
                        serde_json::json!({ "status": status, "next_report_s": 900,
                            "warnings": [{ "code": "lines_skipped", "count": 1 }] }),
                    )
                }
            }
            ("GET", "/api/v1/agent/status") if !authorized(&s) => error(401, "invalid_token"),
            ("GET", "/api/v1/agent/status") => reply(
                200,
                serde_json::json!({ "device": "laptop", "last_report": null, "next_report_s": 900 }),
            ),
            _ => error(404, "not_found"),
        }
    };
    let mut response = hyper::Response::builder()
        .status(status)
        .header("content-type", "application/json");
    if let Some(s) = retry_after {
        response = response.header("retry-after", s.to_string());
    }
    response.body(Full::new(Bytes::from(body))).unwrap()
}

fn inventory() -> Inventory {
    Inventory {
        hostname: "laptop".into(),
        os: Os {
            family: "linux",
            os_release: Some("NAME=\"Debian GNU/Linux\"\nID=debian\nVERSION_ID=\"13\"\n".into()),
            kernel: Some("6.12.48+deb13-amd64".into()),
            reboot_required: Some(false),
            ubuntu_pro: Some(false),
            arch: "x86_64".into(),
            ..Os::default()
        },
        dpkg: Some(
            "ii \tbash\t5.2.37-2\tbash\t5.2.37-2\nii \topenssl\t3.5.4-1\topenssl\t3.5.4-1\n".into(),
        ),
        items: vec![Item {
            kind: "flatpak",
            id: "org.mozilla.firefox".into(),
            name: "Firefox".into(),
            version: "144.0.2".into(),
            publisher: None,
            winget_id: None,
            scope: Some("machine"),
        }],
    }
}

fn the_error(r: Result<impl std::fmt::Debug, Error>) -> Error {
    r.expect_err("an error")
}

#[tokio::test]
async fn enroll_with_the_pinned_certificate_and_report() {
    let fake = Fake::self_signed().await;
    let (agent, _dir) = fake.agent();

    let enrolled = agent.enroll(&fake.pinned_code(), "laptop").await.unwrap();
    assert_eq!(enrolled.device, "laptop");
    let seen = fake.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0].host,
        format!("127.0.0.1:{}", fake.port),
        "as in allowed_hosts"
    );
    assert_eq!(seen[0].auth, None);
    let body: serde_json::Value = serde_json::from_slice(&seen[0].body).unwrap();
    assert_eq!(body["code"], "K7FQ-29XM");
    assert_eq!(body["hostname"], "laptop");
    assert_eq!(body["agent"]["name"], "opensecagent");
    assert_eq!(agent.store().state().next_report_s, Some(900));
    let enrollment = agent.store().enrollment().unwrap().unwrap();
    assert_eq!(enrollment.pin(), Some(fake.fingerprint));
    assert_eq!(
        agent.store().token().unwrap().unwrap().expose(),
        fake.state().token.as_deref().unwrap()
    );

    // First report: the full list, compressed.
    let reported = agent.report(&inventory(), false).await.unwrap();
    assert_eq!(
        (reported.status.as_str(), reported.short),
        ("stored", false)
    );
    assert_eq!(reported.next_report_s, 900);
    assert_eq!(reported.warnings[0].code, "lines_skipped");
    let full = fake.seen().pop().unwrap();
    assert!(full.gzip);
    assert_eq!(full.content_type, "application/json");
    assert_eq!(
        full.auth,
        fake.state().token.as_ref().map(|t| format!("Bearer {t}"))
    );
    let body: serde_json::Value = serde_json::from_slice(&full.body).unwrap();
    assert_eq!(body["schema"], 1);
    assert_eq!(body["os"]["family"], "linux");
    assert_eq!(body["os"]["kernel"], "6.12.48+deb13-amd64");
    assert!(body["dpkg"].as_str().unwrap().contains("\topenssl\t"));
    assert_eq!(body["items"][0]["type"], "flatpak");
    assert_eq!(reported.size, full.body.len());

    // Unchanged: only the hash.
    let reported = agent.report(&inventory(), false).await.unwrap();
    assert_eq!(
        (reported.status.as_str(), reported.short),
        ("unchanged", true)
    );
    let short = fake.seen().pop().unwrap();
    assert!(!short.gzip);
    let body: serde_json::Value = serde_json::from_slice(&short.body).unwrap();
    assert_eq!(
        body["unchanged"].as_str(),
        fake.state().stored_hash.as_deref()
    );

    // Asked for the full list: sent although unchanged.
    let reported = agent.report(&inventory(), true).await.unwrap();
    assert_eq!(
        (reported.status.as_str(), reported.short),
        ("unchanged", false)
    );

    // Changed: the full list again.
    let mut changed = inventory();
    changed.dpkg = Some("ii \tbash\t5.2.37-3\tbash\t5.2.37-3\n".into());
    let reported = agent.report(&changed, false).await.unwrap();
    assert_eq!(
        (reported.status.as_str(), reported.short),
        ("stored", false)
    );
    assert_eq!(fake.seen().len(), 5);

    let state = agent.store().state();
    assert_eq!(
        state.last_hash.as_deref(),
        fake.state().stored_hash.as_deref()
    );
    assert_eq!(state.next_report_s, Some(900));
    assert!(state.last_report.is_some());
    assert_eq!(state.last_error, None);

    let status = agent.status().await.unwrap();
    assert_eq!(status.remote.unwrap().device, "laptop");
    assert_eq!(status.enrollment.device, "laptop");
}

#[tokio::test]
async fn another_certificate_is_refused_before_anything_is_sent() {
    let fake = Fake::self_signed().await;
    let (agent, _dir) = fake.agent();
    let wrong = fake.code(&format!("127.0.0.1:{}", fake.port), Some([7; 32]));
    match the_error(agent.enroll(&wrong, "laptop").await) {
        Error::Api(ApiError::Http(HttpError::Untrusted { offered, reason })) => {
            assert_eq!(offered, code::format_fingerprint(&fake.fingerprint));
            assert!(
                reason.contains("not the certificate of the enrollment code"),
                "{reason}"
            );
        }
        e => panic!("{e:?}"),
    }
    assert!(fake.seen().is_empty(), "no request, no code sent");
    assert_eq!(agent.store().enrollment().unwrap(), None);
    // Without a fingerprint a self-signed certificate is never good enough.
    let without = fake.code(&format!("127.0.0.1:{}", fake.port), None);
    assert!(matches!(
        the_error(agent.enroll(&without, "laptop").await),
        Error::Api(ApiError::Http(HttpError::Untrusted { .. }))
    ));
    assert!(fake.seen().is_empty());
}

#[tokio::test]
async fn a_certificate_valid_for_the_address_is_enough() {
    let fake = Fake::with_ca().await;
    // No fingerprint (opensecmon behind a proxy), or one that does not match: the certificate is valid for the
    // name either way.
    for fp in [None, Some([7; 32])] {
        let (agent, _dir) = fake.agent();
        fake.state().codes.push("K7FQ29XM".into());
        let code = fake.code(&format!("localhost:{}", fake.port), fp);
        agent.enroll(&code, "laptop").await.unwrap();
        agent.report(&inventory(), false).await.unwrap();
    }
    // Not valid for the IP address, and no fingerprint that matches.
    let (agent, _dir) = fake.agent();
    let code = fake.code(&format!("127.0.0.1:{}", fake.port), None);
    assert!(matches!(
        the_error(agent.enroll(&code, "laptop").await),
        Error::Api(ApiError::Http(HttpError::Untrusted { .. }))
    ));
}

#[tokio::test]
async fn a_used_code_and_a_damaged_code_say_what_to_do() {
    let fake = Fake::self_signed().await;
    let (agent, _dir) = fake.agent();
    agent.enroll(&fake.pinned_code(), "laptop").await.unwrap();
    let (other, _dir2) = fake.agent();
    let e = the_error(other.enroll(&fake.pinned_code(), "laptop").await);
    assert!(matches!(e, Error::CodeInvalid), "{e:?}");
    assert!(e.to_string().contains("create a new code"));
    assert_eq!(other.store().enrollment().unwrap(), None);

    let code = fake.pinned_code();
    let damaged = format!("{}x", &code[..code.len() - 1]);
    assert!(matches!(
        the_error(other.enroll(&damaged, "laptop").await),
        Error::Code(CodeError::Damaged)
    ));
    assert!(matches!(
        the_error(other.enroll("K7FQ-29XM", "laptop").await),
        Error::Code(CodeError::Prefix)
    ));
}

#[tokio::test]
async fn an_unknown_hash_brings_the_full_list() {
    let fake = Fake::self_signed().await;
    let (agent, _dir) = fake.agent();
    agent.enroll(&fake.pinned_code(), "").await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&fake.seen()[0].body).unwrap();
    assert!(body.get("hostname").is_none(), "no empty host name");
    agent.report(&inventory(), false).await.unwrap();
    // opensecmon forgot the last report (e.g. a restored backup).
    fake.state().stored_hash = None;
    let reported = agent.report(&inventory(), false).await.unwrap();
    assert_eq!(
        (reported.status.as_str(), reported.short),
        ("stored", false)
    );
    let seen = fake.seen();
    assert_eq!(seen.len(), 4);
    assert!(!seen[2].gzip, "the short form first");
    assert!(seen[3].gzip, "then the full list");
}

#[tokio::test]
async fn a_revoked_computer_stops_reporting() {
    let fake = Fake::self_signed().await;
    let (agent, _dir) = fake.agent();
    agent.enroll(&fake.pinned_code(), "laptop").await.unwrap();
    fake.state().revoked = true;
    let e = the_error(agent.report(&inventory(), false).await);
    assert!(matches!(e, Error::Revoked), "{e:?}");
    let state = agent.store().state();
    assert!(state.revoked);
    assert!(state.last_error.unwrap().contains("enroll it again"));
    let before = fake.seen().len();
    assert!(matches!(
        the_error(agent.report(&inventory(), false).await),
        Error::Revoked
    ));
    assert_eq!(fake.seen().len(), before, "nothing sent any more");
    assert!(matches!(
        agent.status().await.unwrap().remote,
        Err(Error::Revoked)
    ));

    // A new code for the same computer starts over.
    fake.state().codes.push("K7FQ29XM".into());
    agent.enroll(&fake.pinned_code(), "laptop").await.unwrap();
    assert!(!agent.store().state().revoked);
    agent.report(&inventory(), false).await.unwrap();
}

#[tokio::test]
async fn opensecmon_asking_to_wait_is_respected() {
    let fake = Fake::self_signed().await;
    let (agent, _dir) = fake.agent();
    agent.enroll(&fake.pinned_code(), "laptop").await.unwrap();
    let too_many = serde_json::json!({ "error": "too_many_requests", "message": "slow down" });
    fake.state().force = Some((429, Some(30), too_many.to_string()));
    let e = the_error(agent.report(&inventory(), false).await);
    assert!(matches!(e, Error::Wait(30)), "{e:?}");
    assert_eq!(agent.store().state().last_hash, None);
    assert!(agent.store().state().last_error.is_some());
    fake.state().force = Some((503, None, r#"{"error":"busy","message":""}"#.into()));
    assert!(matches!(
        the_error(agent.report(&inventory(), false).await),
        Error::Wait(DEFAULT_WAIT_S)
    ));
    let refused = r#"{"error":"network_not_allowed","message":"no"}"#;
    fake.state().force = Some((403, None, refused.into()));
    assert!(matches!(
        the_error(agent.report(&inventory(), false).await),
        Error::NetworkNotAllowed
    ));
    fake.state().force = Some((
        422,
        None,
        r#"{"error":"unsupported_system","message":"Fedora"}"#.into(),
    ));
    assert!(matches!(
        the_error(agent.report(&inventory(), false).await),
        Error::UnsupportedSystem(m) if m == "Fedora"
    ));
    // An answer that is no error document.
    fake.state().force = Some((500, None, "<html>\u{1b}[31m oops".into()));
    match the_error(agent.report(&inventory(), false).await) {
        Error::Api(ApiError::Refused {
            status,
            code,
            message,
            ..
        }) => {
            assert_eq!((status, code.as_str()), (500, ""));
            assert!(!message.contains('\u{1b}'));
        }
        e => panic!("{e:?}"),
    }
    // A success that is no answer of the API.
    fake.state().force = Some((200, None, "not json".into()));
    assert!(matches!(
        the_error(agent.report(&inventory(), false).await),
        Error::Api(ApiError::Unexpected(_))
    ));
    // And then it works again, and the error is gone.
    agent.report(&inventory(), false).await.unwrap();
    assert_eq!(agent.store().state().last_error, None);
}

#[tokio::test]
async fn nothing_works_without_an_enrollment() {
    let fake = Fake::self_signed().await;
    let (agent, _dir) = fake.agent();
    assert!(matches!(
        the_error(agent.report(&inventory(), false).await),
        Error::NotEnrolled
    ));
    assert!(matches!(
        the_error(agent.status().await),
        Error::NotEnrolled
    ));
    assert!(!agent.forget().unwrap());
    agent.enroll(&fake.pinned_code(), "laptop").await.unwrap();
    assert!(agent.forget().unwrap());
    assert!(matches!(
        the_error(agent.status().await),
        Error::NotEnrolled
    ));
}

#[tokio::test]
async fn an_unreachable_opensecmon_is_a_plain_error() {
    // A port nobody listens on.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    drop(listener);
    let fake = Fake::self_signed().await;
    let (agent, _dir) = fake.agent();
    let code = fake.code(&addr.to_string(), Some(fake.fingerprint));
    assert!(matches!(
        the_error(agent.enroll(&code, "laptop").await),
        Error::Api(ApiError::Http(HttpError::Connect(_)))
    ));
}

#[test]
fn the_report_body_is_stable() {
    let a = report_body(&inventory(), AgentInfo::this());
    let b = report_body(&inventory(), AgentInfo::this());
    assert_eq!(a, b, "the same inventory gives the same hash");
    let json: serde_json::Value = serde_json::from_slice(&a).unwrap();
    assert_eq!(json["agent"]["name"], "opensecagent");
    assert!(json["agent"].get("updates").is_none());
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(&gzip(&a)[..])
        .read_to_end(&mut out)
        .unwrap();
    assert_eq!(out, a);
    assert_eq!(sha256_hex(b"").len(), 64);
}

#[tokio::test]
async fn an_error_before_sending_shows_until_the_next_report() {
    let fake = Fake::self_signed().await;
    let (agent, _dir) = fake.agent();
    let e = Error::Inventory(crate::inventory::InventoryError::Read(
        "the Flatpaks of the users",
        "Permission denied".into(),
    ));
    // Not enrolled: nothing is written.
    agent.note_error(&e);
    assert!(!agent.store().dir().join("state.json").exists());

    agent.enroll(&fake.pinned_code(), "laptop").await.unwrap();
    agent.note_error(&e);
    let state = agent.store().state();
    assert!(
        state
            .last_error
            .as_deref()
            .unwrap()
            .contains("Permission denied"),
        "{state:?}"
    );
    assert_eq!(
        state.next_report_s,
        Some(900),
        "the rest of the state stays"
    );
    assert!(!state.revoked);
    let status = agent.status().await.unwrap();
    assert!(status.remote.is_ok());
    assert!(status.state.last_error.is_some());

    agent.report(&inventory(), false).await.unwrap();
    assert_eq!(agent.store().state().last_error, None);
}
