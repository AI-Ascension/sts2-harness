// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};

use super::{EXO_CONTRACT_VERSION, EXO_SOURCE_REVISION};

/// Independent identities recorded for an Exo executor deployment.
///
/// The source revision is not an executable identity. A process, extension, model binding, and
/// prompt/tool/configuration inputs must be recorded separately before a run can claim a
/// reproducible deployment.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExoIdentity {
    pub source_revision: String,
    pub package_digest: Option<String>,
    pub extension_digest: Option<String>,
    pub bridge_digest: Option<String>,
    pub model_binding: Option<String>,
    /// Provider selected by the upstream model binding (for example, `openai`).
    ///
    /// This is recorded separately from `model_binding` because upstream routing also depends on
    /// the provider/base URL. An executable identity must carry both values explicitly.
    pub provider: Option<String>,
    /// Non-secret provider base URL used by upstream routing.
    ///
    /// Query strings, credentials, and fragments are intentionally outside the bounded identity
    /// grammar. The value is required for executable preflight so an OpenRouter override cannot be
    /// hidden behind an otherwise Responses-capable model name.
    pub endpoint: Option<String>,
    pub prompt_digest: Option<String>,
    pub tool_digest: Option<String>,
    pub config_digest: Option<String>,
    pub contract_version: String,
    pub native_instance_id: Option<String>,
}

impl ExoIdentity {
    /// Returns the source-only identity used by the checked-in, unverified manifest.
    pub fn source_only(model_binding: Option<String>) -> Result<Self, ExoIdentityError> {
        let identity = Self {
            source_revision: EXO_SOURCE_REVISION.to_owned(),
            package_digest: None,
            extension_digest: None,
            bridge_digest: None,
            model_binding,
            provider: None,
            endpoint: None,
            prompt_digest: None,
            tool_digest: None,
            config_digest: None,
            contract_version: EXO_CONTRACT_VERSION.to_owned(),
            native_instance_id: None,
        };
        identity.validate()?;
        Ok(identity)
    }

    /// Returns whether all deployment axes needed for an executable preflight are present.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.package_digest.is_some()
            && self.extension_digest.is_some()
            && self.bridge_digest.is_some()
            && self.model_binding.is_some()
            && self.provider.is_some()
            && self.endpoint.is_some()
            && self.prompt_digest.is_some()
            && self.tool_digest.is_some()
            && self.config_digest.is_some()
            && self.native_instance_id.is_some()
    }

    pub(super) fn validate(&self) -> Result<(), ExoIdentityError> {
        if !valid_revision(&self.source_revision) {
            return Err(ExoIdentityError::InvalidSourceRevision);
        }
        if self.contract_version != EXO_CONTRACT_VERSION {
            return Err(ExoIdentityError::InvalidContractVersion);
        }
        for (value, kind) in [
            (&self.package_digest, DigestKind::Package),
            (&self.extension_digest, DigestKind::Extension),
            (&self.bridge_digest, DigestKind::Bridge),
            (&self.prompt_digest, DigestKind::Prompt),
            (&self.tool_digest, DigestKind::Tool),
            (&self.config_digest, DigestKind::Config),
        ] {
            if let Some(value) = value
                && !valid_digest(value)
            {
                return Err(ExoIdentityError::InvalidDigest(kind));
            }
        }
        if self
            .model_binding
            .as_deref()
            .is_some_and(|value| !valid_text(value))
        {
            return Err(ExoIdentityError::InvalidModelBinding);
        }
        if self
            .provider
            .as_deref()
            .is_some_and(|value| !valid_provider(value))
        {
            return Err(ExoIdentityError::InvalidProvider);
        }
        if self
            .endpoint
            .as_deref()
            .is_some_and(|value| !valid_endpoint(value))
        {
            return Err(ExoIdentityError::InvalidEndpoint);
        }
        if self
            .native_instance_id
            .as_deref()
            .is_some_and(|value| !valid_text(value))
        {
            return Err(ExoIdentityError::InvalidNativeInstance);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DigestKind {
    Package,
    Extension,
    Bridge,
    Prompt,
    Tool,
    Config,
}

/// Identity validation failures are intentionally typed so callers can fail closed without
/// printing a credential, prompt, path, or other deployment value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExoIdentityError {
    InvalidSourceRevision,
    InvalidDigest(DigestKind),
    InvalidModelBinding,
    InvalidProvider,
    InvalidEndpoint,
    InvalidContractVersion,
    InvalidNativeInstance,
}

impl std::fmt::Display for ExoIdentityError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidSourceRevision => "Exo source revision is not a lowercase commit hash",
            Self::InvalidDigest(_) => "Exo deployment digest is not a lowercase SHA-256",
            Self::InvalidModelBinding => "Exo model binding is empty or unsafe",
            Self::InvalidProvider => "Exo provider identity is empty or unsafe",
            Self::InvalidEndpoint => "Exo provider endpoint is empty or unsafe",
            Self::InvalidContractVersion => "Exo identity names an unsupported contract version",
            Self::InvalidNativeInstance => "Exo native instance identity is empty or unsafe",
        })
    }
}

impl std::error::Error for ExoIdentityError {}

pub(super) fn valid_revision(value: &str) -> bool {
    (value.len() == 40 || value.len() == 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && value.bytes().any(|byte| byte != b'0')
}

pub(super) fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && value.bytes().any(|byte| byte != b'0')
}

pub(super) fn valid_text(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && !value.chars().any(char::is_control)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
}

pub(super) fn valid_provider(value: &str) -> bool {
    valid_text(value) && value.len() <= 128
}

pub(super) fn valid_endpoint(value: &str) -> bool {
    valid_text(value)
        && value.starts_with("https://")
        && value.len() <= 512
        && value
            .strip_prefix("https://")
            .and_then(|rest| rest.split('/').next())
            .is_some_and(|host| !host.is_empty())
}
