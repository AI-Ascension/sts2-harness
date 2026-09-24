// SPDX-License-Identifier: MIT

//! Bounded provider port and adversarial guard for authoring inference.
//!
//! The port is the only place a proposal source is contacted. It receives a
//! bounded, credential-free request (a requirement summary plus the owner's
//! admitted node kinds) and returns a candidate definition, unsatisfied
//! requirements, diagnostics and an honest cost. It cannot publish, start a run
//! or reach a game instance; those remain separate authorities.
//!
//! The guard runs before any candidate is returned to a caller. It is a
//! defense-in-depth refusal, not an authority: it rejects endpoint/command-like
//! strings, secret-like fields and authority-escalating operation tokens so a
//! model-authored candidate can never smuggle a transport, credential or
//! publish/run path into portable workflow content.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::contract::Diagnostic;
use super::contract_authoring_inference::AuthoringInferenceCost;
use super::service::ManagementError;

/// A bounded request handed to an authoring-inference provider.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AuthoringInferenceProviderRequest {
    pub requirement_summary: String,
    pub max_stages: u64,
    pub max_candidate_bytes: u64,
    /// Capabilities the owner advertises, derived from the served manifest
    /// rather than from the caller.
    pub admitted_capabilities: Vec<String>,
}

/// One candidate produced by a provider. Every field is untrusted data.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AuthoringInferenceCandidate {
    pub definition: Value,
    #[serde(default)]
    pub unsatisfied: Vec<String>,
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
    pub cost: AuthoringInferenceCost,
}

/// The single boundary through which a proposal source is contacted.
pub trait AuthoringInferencePort: Send + Sync {
    fn propose(
        &self,
        request: &AuthoringInferenceProviderRequest,
    ) -> Result<AuthoringInferenceCandidate, ManagementError>;
}

/// Refuses every proposal when no provider is attached.
pub struct UnavailableAuthoringInferencePort;

impl AuthoringInferencePort for UnavailableAuthoringInferencePort {
    fn propose(
        &self,
        _request: &AuthoringInferenceProviderRequest,
    ) -> Result<AuthoringInferenceCandidate, ManagementError> {
        Err(ManagementError::unavailable(
            "authoring_inference_unavailable",
            "no authoring-inference provider is attached to this workflow owner",
        ))
    }
}

const FORBIDDEN_KEY_TOKENS: &[&str] = &[
    "publish", "deploy", "launch", "install", "mutate", "shell", "endpoint", "url", "exec",
];
const FORBIDDEN_OPERATION_TOKENS: &[&str] = &[
    "publish", "run", "execute", "deploy", "launch", "install", "control", "admin", "shell",
    "command", "mutate", "save", "spawn",
];
const ENDPOINT_TOKENS: &[&str] = &[
    "http://",
    "https://",
    "file://",
    "ftp://",
    "ws://",
    "wss://",
    "\\\\",
    "cmd.exe",
    "powershell",
    "bash",
    "/bin/",
];
/// Exact key names whose string values are treated as operation references.
const OPERATION_KEYS: &[&str] = &["operation", "operations", "allowed_operations"];
/// Exact key names whose array values name capabilities.
const CAPABILITY_KEYS: &[&str] = &["required", "optional"];

/// Refuses a candidate that carries a transport, credential or authority path.
pub(super) fn guard_candidate(definition: &Value) -> Result<(), ManagementError> {
    walk(definition, "$", None)
}

fn walk(value: &Value, path: &str, key: Option<&str>) -> Result<(), ManagementError> {
    match value {
        Value::Array(values) => {
            let as_operation = key.is_some_and(|key| OPERATION_KEYS.contains(&key));
            let as_capability = key.is_some_and(|key| CAPABILITY_KEYS.contains(&key));
            for (index, child) in values.iter().enumerate() {
                if (as_operation || as_capability)
                    && let Some(text) = child.as_str()
                    && forbidden_operation(text).is_some()
                {
                    return Err(forbidden_authority(path, text));
                }
                walk(child, &format!("{path}[{index}]"), None)?;
            }
            Ok(())
        }
        Value::Object(object) => {
            for (name, child) in object {
                let lower = name.to_ascii_lowercase();
                if secret_like_key(&lower)
                    || FORBIDDEN_KEY_TOKENS.iter().any(|token| lower == *token)
                {
                    return Err(ManagementError::forbidden(
                        "authoring_inference_forbidden_authority",
                        format!("candidate field {path}.{name} is not admitted in a proposal"),
                    ));
                }
                if key.is_some_and(|key| OPERATION_KEYS.contains(&key))
                    && let Some(text) = child.as_str()
                    && let Some(token) = forbidden_operation(text)
                {
                    return Err(ManagementError::forbidden(
                        "authoring_inference_forbidden_operation",
                        format!("candidate operation {path}.{name} names forbidden token {token}"),
                    ));
                }
                walk(child, &format!("{path}.{name}"), Some(&lower))?;
            }
            Ok(())
        }
        Value::String(text) => {
            if let Some(token) = ENDPOINT_TOKENS.iter().find(|token| text.contains(**token)) {
                return Err(ManagementError::forbidden(
                    "authoring_inference_forbidden_endpoint",
                    format!("candidate string {path} carries forbidden token {token:?}"),
                ));
            }
            Ok(())
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => Ok(()),
    }
}

fn forbidden_operation(text: &str) -> Option<&'static str> {
    let lower = text.to_ascii_lowercase();
    FORBIDDEN_OPERATION_TOKENS
        .iter()
        .find(|token| {
            lower == **token
                || lower
                    .split(['.', '-', '_', ':'])
                    .any(|part| part == **token)
        })
        .copied()
}

/// Mirrors the Studio draft store's secret-like field policy: a field named
/// `token`/`*_token`/`*-token` or carrying a broad credential word is refused,
/// while an ordinary bound such as `max_output_tokens` is not.
fn secret_like_key(lower: &str) -> bool {
    const BROAD: &[&str] = &[
        "secret",
        "password",
        "apikey",
        "api_key",
        "private_key",
        "credential",
    ];
    BROAD
        .iter()
        .any(|needle| lower == *needle || lower.contains(needle))
        || lower == "token"
        || lower.ends_with("_token")
        || lower.ends_with("-token")
}

fn forbidden_authority(path: &str, text: &str) -> ManagementError {
    ManagementError::forbidden(
        "authoring_inference_forbidden_operation",
        format!("candidate operation {path} names forbidden value {text}"),
    )
}

/// The bounded capability list the owner advertises in its manifest.
pub(super) fn admitted_capabilities(capabilities: &Value) -> Vec<String> {
    capabilities
        .get("capabilities")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[path = "authoring_inference_journal.rs"]
mod journal;
pub use journal::{
    AuthoringInferenceBegin, AuthoringInferenceJournal, MemoryAuthoringInferenceJournal,
    UnavailableAuthoringInferenceJournal, authoring_inference_operation_id,
};
