// SPDX-License-Identifier: MIT

//! Concrete standalone composition for the receipt-carrying Exo bridge.

use std::sync::Arc;

use sts2_harness::exo_admission::{AdmittedExoRuntimeTransport, ExoRuntimeAdmission};
use sts2_harness::exo_lifecycle::{
    AuthorityGuard, AuthorityVector, ExoLifecycleRuntimeTransport, InvocationManifest,
    JournalConfig, LifecycleAuthorityPort, LifecycleError, LifecycleOwner, LifecycleProcessEffect,
    OwnerClaim,
};
use sts2_harness::provider_session::{
    ProviderSessionBroker, ProviderSessionMetadataStore, ProviderSessionPolicyOwner, SessionScope,
};
use sts2_harness::{ExecutionCancellation, ExoTransport, sha256_hex};

use super::super::config::RuntimeConfig;
use super::super::runtime_v3_lifecycle_config::{RuntimeLifecycleConfig, RuntimeLifecycleSecrets};
use super::super::runtime_v3_settings::RuntimeV3Settings;
use super::durable::DurableHandle;

pub(super) struct RuntimeTransport(Box<dyn ExoTransport>);
impl ExoTransport for RuntimeTransport {
    fn exchange(
        &mut self,
        request: &[u8],
        max_response_bytes: usize,
        timeout_millis: u32,
    ) -> Result<Vec<u8>, sts2_harness::ExoTransportError> {
        self.0.exchange(request, max_response_bytes, timeout_millis)
    }
    fn close(&mut self) -> Result<(), sts2_harness::ExoTransportError> {
        self.0.close()
    }
}

struct RuntimeAuthority {
    scope: SessionScope,
    lease_id: String,
    lease_epoch: u64,
}
struct Guard;
impl AuthorityGuard for Guard {}

impl LifecycleAuthorityPort for RuntimeAuthority {
    fn claim<'a>(
        &'a self,
        request: &OwnerClaim<'_>,
    ) -> Result<Box<dyn AuthorityGuard + 'a>, LifecycleError> {
        if request.config.scope != self.scope || request.config.owner_binding_digest.is_empty() {
            return Err(LifecycleError::Fenced);
        }
        Ok(Box::new(Guard))
    }

    fn admit<'a>(
        &'a self,
        manifest: &InvocationManifest,
    ) -> Result<Box<dyn AuthorityGuard + 'a>, LifecycleError> {
        if manifest.scope != self.scope
            || manifest.authority.lease_id != self.lease_id
            || manifest.authority.lease_epoch != self.lease_epoch
        {
            return Err(LifecycleError::Stale);
        }
        Ok(Box::new(Guard))
    }

    fn consume<'a>(
        &'a self,
        manifest: &InvocationManifest,
        _result_digest: &str,
    ) -> Result<Box<dyn AuthorityGuard + 'a>, LifecycleError> {
        self.admit(manifest)
    }
}

/// Builds the only standalone runtime transport that can use lifecycle capability promotion.
/// Without the closed lifecycle configuration, the established generic admission route remains
/// available for legacy deployments.
pub(super) fn admit(
    config: &RuntimeConfig,
    settings: &RuntimeV3Settings,
    durable: DurableHandle,
) -> Result<RuntimeTransport, String> {
    let Some((lifecycle, secrets)) = settings.lifecycle.as_ref() else {
        return super::super::runtime_v3_admission::admit(
            &settings.admission,
            settings.process.clone(),
        )
        .map(|transport| RuntimeTransport(Box::new(transport)));
    };
    let transport = build(config, settings, durable, lifecycle, secrets)?;
    match &settings.admission {
        ExoRuntimeAdmission::Enveloped(plan) => plan
            .admit_lifecycle(transport)
            .map(|transport| {
                RuntimeTransport(Box::new(AdmittedExoRuntimeTransport::Enveloped(Box::new(
                    transport,
                ))))
            })
            .map_err(String::from),
        ExoRuntimeAdmission::Legacy => Err(String::from(
            "lifecycle transport requires reviewed envelope admission",
        )),
    }
}

fn build(
    config: &RuntimeConfig,
    settings: &RuntimeV3Settings,
    durable: DurableHandle,
    lifecycle: &RuntimeLifecycleConfig,
    secrets: &RuntimeLifecycleSecrets,
) -> Result<
    ExoLifecycleRuntimeTransport<
        impl FnMut(
            &str,
            &str,
            &sts2_harness::ExoDecisionRequest,
        ) -> Result<InvocationManifest, LifecycleError>
        + use<>,
    >,
    String,
