// SPDX-License-Identifier: MIT

use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use sts2_harness::context_memory::policy_owner::{PolicyCommand, SavedPolicy, SavedPolicyRef};
use sts2_harness::context_memory::{
    DurableMemoryStore, MemoryScope, memory_policy_schema_sha256,
    policy_owner::{
        ActivePolicyBinding, MemoryPolicyAuthority, MemoryPolicyOwner, PolicyAccess, PolicyClock,
        PolicyOwnerError, PolicyStoreConsent, TrustedPolicyState,
    },
};
use sts2_harness::management::{
    Authenticator, EnvironmentAuthenticator, ManagementError, MemoryPolicyOwnerManagementPort,
};
use zeroize::Zeroizing;

use config::{
    CONFIG_SCHEMA, deployment_grants, read_config, read_key_environment, required_environment,
    utc_timestamp, valid_digest, valid_id, workflow_token_environment_name,
};

#[path = "runtime_v3_game_information_archive.rs"]
mod archive;
#[path = "runtime_v3_game_information_owner_authority.rs"]
mod authority;
#[path = "runtime_v3_game_information_owner_config.rs"]
mod config;
#[path = "runtime_v3_game_information_owner_management.rs"]
mod management;
#[path = "runtime_v3_game_information_preflight.rs"]
mod preflight;
pub(super) use preflight::begin_memory_policy_preflight;
#[cfg(test)]
pub(super) use preflight::{start_management_server, wait_for_owner_ready};

#[cfg(test)]
#[path = "runtime_v3_game_information_owner_tests.rs"]
pub(super) mod owner_management_tests;

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
    pub(super) owner: Arc<MemoryPolicyOwner>,
    pub(super) corpus_store: Mutex<DurableMemoryStore>,
    archive_store: Mutex<DurableMemoryStore>,
    pub(super) authenticator: Arc<dyn Authenticator>,
    selector_grant_id: String,
    operator_grant_id: String,
    bearer: Zeroizing<String>,
    pub(super) deployment_sha256: String,
    management_listen: SocketAddr,
    preflight_timeout_seconds: u64,
    replay_archive: bool,
    archive_retention_seconds: u64,
}

impl RuntimeGameInformationOwner {
    pub(super) fn from_environment(expected_scope: MemoryScope) -> Result<Arc<Self>, String> {
        ensure_private_sqlite_platform()?;
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
            || config.corpus_store_path == config.lookup_archive_store_path
            || config.policy_store_path == config.lookup_archive_store_path
            || !config.corpus_store_path.is_absolute()
            || !config.policy_store_path.is_absolute()
            || !config.lookup_archive_store_path.is_absolute()
        {
            return Err(String::from(
                "lookup owner stores must use distinct absolute paths",
            ));
        }
        if !config.management_listen.ip().is_loopback()
            || config.preflight_timeout_seconds == 0
            || config.preflight_timeout_seconds > 300
            || config.archive_retention_seconds == 0
            || config.archive_retention_seconds > 2_592_000
            || !valid_id(&config.policy_store_consent_ref)
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
        let authenticator: Arc<dyn Authenticator> = Arc::new(
            EnvironmentAuthenticator::from_profile(&config.auth_profile)
                .map_err(|_| String::from("lookup owner authenticator configuration is invalid"))?,
        );
        let subject = format!("profile:{}", config.auth_profile);
        let grants = deployment_grants(&config, &subject)?;

        let corpus_key = read_key_environment("STS2_LOOKUP_CORPUS_STORE_KEY_HEX")?;
        let policy_key = read_key_environment("STS2_LOOKUP_POLICY_STORE_KEY_HEX")?;
        let archive_key = read_key_environment("STS2_LOOKUP_ARCHIVE_STORE_KEY_HEX")?;
        let corpus_path = config
            .corpus_store_path
            .to_str()
            .ok_or("lookup corpus store path is not valid UTF-8")?;
        let corpus_store =
            DurableMemoryStore::open_private(corpus_path, config.scope.clone(), corpus_key)
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
        let archive_path = config
            .lookup_archive_store_path
            .to_str()
            .ok_or("lookup archive store path is not valid UTF-8")?;
        let archive_store =
            DurableMemoryStore::open_private(archive_path, config.scope.clone(), archive_key)
                .map_err(|_| String::from("lookup archive store is unavailable"))?;
        let owner = Arc::new(
            MemoryPolicyOwner::open_private(
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
            owner,
            corpus_store: Mutex::new(corpus_store),
            archive_store: Mutex::new(archive_store),
            authenticator,
            selector_grant_id: config.selector_grant_id,
            operator_grant_id: config.operator_grant_id,
            bearer,
            deployment_sha256,
            management_listen: config.management_listen,
            preflight_timeout_seconds: config.preflight_timeout_seconds,
            replay_archive: config.replay_archive,
            archive_retention_seconds: config.archive_retention_seconds,
        }))
    }

    pub(super) fn access<'a>(&'a self, grant_id: &'a str) -> PolicyAccess<'a> {
        PolicyAccess {
            bearer: Some(self.bearer.as_str()),
            grant_id,
        }
    }

    fn management_access<'a>(
        &'a self,
        bearer: Option<&'a str>,
        grant_id: &'a str,
    ) -> PolicyAccess<'a> {
        PolicyAccess { bearer, grant_id }
    }

    pub(super) fn management_listen(&self) -> SocketAddr {
        self.management_listen
    }

    pub(super) fn preflight_timeout_seconds(&self) -> u64 {
        self.preflight_timeout_seconds
    }

    pub(super) fn replay_archive(&self) -> bool {
        self.replay_archive
    }

    pub(super) fn archive_retention_seconds(&self) -> u64 {
        self.archive_retention_seconds
    }

    pub(super) fn persist_lookup_archive(
        &self,
        expected: &ActivePolicyBinding,
        session: &sts2_harness::game_information::LookupSession,
        corpus: &sts2_harness::context_memory::MemoryCorpus,
    ) -> Result<Option<(String, usize)>, String> {
        archive::persist(self, expected, session, corpus)
    }

    pub(super) fn restore_lookup_archive(
        &self,
        expected: &ActivePolicyBinding,
        binding: sts2_harness::game_information::LookupBinding,
        now: &str,
        expires_at: &str,
    ) -> Result<
        Option<(
            sts2_harness::game_information::LookupSession,
            sts2_harness::context_memory::MemoryCorpus,
        )>,
        String,
    > {
        archive::restore(self, expected, binding, now, expires_at)
    }

    pub(super) fn policy_now_seconds(&self) -> u64 {
        RuntimePolicyClock.now_seconds()
    }

    pub(super) fn policy_now_timestamp() -> String {
        RuntimePolicyClock.now_timestamp()
    }

    pub(super) fn policy_timestamp_after(seconds: u64) -> String {
        utc_timestamp(RuntimePolicyClock.now_seconds().saturating_add(seconds))
    }
}

#[cfg(unix)]
fn ensure_private_sqlite_platform() -> Result<(), String> {
    Ok(())
}

#[cfg(not(unix))]
fn ensure_private_sqlite_platform() -> Result<(), String> {
    Err(String::from(
        "runtime lookup owner requires no-follow private SQLite support on this platform",
    ))
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
