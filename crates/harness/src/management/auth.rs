// SPDX-License-Identifier: MIT

use std::collections::BTreeSet;
use std::env;
use std::fmt;
use std::sync::Arc;

use super::contract::{ContractError, MAX_IDENTIFIER_BYTES, validate_identifier};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthContext {
    pub subject: String,
    scopes: BTreeSet<String>,
    run_prefix: Option<String>,
}

impl AuthContext {
    pub fn new(
        subject: impl Into<String>,
        scopes: impl IntoIterator<Item = String>,
    ) -> Result<Self, AuthError> {
        Self::with_run_prefix(subject, scopes, None)
    }

    pub fn with_run_prefix(
        subject: impl Into<String>,
        scopes: impl IntoIterator<Item = String>,
        run_prefix: Option<String>,
    ) -> Result<Self, AuthError> {
        let subject = subject.into();
        validate_identifier("subject", &subject).map_err(AuthError::invalid)?;
        let mut validated_scopes = BTreeSet::new();
        for scope in scopes {
            if scope.is_empty() || scope.len() > MAX_IDENTIFIER_BYTES {
                return Err(AuthError::invalid(ContractError::new(
                    "invalid_scope",
                    "authentication scope is outside the supported bound",
                )));
            }
            if !validated_scopes.insert(scope) {
                return Err(AuthError::invalid(ContractError::new(
                    "duplicate_scope",
                    "authentication scopes must be unique",
                )));
            }
        }
        let run_prefix = run_prefix
            .map(|prefix| {
                validate_identifier("run_prefix", &prefix).map_err(AuthError::invalid)?;
                Ok(prefix)
            })
            .transpose()?;
        Ok(Self {
            subject,
            scopes: validated_scopes,
            run_prefix,
        })
    }

    pub fn can(&self, required: &str) -> bool {
        self.scopes.contains("workflow:*")
            || self.scopes.contains(required)
            || (required == "workflow:read" && self.scopes.contains("workflow:control"))
    }

    pub fn can_run(&self, run_id: &str) -> bool {
        self.run_prefix
            .as_ref()
            .is_none_or(|prefix| run_id == prefix || run_id.starts_with(&format!("{prefix}:")))
    }

    pub fn scopes(&self) -> impl Iterator<Item = &str> {
        self.scopes.iter().map(String::as_str)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthError {
    Invalid(ContractError),
    Missing,
    InvalidCredentials,
    Configuration(String),
}

impl AuthError {
    fn invalid(error: ContractError) -> Self {
        Self::Invalid(error)
    }
}

impl fmt::Display for AuthError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(error) => write!(formatter, "{}", error),
            Self::Missing => formatter.write_str("authentication is required"),
            Self::InvalidCredentials => formatter.write_str("authentication failed"),
            Self::Configuration(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for AuthError {}

pub trait Authenticator: Send + Sync {
    fn authenticate(&self, bearer_token: Option<&str>) -> Result<AuthContext, AuthError>;
}

#[derive(Clone, Debug, Default)]
pub struct StaticAuthenticator {
    credentials: Arc<Vec<Credential>>,
}

#[derive(Clone, Debug)]
struct Credential {
    token: String,
    context: AuthContext,
}

impl StaticAuthenticator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_credential(
        mut self,
        token: impl Into<String>,
        context: AuthContext,
    ) -> Result<Self, AuthError> {
        let token = token.into();
        if token.is_empty() || token.len() > 4096 {
            return Err(AuthError::Configuration(
                "credential token is outside the supported bound".to_owned(),
            ));
        }
        let credentials = Arc::make_mut(&mut self.credentials);
        if credentials.iter().any(|entry| entry.token == token) {
            return Err(AuthError::Configuration(
                "duplicate credential token".to_owned(),
            ));
        }
        credentials.push(Credential { token, context });
        Ok(self)
    }

    pub fn single(token: impl Into<String>, context: AuthContext) -> Result<Self, AuthError> {
        Self::new().with_credential(token, context)
    }
}

impl Authenticator for StaticAuthenticator {
    fn authenticate(&self, bearer_token: Option<&str>) -> Result<AuthContext, AuthError> {
        let token = bearer_token.ok_or(AuthError::Missing)?;
        self.credentials
            .iter()
            .find(|credential| constant_time_equal(&credential.token, token))
            .map(|credential| credential.context.clone())
            .ok_or(AuthError::InvalidCredentials)
    }
}

#[derive(Clone, Debug)]
pub struct EnvironmentAuthenticator {
    profile: String,
    inner: StaticAuthenticator,
}

impl EnvironmentAuthenticator {
    pub fn from_profile(profile: &str) -> Result<Self, AuthError> {
        validate_identifier("auth_profile", profile).map_err(AuthError::invalid)?;
        let env_name = credential_environment_name(profile)?;
        let token = env::var(&env_name).map_err(|_| {
            AuthError::Configuration(format!(
                "credential environment variable {env_name} is not set"
            ))
        })?;
        let context = AuthContext::new(format!("profile:{profile}"), ["workflow:*".to_owned()])?;
        Ok(Self {
            profile: profile.to_owned(),
            inner: StaticAuthenticator::single(token, context)?,
        })
    }

    pub fn profile(&self) -> &str {
        &self.profile
    }
}

impl Authenticator for EnvironmentAuthenticator {
    fn authenticate(&self, bearer_token: Option<&str>) -> Result<AuthContext, AuthError> {
        self.inner.authenticate(bearer_token)
    }
}

fn credential_environment_name(profile: &str) -> Result<String, AuthError> {
    let mut name = String::from("STS2_WORKFLOW_TOKEN_");
    for character in profile.chars() {
        if character.is_ascii_alphanumeric() {
            name.push(character.to_ascii_uppercase());
        } else if character == '_' || character == '-' {
            name.push('_');
        } else {
            return Err(AuthError::Configuration(
                "auth profile contains an unsupported environment-name character".to_owned(),
            ));
        }
    }
    Ok(name)
}

fn constant_time_equal(left: &str, right: &str) -> bool {
    let left_bytes = left.as_bytes();
    let right_bytes = right.as_bytes();
    let mut difference = left_bytes.len() ^ right_bytes.len();
    let max_len = left_bytes.len().max(right_bytes.len());
    for index in 0..max_len {
        let left_byte = left_bytes.get(index).copied().unwrap_or(0);
        let right_byte = right_bytes.get(index).copied().unwrap_or(0);
        difference |= usize::from(left_byte ^ right_byte);
    }
    difference == 0
}
