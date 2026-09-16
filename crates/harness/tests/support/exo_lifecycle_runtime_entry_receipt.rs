// SPDX-License-Identifier: MIT

use std::sync::Arc;
use sts2_harness::exo_lifecycle::{
    AuthorityGuard, InvocationManifest, JournalConfig, LifecycleAuthorityPort, LifecycleError,
    LifecycleOwner, LifecyclePhase, OwnerClaim,
};
use sts2_harness::provider_session::{
    ProviderSessionMetadataStore, ProviderSessionPolicyOwner, SessionScope,
};
use sts2_harness::sha256_hex;

use super::fixture::{self, Fixture};
use super::{EPISODE_ID, RUN_ID};

struct ReadGuard;
impl AuthorityGuard for ReadGuard {}

struct ReadAuthority;
impl LifecycleAuthorityPort for ReadAuthority {
    fn claim<'a>(
        &'a self,
        _: &OwnerClaim<'_>,
    ) -> Result<Box<dyn AuthorityGuard + 'a>, LifecycleError> {
        Ok(Box::new(ReadGuard))
    }

    fn admit<'a>(
        &'a self,
        _: &InvocationManifest,
    ) -> Result<Box<dyn AuthorityGuard + 'a>, LifecycleError> {
        Ok(Box::new(ReadGuard))
    }

    fn consume<'a>(
        &'a self,
        _: &InvocationManifest,
        _: &str,
    ) -> Result<Box<dyn AuthorityGuard + 'a>, LifecycleError> {
        Ok(Box::new(ReadGuard))
    }
}

pub(super) fn assert_persisted_receipt(fixture: &Fixture) -> Result<(), String> {
    let scope = SessionScope::new("entry-project", RUN_ID, EPISODE_ID, "entry-agent")
        .map_err(|error| error.to_string())?;
    let capabilities = fixture::lifecycle_capabilities(&fixture.inspected_identity)?;
    let policy_store =
        ProviderSessionMetadataStore::encrypted(&fixture.policy_store, [0x22; 32], scope.clone())
            .map_err(|error| error.to_string())?;
    let policy_owner =
        ProviderSessionPolicyOwner::open(policy_store, scope.clone(), capabilities.clone())
            .map_err(|error| format!("cannot reopen CLI policy store: {error}"))?;
    let (policy, _, _) = policy_owner
        .active()
        .map_err(|error| format!("CLI policy has no active revision: {error}"))?;
    drop(policy_owner);

    let journal = JournalConfig {
        directory: fixture.journal.clone(),
        legacy_path: None,
        store_id: String::from("entry-journal"),
        scope,
        owner_binding_digest: sha256_hex(b"entry-owner-token"),
    };
    let owner = LifecycleOwner::open(
        journal,
        [0x11; 32],
        String::from("entry-owner-token"),
        &policy,
        &capabilities,
        Arc::new(ReadAuthority),
    )
    .map_err(|error| format!("cannot reopen CLI lifecycle journal: {error}"))?;
    let [entry] = owner.entries() else {
        return Err(format!(
            "CLI lifecycle journal did not contain exactly one entry: {}",
            owner.entries().len()
        ));
    };
    let manifest = &entry.manifest;
    if entry.phase != LifecyclePhase::Completed
        || !entry.possible_write
        || manifest.request_id != "entry-request"
        || manifest.host_turn_id != "entry-turn"
        || manifest.package_digest
            != fixture
                .inspected_identity
                .package_digest
                .as_deref()
                .ok_or_else(|| String::from("inspected package digest is missing"))?
        || manifest.profile_digest != capabilities.profile_sha256
    {
        return Err(String::from(
            "persisted CLI receipt lost terminal state or package/profile correlation",
        ));
    }

    // The inspected config digest includes the exact extension digest pinned by the CLI config.
    let extension = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../experiments/exo-agent/extension/src/index.ts");
    let extension_digest = sha256_hex(std::fs::read(extension).map_err(|error| error.to_string())?);
    if fixture.inspected_identity.extension_digest.as_deref() != Some(extension_digest.as_str())
        || fixture.inspected_identity.config_digest.as_deref()
            != Some(fixture.bridge_config_digest.as_str())
    {
        return Err(String::from(
            "CLI inspected extension identity is not bound by the persisted configuration digest",
        ));
    }

    let input = std::fs::read(&fixture.input_log)
        .map_err(|error| format!("CLI input was not captured: {error}"))?;
    if manifest.input_length != input.len() || manifest.input_digest != sha256_hex(&input) {
        return Err(String::from(
            "persisted CLI receipt does not bind the exact bytes sent to the provider",
        ));
    }
    let native = entry
        .native
        .as_ref()
        .ok_or_else(|| String::from("persisted CLI receipt omitted native identity"))?;
    if native.agent_id != "entry-agent"
        || native.conversation_id != "entry-conversation"
        || native.session_id != "entry-native-session"
        || native.turn_id != "entry-native-turn"
        || native.event_cursor != "entry-event"
        || entry.result_ref.is_none()
        || entry.result_digest.is_none()
    {
        return Err(String::from(
            "persisted CLI receipt does not match the fake provider's native receipt",
        ));
    }
    Ok(())
}
