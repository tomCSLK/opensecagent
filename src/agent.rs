//! What the agent does: enroll with an enrollment code, report the inventory, ask for its status.

use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use flate2::Compression;
use flate2::write::GzEncoder;
use hyper::Method;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::api::{self, AgentInfo, ApiError, ReportResponse, StatusResponse, Warning};
use crate::code::{self, CodeError};
use crate::http::{Body, Client, HttpError};
use crate::inventory::{Inventory, InventoryError, Item, Os};
use crate::store::{Enrollment, State, Store, Token};

/// Wait when opensecmon asks for one without saying how long.
const DEFAULT_WAIT_S: u64 = 60;

/// Why something did not work, in words for the user.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Code(#[from] CodeError),
    #[error("this computer is not enrolled: enroll it with a code from opensecmon")]
    NotEnrolled,
    #[error(
        "this computer was revoked or deleted in opensecmon: enroll it again with a new code from its page"
    )]
    Revoked,
    #[error(
        "the code has expired, was used already or was replaced by a newer one: create a new code in opensecmon"
    )]
    CodeInvalid,
    #[error(
        "opensecmon does not accept agents from this network (setting agent_networks of opensecmon)"
    )]
    NetworkNotAllowed,
    #[error("opensecmon is busy or asks to report less often: trying again in {0} s")]
    Wait(u64),
    #[error("opensecmon does not support this system: {0}")]
    UnsupportedSystem(String),
    #[error(transparent)]
    Api(ApiError),
    #[error("the files of the agent in {} could not be used: {}{}", .0.display(), .1, permission_hint(.1))]
    Files(PathBuf, std::io::Error),
    #[error(transparent)]
    Inventory(#[from] InventoryError),
    #[cfg_attr(
        not(target_os = "linux"),
        allow(dead_code, reason = "only Linux has the dialog")
    )]
    #[error("the enrollment dialog could not be shown: {0}")]
    Dialog(String),
    #[cfg_attr(
        not(windows),
        allow(dead_code, reason = "only Windows has the service manager")
    )]
    #[error("the Windows service: {0}")]
    Service(String),
}

fn permission_hint(e: &std::io::Error) -> &'static str {
    match e.kind() {
        std::io::ErrorKind::PermissionDenied if cfg!(windows) => " (run it as administrator)",
        std::io::ErrorKind::PermissionDenied => " (run it as root, e.g. with sudo)",
        _ => "",
    }
}

impl Error {
    /// A short name for the kind of error, for the enrollment dialog, which words it in the user's language.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Code(_) => "code_damaged",
            Self::NotEnrolled => "not_enrolled",
            Self::Revoked => "revoked",
            Self::CodeInvalid => "code_invalid",
            Self::NetworkNotAllowed => "network_not_allowed",
            Self::Wait(_) => "wait",
            Self::UnsupportedSystem(_) => "unsupported_system",
            Self::Api(ApiError::Http(HttpError::Untrusted { .. })) => "untrusted",
            Self::Api(ApiError::Http(
                HttpError::Resolve(..) | HttpError::Connect(_) | HttpError::Timeout,
            )) => "unreachable",
            Self::Api(_) => "other",
            Self::Files(..) => "files",
            Self::Inventory(_) => "inventory",
            Self::Dialog(_) | Self::Service(_) => "other",
        }
    }
}

impl From<ApiError> for Error {
    fn from(e: ApiError) -> Self {
        match (e.code(), &e) {
            (Some("invalid_code"), _) => Self::CodeInvalid,
            (Some("invalid_token"), _) => Self::Revoked,
            (Some("network_not_allowed"), _) => Self::NetworkNotAllowed,
            (Some("unsupported_system"), ApiError::Refused { message, .. }) => {
                Self::UnsupportedSystem(message.clone())
            }
            (
                _,
                ApiError::Refused {
                    status: 429 | 503, ..
                },
            ) => Self::Wait(e.retry_after().unwrap_or(DEFAULT_WAIT_S)),
            _ => Self::Api(e),
        }
    }
}

