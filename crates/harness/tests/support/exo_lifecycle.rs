// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, dead_code)]

use super::harness_api as sts2_harness;
use std::sync::{Arc, Mutex, MutexGuard};
use sts2_harness::exo_lifecycle::*;
use sts2_harness::provider_session::*;
use sts2_harness::*;

#[path = "provider_session.rs"]
mod expiry_support;

/// A per-call nonce for the fixture root.
///
/// The process id and the clock are not an isolation primitive on their own: clock granularity on a
/// loaded or virtualised host can exceed the interval between two tests, and two fixtures that
/// resolve to the same root fail in `create_dir` rather than in the code under test. Adding a
/// process-wide counter, which no two calls in the same process can share, removes that residue.
fn fixture_nonce() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    format!(
        "{}-{nanos}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

fn broker() -> ProviderSessionBroker {
    let scope = SessionScope::new(
        "project-fixture",
        "run-fixture",
        "episode-fixture",
        "agent-fixture",
    )
    .expect("scope");
    let mut policy = ProviderSessionPolicy::disabled(scope.clone());
    policy.mode = ProviderSessionMode::FixtureOnly;
    policy.credential_realm_ref = "fixture-realm".into();
    policy.profile_sha256 = sha256_hex("codex-app-server-fixture-v1");
    ProviderSessionBroker::new(
        scope,
        policy,
        NativeCapabilities::fixture(),
        "owner-fixture",
    )
    .expect("broker")
}

fn held_binding(broker: &mut ProviderSessionBroker) -> SessionBinding {
    let operation = broker
        .create_candidate(
            "owner-fixture",
            "create-prepared",
            "branch-prepared",
            SessionPurpose::Executable,
            expiry_support::expiry(),
        )
        .expect("candidate");
    broker
        .complete_candidate(
            "owner-fixture",
            &operation.operation_id,
            "native-thread-prepared",
        )
        .expect("binding")
}

pub struct Guard<'a>(MutexGuard<'a, bool>);
impl AuthorityGuard for Guard<'_> {}

#[derive(Default)]
pub struct Authority {
    pub revoked: Mutex<bool>,
    pub legacy_quiesced: Mutex<bool>,
}
impl LifecycleAuthorityPort for Authority {
    fn claim<'a>(
        &'a self,
        claim: &OwnerClaim<'_>,
    ) -> Result<Box<dyn AuthorityGuard + 'a>, LifecycleError> {
        if matches!(claim.kind, ClaimKind::ImportLegacy { .. })
            && !*self.legacy_quiesced.lock().expect("legacy authority")
        {
            return Err(LifecycleError::LegacyOwnerNotQuiesced);
        }
        self.guard()
    }
    fn admit<'a>(
        &'a self,
        _: &InvocationManifest,
    ) -> Result<Box<dyn AuthorityGuard + 'a>, LifecycleError> {
        self.guard()
    }
    fn consume<'a>(
        &'a self,
        _: &InvocationManifest,
        _: &str,
    ) -> Result<Box<dyn AuthorityGuard + 'a>, LifecycleError> {
        self.guard()
    }
}
impl Authority {
    fn guard(&self) -> Result<Box<dyn AuthorityGuard + '_>, LifecycleError> {
        let guard = self.revoked.lock().expect("authority");
        if *guard {
            return Err(LifecycleError::Fenced);
        }
        Ok(Box::new(Guard(guard)))
    }
}

pub struct Fixture {
    pub root: std::path::PathBuf,
    pub config: JournalConfig,
    pub manifest: InvocationManifest,
    pub input: Vec<u8>,
    pub broker: Option<ProviderSessionBroker>,
    pub policy: ProviderSessionPolicy,
    pub capabilities: NativeCapabilities,
    pub authority: Arc<Authority>,
    pub store: ExecutionStore,
    pub fingerprint: ExecutionFingerprint,
}

