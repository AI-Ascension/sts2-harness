// SPDX-License-Identifier: MIT

use serde::Deserialize;
use std::path::PathBuf;
use sts2_harness::ExoIdentity;
use sts2_harness::exo_lifecycle::EXO_LIFECYCLE_WIRE_V2;
use sts2_harness::provider_session::{NativeCapabilities, ProviderSessionPolicy};

const CONFIG_ENV: &str = "STS2_EXO_LIFECYCLE_CONFIG";
const SCHEMA: &str = "sts2.exo-lifecycle-runtime-v1";

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RuntimeLifecycleConfig {
    schema_version: String,
    pub(super) directory: PathBuf,
    pub(super) store_id: String,
    key_reference: String,
    owner_token_reference: String,
    pub(super) project_id: String,
    pub(super) agent_id: String,
    pub(super) policy_store_path: PathBuf,
    policy_key_reference: String,
    pub(super) legacy_path: Option<PathBuf>,
}

#[allow(dead_code)]
pub(super) struct RuntimeLifecycleSecrets {
    pub(super) journal_key: [u8; 32],
    pub(super) owner_token: String,
    pub(super) policy_key: [u8; 32],
}

impl RuntimeLifecycleConfig {
    pub(super) fn from_environment() -> Result<Option<(Self, RuntimeLifecycleSecrets)>, String> {
        let Some(value) = optional(CONFIG_ENV)? else {
            return Ok(None);
        };
        let config: Self =
            serde_json::from_str(&value).map_err(|_| format!("{CONFIG_ENV} is invalid JSON"))?;
        config.validate()?;
        let secrets = RuntimeLifecycleSecrets {
            journal_key: key(&config.key_reference)?,
            owner_token: secret(&config.owner_token_reference)?,
            policy_key: key(&config.policy_key_reference)?,
        };
        Ok(Some((config, secrets)))
    }

    fn validate(&self) -> Result<(), String> {
        if self.schema_version != SCHEMA
            || !absolute(&self.directory)
            || !absolute(&self.policy_store_path)
            || self
                .legacy_path
                .as_ref()
                .is_some_and(|path| !absolute(path))
            || !id(&self.store_id)
            || !id(&self.project_id)
            || !id(&self.agent_id)
            || !environment_name(&self.key_reference)
            || !environment_name(&self.owner_token_reference)
            || !environment_name(&self.policy_key_reference)
        {
            return Err(format!("{CONFIG_ENV} has an invalid lifecycle field"));
        }
        Ok(())
    }

    /// Opens no provider route. This derives the sole native-session capability descriptor from
    /// the inspected executor/configuration identity and the v2 wire the local effect parses.
    /// A saved policy is separately checked against this profile before an owner exists.
    pub(super) fn capabilities(
        &self,
        inspected: &ExoIdentity,
    ) -> Result<NativeCapabilities, String> {
        let executor = inspected
            .package_digest
            .clone()
            .ok_or_else(|| String::from("lifecycle executor was not inspected"))?;
        let configuration = inspected
            .config_digest
            .clone()
            .ok_or_else(|| String::from("lifecycle configuration was not inspected"))?;
        let profile = sts2_harness::sha256_hex(
            serde_json::to_vec(&serde_json::json!({
                "adapter": "sts2-exo-lifecycle-v2",
                "configuration_sha256": configuration,
                "executor_sha256": executor,
                "wire": EXO_LIFECYCLE_WIRE_V2,
            }))
            .map_err(|_| String::from("cannot encode lifecycle capability profile"))?,
        );
        NativeCapabilities::reviewed_exo_lifecycle(
            "sts2-exo-lifecycle-v2",
            profile,
            executor,
            sts2_harness::sha256_hex(EXO_LIFECYCLE_WIRE_V2),
        )
        .map_err(|_| String::from("lifecycle capability profile is invalid"))
    }

    pub(super) fn validate_policy(
        &self,
        policy: &ProviderSessionPolicy,
        capabilities: &NativeCapabilities,
    ) -> Result<(), String> {
        if policy.scope.project_id != self.project_id
            || policy.scope.agent_id != self.agent_id
            || policy
                .admit_for_profile(capabilities)
                .is_err()
        {
            return Err(String::from(
                "adopted provider-session policy is incompatible with lifecycle profile",
            ));
        }
        Ok(())
    }
}

fn optional(name: &str) -> Result<Option<String>, String> {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => Ok(Some(value)),
        Ok(_) => Err(format!("{name} must not be empty")),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} is not valid UTF-8")),
    }
}

fn secret(reference: &str) -> Result<String, String> {
    let value = std::env::var(reference).map_err(|_| format!("{reference} is required"))?;
    if value.is_empty() || value.len() > 512 || value.chars().any(char::is_control) {
        return Err(format!("{reference} is invalid"));
    }
    Ok(value)
}

fn key(reference: &str) -> Result<[u8; 32], String> {
    let value = secret(reference)?;
    if value.len() != 64 {
        return Err(format!("{reference} must be 32-byte lowercase hex"));
    }
    let mut output = [0_u8; 32];
    for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
        let text = std::str::from_utf8(chunk).map_err(|_| format!("{reference} is invalid"))?;
        output[index] =
            u8::from_str_radix(text, 16).map_err(|_| format!("{reference} is invalid"))?;
    }
    Ok(output)
}

fn absolute(path: &std::path::Path) -> bool {
    path.is_absolute()
        && path.components().all(|part| {
            matches!(
                part,
                std::path::Component::RootDir | std::path::Component::Normal(_)
            )
        })
}

fn id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
}

fn environment_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().enumerate().all(|(index, byte)| {
            if index == 0 {
                byte.is_ascii_alphabetic() || byte == b'_'
            } else {
                byte.is_ascii_alphanumeric() || byte == b'_'
            }
        })
}