impl From<HttpError> for Error {
    fn from(e: HttpError) -> Self {
        Self::Api(ApiError::Http(e))
    }
}

/// Opens the connection to an opensecmon; replaced in the tests by one that trusts their own CA.
pub type Connect = Box<dyn Fn(&str, Option<[u8; 32]>) -> Result<Client, HttpError>>;

/// The result of an enrollment.
#[derive(Debug)]
pub struct Enrolled {
    pub device: String,
    pub address: String,
}

/// The result of a report.
#[derive(Debug)]
pub struct Reported {
    /// `stored` or `unchanged`.
    pub status: String,
    /// Only the hash was sent.
    pub short: bool,
    /// Size of the full report (uncompressed).
    pub size: usize,
    pub next_report_s: u64,
    pub warnings: Vec<Warning>,
}

/// What `status` found: the local files and, if it could be reached, what opensecmon says.
#[derive(Debug)]
pub struct Status {
    pub enrollment: Enrollment,
    pub state: State,
    pub remote: Result<StatusResponse, Error>,
}

pub struct Agent {
    store: Store,
    connect: Connect,
}

impl Agent {
    pub fn new(store: Store) -> Self {
        Self::with_connect(store, Box::new(Client::new))
    }

    pub fn with_connect(store: Store, connect: Connect) -> Self {
        Self { store, connect }
    }

    /// Whether the files of an enrollment are there (it may have been revoked since).
    pub fn is_enrolled(&self) -> bool {
        self.enrolled().is_ok()
    }

    /// The local state of the reports.
    pub fn state(&self) -> State {
        self.store.state()
    }

