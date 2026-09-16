// SPDX-License-Identifier: MIT

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use sts2_harness::context_memory::{
    DurableMemoryStore, MemoryScope, memory_policy_schema_sha256,
    policy_owner::{
        ActivePolicyBinding, LookupPolicyAuthorityGuard, LookupPolicySnapshot,
        MemoryPolicyAuthority, MemoryPolicyOwner, PolicyAccess, PolicyClock, PolicyGrant,
        PolicyOwnerError, PolicyPermission, PolicyStoreConsent, TrustedPolicyState,
    },
};
use sts2_harness::management::EnvironmentAuthenticator;
use zeroize::Zeroizing;

const CONFIG_SCHEMA: &str = "ascension.runtime-v3.memory-policy-owner-config.v1";
const MAX_CONFIG_BYTES: usize = 64 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeploymentConfig {
    schema: String,
    scope: MemoryScope,
    corpus_store_path: PathBuf,
    policy_store_path: PathBuf,
    policy_store_consent_ref: String,
    auth_profile: String,
    selector_grant_id: String,
    operator_grant_id: String,
    phase2_revision_id: String,
    control_epoch: u64,
    plan_epoch: u64,
    owner_epoch: u64,
    grants: Vec<DeploymentGrant>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeploymentGrant {
    grant_id: String,
    subject: String,
    permissions: BTreeSet<PolicyPermission>,
    epoch: u64,
    expires_at: u64,
    revoked: bool,
}

struct RuntimePolicyClock;

impl PolicyClock for RuntimePolicyClock {
    fn now_seconds(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(u64::MAX, |duration| duration.as_secs())
    }

    fn now_timestamp(&self) -> String {
        utc_timestamp(self.now_seconds())
    }
}

/// Trusted runtime composition for the existing durable ContextMemory owner.
/// It never imports, approves, or adopts a policy by itself.
pub(super) struct RuntimeGameInformationOwner {
    pub(super) scope: MemoryScope,
    pub(super) authority: Arc<MemoryPolicyAuthority>,
    pub(super) owner: Arc<MemoryPolicyOwner>,
    pub(super) corpus_store: Mutex<DurableMemoryStore>,
    pub(super) authenticator: Arc<EnvironmentAuthenticator>,
    selector_grant_id: String,
    operator_grant_id: String,
    bearer: Zeroizing<String>,
    pub(super) deployment_sha256: String,
}

impl RuntimeGameInformationOwner {
    pub(super) fn from_environment(expected_scope: MemoryScope) -> Result<Arc<Self>, String> {
        let config_path = required_environment("STS2_LOOKUP_OWNER_CONFIG")?;
        let expected_sha256 = required_environment("STS2_LOOKUP_OWNER_CONFIG_SHA256")?;
        if !valid_digest(&expected_sha256) {
            return Err(String::from(
                "STS2_LOOKUP_OWNER_CONFIG_SHA256 must be a lowercase SHA256 digest",
            ));
        }
        let (config, deployment_sha256) = read_config(Path::new(&config_path), &expected_sha256)?;
        if config.schema != CONFIG_SCHEMA || config.scope != expected_scope {
            return Err(String::from(
                "lookup owner configuration schema or runtime scope does not match",
            ));
        }
        if config.corpus_store_path == config.policy_store_path
            || !config.corpus_store_path.is_absolute()
            || !config.policy_store_path.is_absolute()
        {
            return Err(String::from(
                "lookup owner stores must use distinct absolute paths",
            ));
        }
        validate_store_path(&config.corpus_store_path)?;
        validate_store_path(&config.policy_store_path)?;
        if !valid_id(&config.policy_store_consent_ref)
            || !valid_id(&config.auth_profile)
            || !valid_id(&config.selector_grant_id)
            || !valid_id(&config.operator_grant_id)
            || !valid_id(&config.phase2_revision_id)
            || [config.control_epoch, config.plan_epoch, config.owner_epoch]
                .iter()
                .any(|value| *value == 0 || *value > 9_007_199_254_740_991)
        {
            return Err(String::from(
                "lookup owner configuration contains an invalid identity or epoch",
            ));
        }

        let auth_environment_name = workflow_token_environment_name(&config.auth_profile)?;
        let bearer = Zeroizing::new(
            std::env::var(&auth_environment_name)
                .map_err(|_| format!("{auth_environment_name} is not set"))?,
        );
        let authenticator = Arc::new(
            EnvironmentAuthenticator::from_profile(&config.auth_profile)
                .map_err(|_| String::from("lookup owner authenticator configuration is invalid"))?,
        );
        let subject = format!("profile:{}", config.auth_profile);
        let grants = deployment_grants(&config, &subject)?;

        let corpus_key = read_key_environment("STS2_LOOKUP_CORPUS_STORE_KEY_HEX")?;
        let policy_key = read_key_environment("STS2_LOOKUP_POLICY_STORE_KEY_HEX")?;
        let corpus_path = config
            .corpus_store_path
            .to_str()
            .ok_or("lookup corpus store path is not valid UTF-8")?;
        let corpus_store = DurableMemoryStore::open(corpus_path, config.scope.clone(), corpus_key)
            .map_err(|_| String::from("lookup corpus store is unavailable"))?;
        let corpus = corpus_store
            .load_corpus()
            .map_err(|_| String::from("lookup corpus cannot be authenticated"))?;
        let capabilities = corpus.capabilities();
        let authority = Arc::new(
            MemoryPolicyAuthority::new(
                TrustedPolicyState {
                    trusted_owner_revision: capabilities.binding.owner_revision.clone(),
                    trusted_adapter_revision: capabilities.binding.adapter_revision.clone(),
                    trusted_policy_schema_sha256: memory_policy_schema_sha256(),
                    phase2_revision_id: config.phase2_revision_id,
                    control_epoch: config.control_epoch,
                    plan_epoch: config.plan_epoch,
                    owner_epoch: config.owner_epoch,
                    corpus: corpus.clone(),
                    capabilities,
                    grants,
                },
                authenticator.clone(),
                Arc::new(RuntimePolicyClock),
            )
            .map_err(|_| String::from("lookup owner trusted configuration is invalid"))?,
        );
        let policy_path = config
            .policy_store_path
            .to_str()
            .ok_or("lookup policy store path is not valid UTF-8")?;
        let owner = Arc::new(
            MemoryPolicyOwner::open(
                policy_path,
                policy_key,
                authority.clone(),
                PolicyStoreConsent::ApprovedPrivate {
                    policy_ref: config.policy_store_consent_ref,
                },
            )
            .map_err(|_| String::from("lookup policy owner store is unavailable"))?,
        );

        Ok(Arc::new(Self {
            scope: config.scope,
            authority,
            owner,
            corpus_store: Mutex::new(corpus_store),
            authenticator,
            selector_grant_id: config.selector_grant_id,
            operator_grant_id: config.operator_grant_id,
            bearer,
            deployment_sha256,
        }))
    }

    pub(super) fn access<'a>(&'a self, grant_id: &'a str) -> PolicyAccess<'a> {
        PolicyAccess {
            bearer: Some(self.bearer.as_str()),
            grant_id,
        }
    }

    pub(super) fn selector_grant_id(&self) -> &str {
        &self.selector_grant_id
    }

    pub(super) fn operator_grant_id(&self) -> &str {
        &self.operator_grant_id
    }

    pub(super) fn lookup_snapshot(
        &self,
        expected: Option<&ActivePolicyBinding>,
    ) -> Result<LookupPolicySnapshot, PolicyOwnerError> {
        self.owner
            .lookup_snapshot(self.access(&self.selector_grant_id), expected)
    }

    pub(super) fn lock_lookup_snapshot(
        &self,
        expected: &ActivePolicyBinding,
    ) -> Result<LookupPolicyAuthorityGuard<'_>, PolicyOwnerError> {
        self.owner
            .lock_lookup_snapshot(self.access(&self.selector_grant_id), expected)
    }
}

fn deployment_grants(
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

fn read_config(path: &Path, expected_sha256: &str) -> Result<(DeploymentConfig, String), String> {
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

fn validate_store_path(path: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|_| String::from("configured lookup owner store is unavailable"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(String::from(
            "configured lookup owner store must be an existing regular file",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(String::from(
                "configured lookup owner stores must not be accessible by group or other users",
            ));
        }
    }
    Ok(())
}

fn read_key_environment(name: &str) -> Result<[u8; 32], String> {
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

fn workflow_token_environment_name(profile: &str) -> Result<String, String> {
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

fn required_environment(name: &str) -> Result<String, String> {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => Ok(value),
        Ok(_) => Err(format!("{name} must not be empty")),
        Err(std::env::VarError::NotPresent) => Err(format!("{name} is required")),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} is not valid UTF-8")),
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && b"._:-".contains(&byte))
        })
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn utc_timestamp(seconds: u64) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_timestamp_conversion_is_utc_and_calendar_valid() {
        assert_eq!(utc_timestamp(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc_timestamp(1_789_516_800), "2026-09-16T00:00:00Z");
    }
}
