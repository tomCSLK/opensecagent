//! The agent API of opensecmon, version 1 (`docs/agent-api.md` there): the bodies and the errors.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::http::{Body, Client, HttpError};
use crate::store::Token;

/// Version of the formats of the bodies.
pub const SCHEMA: u32 = 1;
/// How the agent names itself; opensecmon gives this name the 15-minute interval and the update offer.
pub const AGENT_NAME: &str = "opensecagent";
pub const AGENT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub const ENROLL: &str = "/api/v1/agent/enroll";
pub const REPORT: &str = "/api/v1/agent/report";
pub const STATUS: &str = "/api/v1/agent/status";

/// Who reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentInfo {
    pub name: &'static str,
    pub version: &'static str,
    /// The agent can install updates offered by opensecmon (only with the publisher's key built in; phase 4).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub updates: bool,
    /// The update offered last could not be installed (at most 200 characters).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update_error: Option<String>,
}

impl AgentInfo {
    pub fn this() -> Self {
        Self {
            name: AGENT_NAME,
            version: AGENT_VERSION,
            updates: false,
            update_error: None,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct EnrollRequest<'a> {
    pub schema: u32,
    pub code: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hostname: Option<&'a str>,
    pub agent: AgentInfo,
}

#[derive(Debug, Deserialize)]
pub struct EnrollResponse {
    pub device: String,
    pub token: String,
    pub report_interval_s: u64,
}

/// The short form of a report when nothing changed.
#[derive(Debug, Serialize)]
pub struct Unchanged<'a> {
    pub schema: u32,
    pub unchanged: &'a str,
}

#[derive(Debug, Deserialize)]
pub struct ReportResponse {
    /// `stored` or `unchanged`.
    pub status: String,
    pub next_report_s: u64,
    #[serde(default)]
    pub warnings: Vec<Warning>,
    /// An update the user asked for.
    #[allow(dead_code, reason = "installing updates comes with phase 4")]
    #[serde(default)]
    pub update: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Warning {
    pub code: String,
    #[serde(default)]
    pub count: u64,
}

#[derive(Debug, Deserialize)]
pub struct StatusResponse {
    pub device: String,
    pub last_report: Option<String>,
    pub next_report_s: u64,
    #[allow(dead_code, reason = "installing updates comes with phase 4")]
    #[serde(default)]
    pub update: Option<serde_json::Value>,
}

/// An error document of opensecmon.
#[derive(Debug, Deserialize)]
struct ErrorBody {
    error: String,
    #[serde(default)]
    message: String,
}

/// Why a call failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApiError {
    #[error(transparent)]
    Http(#[from] HttpError),
    /// opensecmon answered with an error document (or with a status and no document).
    #[error("opensecmon answered {status} {code}: {message}")]
    Refused {
        status: u16,
        code: String,
        message: String,
        retry_after: Option<u64>,
    },
    /// A success whose body is not what the API describes.
    #[error("the answer of opensecmon is not what this agent understands: {0}")]
    Unexpected(String),
}

impl ApiError {
    /// The `error` code of opensecmon, if it sent one.
    pub fn code(&self) -> Option<&str> {
        match self {
            Self::Refused { code, .. } => Some(code),
            _ => None,
        }
    }

    pub fn retry_after(&self) -> Option<u64> {
        match self {
            Self::Refused { retry_after, .. } => *retry_after,
            _ => None,
        }
    }
}

/// Sends a JSON body (or none) and reads the JSON answer.
pub async fn call<T: DeserializeOwned>(
    client: &Client,
    method: hyper::Method,
    path: &str,
    token: Option<&Token>,
    body: Option<Body>,
) -> Result<T, ApiError> {
    let response = client.send(method, path, token, body).await?;
    if (200..300).contains(&response.status) {
        return serde_json::from_slice(&response.body)
            .map_err(|e| ApiError::Unexpected(e.to_string()));
    }
    let (code, message) = match serde_json::from_slice::<ErrorBody>(&response.body) {
        Ok(e) => (e.error, e.message),
        Err(_) => (
            String::new(),
            String::from_utf8_lossy(&response.body)
                .chars()
                .filter(|c| !c.is_control())
                .take(200)
                .collect(),
        ),
    };
    Err(ApiError::Refused {
        status: response.status,
        code: clean(&code, 64),
        message: clean(&message, 500),
        retry_after: response.retry_after,
    })
}

/// A JSON body.
pub fn json<T: Serialize>(value: &T) -> Body {
    Body {
        content_type: "application/json",
        gzip: false,
        bytes: serde_json::to_vec(value).expect("plain data"),
    }
}

/// Text from opensecmon for the log and the terminal: no control characters, limited length.
fn clean(text: &str, max: usize) -> String {
    text.chars().filter(|c| !c.is_control()).take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_agent_names_itself_as_opensecmon_expects() {
        let body = serde_json::to_value(EnrollRequest {
            schema: SCHEMA,
            code: "K7FQ-29XM",
            hostname: Some("laptop"),
            agent: AgentInfo::this(),
        })
        .unwrap();
        assert_eq!(
            body,
            serde_json::json!({
                "schema": 1,
                "code": "K7FQ-29XM",
                "hostname": "laptop",
                "agent": { "name": "opensecagent", "version": AGENT_VERSION }
            })
        );
        let mut agent = AgentInfo::this();
        agent.updates = true;
        agent.update_error = Some("signature does not match".into());
        assert_eq!(
            serde_json::to_value(agent).unwrap(),
            serde_json::json!({ "name": "opensecagent", "version": AGENT_VERSION,
                "updates": true, "update_error": "signature does not match" })
        );
    }

    #[test]
    fn answers_with_unknown_fields_are_understood() {
        let r: ReportResponse = serde_json::from_str(
            r#"{"status":"stored","next_report_s":900,"warnings":[{"code":"items_skipped","count":3}],"new":1}"#,
        )
        .unwrap();
        assert_eq!(r.next_report_s, 900);
        assert_eq!(r.warnings[0].count, 3);
        let s: StatusResponse =
            serde_json::from_str(r#"{"device":"laptop","last_report":null,"next_report_s":900}"#)
                .unwrap();
        assert_eq!(s.last_report, None);
    }

    #[test]
    fn text_of_opensecmon_is_cleaned_for_the_terminal() {
        assert_eq!(clean("a\u{1b}[31mb\nc", 100), "a[31mbc");
        assert_eq!(clean(&"x".repeat(1000), 500).len(), 500);
    }
}
