// SPDX-License-Identifier: MIT

use super::RuntimePolicyClock;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use sts2_harness::context_memory::{
    MemoryScope,
    policy_owner::{PolicyClock, PolicyGrant, PolicyPermission},
};

const MAX_CONFIG_BYTES: usize = 64 * 1024;

pub(super) const CONFIG_SCHEMA: &str = "ascension.runtime-v3.memory-policy-owner-config.v2";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DeploymentConfig {
    pub(super) schema: String,
    pub(super) scope: MemoryScope,
    pub(super) corpus_store_path: PathBuf,
    pub(super) policy_store_path: PathBuf,
    pub(super) lookup_archive_store_path: PathBuf,
    pub(super) replay_archive: bool,
    pub(super) archive_retention_seconds: u64,
    pub(super) policy_store_consent_ref: String,
    pub(super) auth_profile: String,
    pub(super) management_listen: SocketAddr,
    pub(super) preflight_timeout_seconds: u64,
    pub(super) selector_grant_id: String,
    pub(super) operator_grant_id: String,
    pub(super) phase2_revision_id: String,
    pub(super) control_epoch: u64,
    pub(super) plan_epoch: u64,
    pub(super) owner_epoch: u64,
    pub(super) grants: Vec<DeploymentGrant>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DeploymentGrant {
    pub(super) grant_id: String,
    pub(super) subject: String,
    pub(super) permissions: BTreeSet<PolicyPermission>,
    pub(super) epoch: u64,
    pub(super) expires_at: u64,
    pub(super) revoked: bool,
}

pub(super) fn deployment_grants(
    config: &DeploymentConfig,
    authenticated_subject: &str,
) -> Result<BTreeMap<String, PolicyGrant>, String> {
    if config.grants.is_empty() || config.grants.len() > 64 {
        return Err(String::from(
            "lookup owner configuration must contain 1 to 64 trusted grants",
        ));
    }
    let mut grants = BTreeMap::new();
    for grant in &config.grants {
        if !valid_id(&grant.grant_id)
            || !valid_id(&grant.subject)
            || grant.epoch == 0
            || grant.epoch > 9_007_199_254_740_991
            || grant.expires_at <= RuntimePolicyClock.now_seconds()
            || grant.permissions.is_empty()
        {
            return Err(String::from(
                "lookup owner configuration contains an invalid or expired grant",
            ));
        }
        if grants
            .insert(
                grant.grant_id.clone(),
                PolicyGrant {
                    grant_id: grant.grant_id.clone(),
                    subject: grant.subject.clone(),
                    scope: config.scope.clone(),
                    permissions: grant.permissions.clone(),
                    epoch: grant.epoch,
                    expires_at: grant.expires_at,
                    revoked: grant.revoked,
                },
            )
            .is_some()
        {
            return Err(String::from(
                "lookup owner configuration contains duplicate grant ids",
            ));
        }
    }
    let selector = grants
        .get(&config.selector_grant_id)
        .ok_or("selector grant is missing")?;
    let operator = grants
        .get(&config.operator_grant_id)
        .ok_or("operator grant is missing")?;
    if selector.subject != authenticated_subject
        || selector.revoked
        || operator.subject != authenticated_subject
        || operator.revoked
        || !selector.permissions.contains(&PolicyPermission::Select)
        || ![
            PolicyPermission::ReadMetadata,
            PolicyPermission::ReadContent,
            PolicyPermission::Write,
            PolicyPermission::Approve,
            PolicyPermission::Adopt,
            PolicyPermission::Select,
        ]
        .iter()
        .all(|permission| operator.permissions.contains(permission))
    {
        return Err(String::from(
            "lookup owner principal lacks the configured scoped grants",
        ));
    }
    Ok(grants)
}

pub(super) fn read_config(
    path: &Path,
    expected_sha256: &str,
) -> Result<(DeploymentConfig, String), String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|_| String::from("lookup owner configuration is unavailable"))?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() as usize > MAX_CONFIG_BYTES
    {
        return Err(String::from(
            "lookup owner configuration must be a bounded regular file",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(String::from(
                "lookup owner configuration must not be accessible by group or other users",
            ));
        }
    }
    let bytes = std::fs::read(path)
        .map_err(|_| String::from("lookup owner configuration cannot be read"))?;
    if bytes.len() > MAX_CONFIG_BYTES {
        return Err(String::from(
            "lookup owner configuration exceeds its byte bound",
        ));
    }
    let digest = sts2_harness::sha256_hex(&bytes);
    if digest != expected_sha256 {
        return Err(String::from(
            "lookup owner configuration SHA256 does not match its pin",
        ));
    }
    let config: DeploymentConfig = serde_json::from_slice(&bytes)
        .map_err(|_| String::from("lookup owner configuration schema is invalid"))?;
    Ok((config, digest))
}

pub(super) fn read_key_environment(name: &str) -> Result<[u8; 32], String> {
    let value = required_environment(name)?;
    if value.len() != 64 {
        return Err(format!(
            "{name} must contain 64 lowercase hexadecimal characters"
        ));
    }
    let mut key = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let text = std::str::from_utf8(pair).map_err(|_| format!("{name} is invalid"))?;
        key[index] = u8::from_str_radix(text, 16).map_err(|_| format!("{name} is invalid"))?;
    }
    if key.iter().all(|byte| *byte == 0) {
        return Err(format!("{name} must not be all zeroes"));
    }
    Ok(key)
}

pub(super) fn workflow_token_environment_name(profile: &str) -> Result<String, String> {
    if !valid_id(profile)
        || !profile
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
    {
        return Err(String::from("lookup owner auth profile is invalid"));
    }
    Ok(format!(
        "STS2_WORKFLOW_TOKEN_{}",
        profile
            .bytes()
            .map(|byte| if byte == b'-' {
                b'_'
            } else {
                byte.to_ascii_uppercase()
            } as char)
            .collect::<String>()
    ))
}

pub(super) fn required_environment(name: &str) -> Result<String, String> {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => Ok(value),
        Ok(_) => Err(format!("{name} must not be empty")),
        Err(std::env::VarError::NotPresent) => Err(format!("{name} is required")),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} is not valid UTF-8")),
    }
}

pub(super) fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && b"._:-".contains(&byte))
        })
}

pub(super) fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(super) fn utc_timestamp(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let day_seconds = seconds % 86_400;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    let hour = day_seconds / 3_600;
    let minute = (day_seconds % 3_600) / 60;
    let second = day_seconds % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}
