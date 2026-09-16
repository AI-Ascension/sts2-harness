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

#[cfg(test)]
mod bootstrap_tests {
    use super::*;
    use crate::runtime_support::runtime_v3_lifecycle_config::RuntimeLifecycleConfig;
    use sts2_harness::provider_session::{ProviderSessionMode, ProviderSessionPolicy};
    use sts2_harness::{
        EXO_SOURCE_REVISION, ExecutionFingerprint, ExecutionLineage, ExecutionStore, ExoConfig,
        ExoProcessConfig,
    };

    fn root() -> std::path::PathBuf {
        std::path::PathBuf::from(format!(
            "/tmp/sts2-lifecycle-bootstrap-{}",
            std::process::id()
        ))
    }

    #[test]
    #[ignore = "requires the reviewed external Exo checkout prepared by the executable fixture"]
    fn inspected_adopted_bootstrap_selects_lifecycle_before_any_effect() {
        let source = std::path::PathBuf::from("/tmp/sts2-exo-source-b068");
        assert_eq!(
            std::process::Command::new("/usr/bin/git")
                .args(["-C", source.to_str().expect("source"), "rev-parse", "HEAD"])
                .output()
                .expect("git")
                .stdout,
            format!("{EXO_SOURCE_REVISION}\n").as_bytes()
        );
        let base = root();
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).expect("fixture directory");
        let executor = base.join("executor");
        std::fs::write(&executor, "#!/bin/sh\nexit 97\n").expect("executor");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&executor, std::fs::Permissions::from_mode(0o700))
                .expect("executor mode");
        }
        let extension = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../experiments/exo-agent/extension/src/index.ts");
        let node = std::path::PathBuf::from("/usr/local/bin/node");
        let configuration = base.join("bridge.json");
        let config_bytes = serde_json::json!({
            "schema":"sts2.exo-one-shot-config-v1",
            "executor":executor,
            "executor_sha256":sha256_hex(std::fs::read(&executor).expect("executor bytes")),
            "source_root":source,
            "extension":extension,
            "extension_sha256":sha256_hex(std::fs::read(&extension).expect("extension bytes")),
            "node":node,
            "node_sha256":sha256_hex(std::fs::read(&node).expect("node bytes")),
            "model":"o3-pro",
            "endpoint":"https://api.openai.com/v1"
        });
        let original_configuration = serde_json::to_vec(&config_bytes).expect("config");
        std::fs::write(&configuration, &original_configuration).expect("configuration");
        let digest = sha256_hex(std::fs::read(&configuration).expect("configuration bytes"));
        let (lifecycle, secrets) =
            RuntimeLifecycleConfig::bootstrap_test(base.join("journal"), base.join("policy.bin"));
        let process = ExoProcessConfig::new(
            executor.to_string_lossy(),
            vec![
                String::from("--run-v2"),
                configuration.to_string_lossy().into_owned(),
                digest,
            ],
            None,
            Vec::new(),
        )
        .expect("process");
        let inspected =
            sts2_harness::exo_bridge_configuration::load(process.arguments()[1].as_str())
                .expect("inspection");
        let identity = inspected
            .inspected_identity(
                std::path::Path::new(process.executable()),
                "bootstrap-instance",
            )
            .expect("identity");
        let capabilities = lifecycle.capabilities(&identity).expect("capabilities");
        let scope = SessionScope::new(
            "bootstrap-project",
            "bootstrap-run",
            "bootstrap-episode",
            "bootstrap-agent",
        )
        .expect("scope");
        let owner = ProviderSessionPolicyOwner::open(
            ProviderSessionMetadataStore::encrypted(
                &lifecycle.policy_store_path,
                secrets.policy_key,
                scope.clone(),
            )
            .expect("policy store"),
            scope.clone(),
            capabilities.clone(),
        )
        .expect("policy owner");
        let mut policy = ProviderSessionPolicy::disabled(scope.clone());
        policy.mode = ProviderSessionMode::FixtureOnly;
        policy.credential_realm_ref = String::from("bootstrap-realm");
        policy.profile_sha256 = capabilities.profile_sha256.clone();
        let bytes = serde_json::to_vec(&policy).expect("policy");
        let policy_digest = owner.import(bytes).expect("import");
        owner.adopt_imported(&policy_digest, 2).expect("adopt");
        let config = RuntimeConfig {
            seed_transport: None,
            gateway_address: String::new(),
            gateway_token: String::new(),
            mcp_binary: String::new(),
            runtime_profile: String::from("runtime-v3-gameplay"),
            instance_id: String::from("bootstrap-instance"),
            caller_id: String::from("bootstrap-caller"),
            session_id: String::from("bootstrap-session"),
            lease_id: String::from("bootstrap-lease"),
            lease_epoch: 1,
            mcp_session_id: String::from("bootstrap-mcp"),
            run_id: String::from("bootstrap-run"),
            episode_id: String::from("bootstrap-episode"),
            trajectory_id: String::from("bootstrap-trajectory"),
            trace_id: String::from("bootstrap-trace"),
            artifact_id: String::from("bootstrap-artifact"),
            wait_for_combat_seconds: 0,
            settlement_timeout_seconds: 1,
            map_context_enabled: false,
            recovery_environment: Vec::new(),
        };
        let fingerprint =
            ExecutionFingerprint::new("seed", "build", "state", sha256_hex("config"), "provider")
                .expect("fingerprint");
        let lineage = ExecutionLineage::new(
            "bootstrap-run",
            "bootstrap-episode",
            "bootstrap-attempt",
            "bootstrap-trajectory",
        )
        .expect("lineage");
        let mut store = ExecutionStore::open_in_memory().expect("store");
        store
            .start_episode(&lineage, &fingerprint)
            .expect("episode");
        let durable =
            DurableHandle::from_store_for_test(store, lineage, fingerprint).expect("durable");
        let settings = RuntimeV3Settings {
            runner: sts2_harness::EpisodeRunnerConfig::new(
                1,
                sts2_harness::StabilityBarrier::new(1, 1).expect("barrier"),
                sts2_harness::RecoveryController::new(1).expect("recovery"),
                "objective",
                Vec::new(),
            )
            .expect("runner"),
            exo: ExoConfig::new(EXO_SOURCE_REVISION, 8192, 8192, 1000).expect("exo"),
            process,
            admission: ExoRuntimeAdmission::legacy(),
            lifecycle: Some((lifecycle, secrets)),
        };
        let (lifecycle, secrets) = settings.lifecycle.as_ref().expect("lifecycle");
        std::fs::write(&configuration, br#"{"schema":"swapped"}"#).expect("swap");
        assert!(build(&config, &settings, durable.clone(), lifecycle, secrets).is_err());
        std::fs::write(&configuration, original_configuration).expect("restore");
        assert!(build(&config, &settings, durable, lifecycle, secrets).is_ok());
        let _ = std::fs::remove_dir_all(base);
    }
}
