// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticCode {
    InvalidJson,
    DuplicateKey,
    TrailingInput,
    UnsafeInteger,
    FloatNotAllowed,
    NonFinite,
    DepthExceeded,
    StringTooLong,
    CollectionTooLarge,
    UnknownField,
    UnknownEnum,
    InvalidIdentifier,
    InvalidVersion,
    InvalidDigest,
    MissingField,
    TypeMismatch,
    DuplicateIdentifier,
    MissingReference,
    TypeError,
    GraphCycle,
    PriorityTie,
    UnreachableTerminal,
    InvalidBinding,
    InvalidLimit,
    DigestMismatch,
    ImmutableConflict,
    UnsupportedNode,
    CapabilityUnavailable,
    StaleRevision,
    InvalidEvent,
    InvalidCommand,
    ReplayDivergence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructuralLocation {
    pub path: String,
    pub offset: Option<u64>,
}

impl StructuralLocation {
    pub fn root() -> Self {
        Self {
            path: "$".to_owned(),
            offset: None,
        }
    }

    pub fn field(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            offset: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Diagnostic {
    pub code: DiagnosticCode,
    pub severity: DiagnosticSeverity,
    pub location: StructuralLocation,
}

impl Diagnostic {
    pub const fn error(code: DiagnosticCode, location: StructuralLocation) -> Self {
        Self {
            code,
            severity: DiagnosticSeverity::Error,
            location,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticReport {
    diagnostics: Vec<Diagnostic>,
}

impl DiagnosticReport {
    pub fn new(diagnostics: Vec<Diagnostic>) -> Self {
        Self { diagnostics }
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn is_empty(&self) -> bool {
        self.diagnostics.is_empty()
    }
}