    #[cfg(test)]
    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Enrolls this computer with an enrollment code; replaces an earlier enrollment (a new code for the same
    /// computer keeps its assets in opensecmon).
    pub async fn enroll(&self, code_text: &str, hostname: &str) -> Result<Enrolled, Error> {
        let code = code::decode(code_text)?;
        let client = (self.connect)(&code.address, code.fingerprint)?;
        let request = api::EnrollRequest {
            schema: api::SCHEMA,
            code: &code.code,
            hostname: (!hostname.is_empty()).then_some(hostname),
            agent: AgentInfo::this(),
        };
        let response: api::EnrollResponse = api::call(
            &client,
            Method::POST,
            api::ENROLL,
            None,
            Some(api::json(&request)),
        )
        .await?;
        let token = Token::new(&response.token).ok_or_else(|| {
            Error::Api(ApiError::Unexpected("the token has an unknown form".into()))
        })?;
        let enrollment = Enrollment {
            address: code.address.clone(),
            fingerprint: code.fingerprint.as_ref().map(code::format_fingerprint),
            device: response
                .device
                .chars()
                .filter(|c| !c.is_control())
                .take(253)
                .collect(),
            enrolled_at: now(),
        };
        self.store
            .save_enrollment(&enrollment, &token)
            .map_err(|e| self.files(e))?;
        self.keep_state(&State {
            next_report_s: Some(response.report_interval_s),
            ..State::default()
        });
        Ok(Enrolled {
            device: enrollment.device,
            address: enrollment.address,
        })
    }

    /// Reports the inventory: the short form if it is unchanged since the last accepted report (unless `full`),
    /// otherwise the full list, gzip-compressed.
    pub async fn report(&self, inventory: &Inventory, full: bool) -> Result<Reported, Error> {
        let (enrollment, token) = self.enrolled()?;
        let mut state = self.store.state();
        if state.revoked {
            return Err(Error::Revoked);
        }
        let body = report_body(inventory, AgentInfo::this());
        let hash = sha256_hex(&body);
        let result = async {
            let client = (self.connect)(&enrollment.address, enrollment.pin())?;
            if !full && state.last_hash.as_deref() == Some(hash.as_str()) {
                let short = api::Unchanged {
                    schema: api::SCHEMA,
                    unchanged: &hash,
                };
                match api::call::<ReportResponse>(
                    &client,
                    Method::POST,
                    api::REPORT,
                    Some(&token),
                    Some(api::json(&short)),
                )
                .await
                {
                    Ok(r) => return Ok((r, true)),
                    Err(e) if e.code() == Some("full_report_required") => {}
                    Err(e) => return Err(Error::from(e)),
                }
            }
            let full = Body {
                content_type: "application/json",
                gzip: true,
                bytes: gzip(&body),
            };
            let r = api::call::<ReportResponse>(
                &client,
                Method::POST,
                api::REPORT,
                Some(&token),
                Some(full),
            )
            .await?;
            Ok((r, false))
        }
        .await;
        match result {
            Ok((response, short)) => {
                state.last_hash = Some(hash);
                state.last_report = Some(now());
                state.next_report_s = Some(response.next_report_s);
                state.last_error = None;
                self.keep_state(&state);
                Ok(Reported {
                    status: response.status,
                    short,
                    size: body.len(),
                    next_report_s: response.next_report_s,
                    warnings: response.warnings,
                })
            }
            Err(e) => {
                self.note_failure(&mut state, &e);
                Err(e)
            }
        }
    }

    /// The local enrollment and what opensecmon says about this computer.
    pub async fn status(&self) -> Result<Status, Error> {
        let (enrollment, token) = self.enrolled()?;
        let mut state = self.store.state();
        let remote = async {
            let client = (self.connect)(&enrollment.address, enrollment.pin())?;
            api::call::<StatusResponse>(&client, Method::GET, api::STATUS, Some(&token), None)
                .await
                .map_err(Error::from)
        }
        .await;
        if let Err(e) = &remote {
            self.note_failure(&mut state, e);
        }
        Ok(Status {
            enrollment,
            state,
            remote,
        })
    }

    /// Forgets the enrollment (the computer stays in opensecmon until it is deleted there).
    pub fn forget(&self) -> Result<bool, Error> {
        self.store.forget().map_err(|e| self.files(e))
    }

    fn enrolled(&self) -> Result<(Enrollment, Token), Error> {
        let enrollment = self.store.enrollment().map_err(|e| self.files(e))?;
        let token = self.store.token().map_err(|e| self.files(e))?;
        match (enrollment, token) {
            (Some(e), Some(t)) => Ok((e, t)),
            _ => Err(Error::NotEnrolled),
        }
    }

    /// Keeps an error that happened before anything was sent (the inventory could not be read) as the last error,
    /// so `status` shows it; the next accepted report clears it. Nothing is written for a computer not enrolled.
    pub fn note_error(&self, e: &Error) {
        if self.is_enrolled() {
            self.note_failure(&mut self.store.state(), e);
        }
    }

    fn note_failure(&self, state: &mut State, e: &Error) {
        if matches!(e, Error::Revoked) {
            state.revoked = true;
        }
        state.last_error = Some(e.to_string());
        self.keep_state(state);
    }

    /// The state is a convenience: if it cannot be written, the next report is simply a full one.
    fn keep_state(&self, state: &State) {
        if let Err(e) = self.store.save_state(state) {
            crate::log::note(format_args!("warning: {}", self.files(e)));
        }
    }

    fn files(&self, e: std::io::Error) -> Error {
        Error::Files(self.store.dir().to_path_buf(), e)
    }
}

/// The JSON report, exactly as sent (and hashed).
#[derive(Serialize)]
struct ReportBody<'a> {
    schema: u32,
    agent: AgentInfo,
    #[serde(skip_serializing_if = "str::is_empty")]
    hostname: &'a str,
    os: &'a Os,
    #[serde(skip_serializing_if = "Option::is_none")]
    dpkg: Option<&'a str>,
    items: &'a [Item],
}

pub fn report_body(inventory: &Inventory, agent: AgentInfo) -> Vec<u8> {
    serde_json::to_vec(&ReportBody {
        schema: api::SCHEMA,
        agent,
        hostname: &inventory.hostname,
        os: &inventory.os,
        dpkg: inventory.dpkg.as_deref(),
        items: &inventory.items,
    })
    .expect("plain data")
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(bytes).expect("writing to memory");
    encoder.finish().expect("writing to memory")
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests;