impl Fixture {
    pub fn new() -> Self {
        let root = std::env::temp_dir().join(format!("sts2-lifecycle-{}", fixture_nonce()));
        std::fs::create_dir(&root).expect("private fixture root");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
                .expect("private");
        }
        let request = parse_bridge_request(
            include_bytes!("../../../../protocol-artifact/exo-bridge-v1/golden/request.json"),
            MAX_INPUT_BYTES,
        )
        .expect("fixture request");
        let input = encode_bridge_request("request-1", "turn-1", &request, MAX_INPUT_BYTES)
            .expect("envelope");
        let mut broker = broker();
        let binding = held_binding(&mut broker);
        let prepared = broker
            .prepare_turn(
                "owner-fixture",
                &binding.binding_id,
                "prepared-lifecycle",
                "preview-1",
                "revision-1",
                "selection-1",
                "boundary-1",
                input.clone(),
                br#"{"type":"object"}"#.to_vec(),
                b"protected".to_vec(),
                Vec::new(),
                expiry_support::expiry(),
            )
            .expect("frozen bytes");
        broker
            .explicit_resume("owner-fixture", &binding.binding_id)
            .expect("resume");
        let operation = broker
            .admit_turn(
                "owner-fixture",
                &binding.binding_id,
                &prepared.prepared_id,
                "turn-lifecycle",
            )
            .expect("admit");
        let config = JournalConfig {
            directory: root.join("owner"),
            legacy_path: None,
            store_id: "journal-fixture".into(),
            scope: broker.scope().clone(),
            owner_binding_digest: sha256_hex("authenticated-owner-fixture"),
        };
        let manifest = manifest(&request, &input, &binding, &prepared, &operation);
        let policy = broker.policy().clone();
        let capabilities = broker.capabilities().clone();
        let mut store =
            ExecutionStore::open(ExecutionStoreConfig::new(root.join("execution.sqlite3")))
                .expect("store");
        let fingerprint = ExecutionFingerprint::new("seed", "build", "state", "config", "provider")
            .expect("fingerprint");
        store
            .start_episode(
                &ExecutionLineage::new(
                    &config.scope.run_id,
                    &config.scope.episode_id,
                    &manifest.episode_attempt_id,
                    &manifest.trajectory_id,
                )
                .expect("lineage"),
                &fingerprint,
            )
            .expect("episode");
        Self {
            root,
            config,
            manifest,
            input,
            broker: Some(broker),
            policy,
            capabilities,
            authority: Arc::new(Authority::default()),
            store,
            fingerprint,
        }
    }
    pub fn owner(&mut self) -> LifecycleOwner {
        LifecycleOwner::create(
            self.config.clone(),
            [7; 32],
            self.broker.take().expect("broker"),
            "owner-fixture".into(),
            self.authority.clone(),
        )
        .expect("owner")
    }
    pub fn reopen(&self) -> Result<LifecycleOwner, LifecycleError> {
        LifecycleOwner::open(
            self.config.clone(),
            [7; 32],
            "fresh-owner".into(),
            &self.policy,
            &self.capabilities,
            self.authority.clone(),
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.store.close();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn manifest(
    request: &ExoDecisionRequest,
    input: &[u8],
    binding: &SessionBinding,
    prepared: &PreparedSessionTurn,
    operation: &NativeOperation,
) -> InvocationManifest {
    InvocationManifest {
        scope: binding.scope.clone(),
        execution_id: request.model_execution_id.clone(),
        episode_attempt_id: "attempt-1".into(),
        trajectory_id: "trajectory-1".into(),
        provider_attempt_id: "provider-attempt-1".into(),
        reservation_id: "reservation-1".into(),
        binding_id: binding.binding_id.clone(),
        operation_id: operation.operation_id.clone(),
        prepared_id: prepared.prepared_id.clone(),
        request_id: "request-1".into(),
        host_turn_id: "turn-1".into(),
        input_digest: sha256_hex(input),
        input_length: input.len(),
        config_digest: sha256_hex("config"),
        package_digest: sha256_hex("package"),
        profile_digest: binding.profile_sha256.clone(),
        model_revision: request.provider_revision.clone(),
        reserved_units: 10,
        authority: AuthorityVector {
            owner_epoch: prepared.owner_epoch,
            auth_epoch: prepared.auth_epoch,
            session_epoch: prepared.session_epoch,
            history_epoch: prepared.history_epoch,
            compaction_epoch: prepared.compaction_epoch,
            revocation_epoch: prepared.revocation_epoch,
            lease_id: "lease-1".into(),
            lease_epoch: 1,
            state_id: request.state_id.clone(),
            generation: request.generation,
            catalog_digest: sha256_hex(
                serde_json::to_vec(&request.legal_action_ids).expect("catalog fixture"),
            ),
        },
    }
}

pub struct Effect {
    pub calls: usize,
    pub ambiguous: bool,
    pub units: Option<u64>,
}
impl Default for Effect {
    fn default() -> Self {
        Self {
            calls: 0,
            ambiguous: false,
            units: Some(3),
        }
    }
}
pub struct Handle {
    pub ready: bool,
    pub units: Option<u64>,
}
impl EffectPort for Effect {
    type Handle = Handle;
    fn try_start(&mut self, permit: SendPermit, _: &[u8]) -> Result<Handle, LifecycleError> {
        assert!(permit.revision() >= 4);
        assert!(permit.claim_epoch() > 0);
        self.calls += 1;
        if self.ambiguous {
            return Err(LifecycleError::Unavailable);
        }
        Ok(Handle {
            ready: true,
            units: self.units,
        })
    }
}
impl EffectHandle for Handle {
    fn poll(&mut self) -> Result<Option<EffectCompletion>, LifecycleError> {
        if !self.ready {
            return Ok(None);
        }
        self.ready = false;
        Ok(Some(EffectCompletion {
            response: encode_bridge_response(
                "request-1",
                "turn-1",
                ExoWireOutcome::Decision,
                Some(include_bytes!(
                    "../../../../protocol-artifact/exo-bridge-v1/golden/decision-action.json"
                )),
                None,
            )
            .expect("response"),
            result_ref: "result-fixture".into(),
            actual_units: self.units,
            native: Some(NativeIdentity {
                agent_id: "synthetic-agent".into(),
                conversation_id: "synthetic-conversation".into(),
                session_id: "synthetic-session".into(),
                turn_id: "synthetic-turn".into(),
                event_cursor: "synthetic-cursor".into(),
            }),
        }))
    }
}
