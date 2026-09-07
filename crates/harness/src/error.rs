// SPDX-License-Identifier: MIT

use std::fmt;

use crate::identity::IdentityError;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Component {
    Routing,
    Provider,
    Storage,
    Replay,
    Artifact,
}

impl fmt::Display for Component {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Routing => "routing",
            Self::Provider => "provider",
            Self::Storage => "storage",
            Self::Replay => "replay",
            Self::Artifact => "artifact",
        };
        formatter.write_str(name)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortError {
    code: &'static str,
    message: String,
    is_retryable: bool,
}

impl PortError {
    #[must_use]
    pub fn new(code: &'static str, message: impl Into<String>, is_retryable: bool) -> Self {
        Self {
            code,
            message: message.into(),
            is_retryable,
        }
    }

    #[must_use]
    pub fn code(&self) -> &'static str {
        self.code
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        self.is_retryable
    }
}

impl fmt::Display for PortError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for PortError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderError {
    code: &'static str,
    message: String,
    is_retryable: bool,
}

impl ProviderError {
    #[must_use]
    pub fn new(code: &'static str, message: impl Into<String>, is_retryable: bool) -> Self {
        Self {
            code,
            message: message.into(),
            is_retryable,
        }
    }

    #[must_use]
    pub fn code(&self) -> &'static str {
        self.code
    }

    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        self.is_retryable
    }
}

impl fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ProviderError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CloseFailure {
    pub component: Component,
    pub error: PortError,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CloseReport {
    pub unbound_episodes: usize,
    pub closed_components: Vec<Component>,
    pub failures: Vec<CloseFailure>,
}

impl CloseReport {
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.failures.is_empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HarnessError {
    Closed,
    Invalid(String),
    Identity(IdentityError),
    Routing(PortError),
    Provider(ProviderError),
    Storage(PortError),
    Replay(PortError),
    Artifact(PortError),
    Cleanup(CloseReport),
}

impl From<IdentityError> for HarnessError {
    fn from(error: IdentityError) -> Self {
        Self::Identity(error)
    }
}

impl fmt::Display for HarnessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => formatter.write_str("harness is closed"),
            Self::Invalid(message) => write!(formatter, "invalid harness request: {message}"),
            Self::Identity(error) => write!(formatter, "identity error: {error}"),
            Self::Routing(error) => write!(formatter, "routing error: {error}"),
            Self::Provider(error) => write!(formatter, "provider error: {error}"),
            Self::Storage(error) => write!(formatter, "storage error: {error}"),
            Self::Replay(error) => write!(formatter, "replay error: {error}"),
            Self::Artifact(error) => write!(formatter, "artifact error: {error}"),
            Self::Cleanup(report) => write!(
                formatter,
                "cleanup failed for {} component(s)",
                report.failures.len()
            ),
        }
    }
}

impl std::error::Error for HarnessError {}