> {
    let scope = SessionScope::new(
        lifecycle.project_id.clone(),
        config.run_id.clone(),
        config.episode_id.clone(),
        lifecycle.agent_id.clone(),
    )
    .map_err(|_| String::from("runtime lifecycle scope is invalid"))?;
    let [mode, configuration_path, _] = settings.process.arguments() else {
        return Err(String::from(
            "lifecycle bridge must have exactly --run-v2 configuration digest",
        ));
    };
    if mode != "--run-v2" {
        return Err(String::from(
            "lifecycle transport requires the --run-v2 bridge",
        ));
    }
    let inspected = sts2_harness::exo_bridge_configuration::load(configuration_path)
        .map_err(|_| String::from("cannot inspect lifecycle bridge configuration"))?;
    let inspected_identity = inspected
        .inspected_identity(
            std::path::Path::new(settings.process.executable()),
            &config.instance_id,
        )
        .map_err(|_| String::from("cannot bind inspected lifecycle deployment identity"))?;
    let capabilities = lifecycle.capabilities(&inspected_identity)?;
    let policy_store = ProviderSessionMetadataStore::encrypted(
        &lifecycle.policy_store_path,
        secrets.policy_key,
        scope.clone(),
    )
    .map_err(|_| String::from("lifecycle policy store is unavailable"))?;
    let policies =
        ProviderSessionPolicyOwner::open(policy_store, scope.clone(), capabilities.clone())
            .map_err(|_| String::from("lifecycle policy owner is unavailable"))?;
    let (policy, _, _) = policies
        .active()
        .map_err(|_| String::from("lifecycle policy has no adopted active revision"))?;
    lifecycle.validate_policy(&policy, &capabilities)?;
    let authority = Arc::new(RuntimeAuthority {
        scope: scope.clone(),
        lease_id: config.lease_id.clone(),
        lease_epoch: config.lease_epoch,
    });
    let journal = JournalConfig {
        directory: lifecycle.directory.clone(),
        legacy_path: lifecycle.legacy_path.clone(),
        store_id: lifecycle.store_id.clone(),
        scope: scope.clone(),
        owner_binding_digest: sha256_hex(secrets.owner_token.as_bytes()),
    };
    let owner = if lifecycle.directory.exists() {
        LifecycleOwner::open(
            journal,
            secrets.journal_key,
            secrets.owner_token.clone(),
            &policy,
            &capabilities,
            authority,
        )
    } else {
        let broker = ProviderSessionBroker::new(
            scope.clone(),
            policy,
            capabilities.clone(),
            secrets.owner_token.clone(),
        )
        .map_err(|_| String::from("active lifecycle policy cannot construct a broker"))?;
        LifecycleOwner::create(
            journal,
            secrets.journal_key,
            broker,
            secrets.owner_token.clone(),
            authority,
        )
    }
    .map_err(|_| String::from("cannot claim lifecycle journal owner"))?;
    let lineage = durable.lifecycle_lineage();
    let config_digest = durable.lifecycle_config_digest();
    let model_revision = settings.exo.revision.clone();
    let package_digest = inspected.config.executor_sha256;
    let profile_digest = capabilities.profile_sha256;
    let lease_id = config.lease_id.clone();
    let lease_epoch = config.lease_epoch;
    let effect = LifecycleProcessEffect::new(
        settings.process.clone(),
        ExecutionCancellation::default(),
        settings.exo.max_response_bytes,
        settings.exo.timeout_millis,
        1,
    )
    .map_err(|_| String::from("lifecycle process effect is invalid"))?;
    Ok(ExoLifecycleRuntimeTransport::new(
        owner,
        durable.lifecycle_store(),
        durable.lifecycle_fingerprint(),
        effect,
        move |request_id: &str, turn_id: &str, request: &sts2_harness::ExoDecisionRequest| {
            let catalog_digest = sha256_hex(
                serde_json::to_vec(&request.legal_action_ids)
                    .map_err(|_| LifecycleError::Invalid)?,
            );
            Ok(InvocationManifest {
                scope: scope.clone(),
                execution_id: request.model_execution_id.clone(),
                episode_attempt_id: lineage.attempt_id.clone(),
                trajectory_id: lineage.trajectory_id.clone(),
                provider_attempt_id: String::from("pending-provider-attempt"),
                reservation_id: String::from("pending-reservation"),
                binding_id: String::from("pending-binding"),
                operation_id: String::from("pending-operation"),
                prepared_id: String::from("pending-prepared"),
                request_id: request_id.to_owned(),
                host_turn_id: turn_id.to_owned(),
                input_digest: String::new(),
                input_length: 1,
                config_digest: config_digest.clone(),
                package_digest: package_digest.clone(),
                profile_digest: profile_digest.clone(),
                model_revision: model_revision.clone(),
                reserved_units: 1,
                authority: AuthorityVector {
                    owner_epoch: 1,
                    auth_epoch: 1,
                    session_epoch: 1,
                    history_epoch: 0,
                    compaction_epoch: 0,
                    revocation_epoch: 0,
                    lease_id: lease_id.clone(),
                    lease_epoch,
                    state_id: request.state_id.clone(),
                    generation: request.generation,
                    catalog_digest,
                },
            })
        },
    ))
}
