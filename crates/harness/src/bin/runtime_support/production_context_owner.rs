// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde::Deserialize;
use sts2_harness::context_control::{
    ContextBoundary, ContextControlStore, ControlAuthority, StoreMode,
};
use sts2_harness::management::{
    AuthContext, ContextBindingCatalog, ContextBindingContinuity, ContextBindingDescriptor,
    ContextBindingGrants, ContextBindingRequest, ContextBindingState, ContextEffectiveLimits,
    ContextOwnerBinding, ContextOwnerPort, LiveContextObservationPort, ManagementError, RunRequest,
};
use sts2_harness::{EpisodeObservation, sha256_hex};
use zeroize::Zeroize;

const SCHEMA: &str = "ascension.workflow-context-owner-config.v1";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Configuration {
    pub schema_version: String,
    pub store_path: std::path::PathBuf,
    pub key_reference: String,
    pub owner_id: String,
    pub owner_version: String,
    pub context_ref: String,
    pub limits: ContextEffectiveLimits,
}

pub(super) struct Owner {
    configuration: Configuration,
    key: [u8; 32],
    current: Mutex<BTreeMap<String, Current>>,
}

struct Current {
    authority: ControlAuthority,
    store: ContextControlStore,
    actor: String,
}

impl Configuration {
    pub(super) fn from_environment() -> Result<Self, String> {
        let raw = std::env::var("STS2_WORKFLOW_CONTEXT_OWNER_CONFIG")
            .map_err(|_| String::from("STS2_WORKFLOW_CONTEXT_OWNER_CONFIG is required"))?;
        let value: Self = serde_json::from_str(&raw)
            .map_err(|_| String::from("STS2_WORKFLOW_CONTEXT_OWNER_CONFIG is invalid"))?;
        if value.schema_version != SCHEMA
            || value.owner_id.is_empty()
            || value.owner_version.is_empty()
            || value.context_ref.is_empty()
            || value.key_reference.is_empty()
        {
            return Err(String::from(
                "STS2_WORKFLOW_CONTEXT_OWNER_CONFIG is invalid",
            ));
        }
        if value.limits.max_items == 0
            || value.limits.max_context_bytes == 0
            || value.limits.max_objective_bytes == 0
            || value.limits.max_control_events == 0
            || value.limits.max_items > 64
            || value.limits.max_notes > 16
            || value.limits.max_context_bytes > 128 * 1024
            || value.limits.max_objective_bytes > 512
            || value.limits.max_control_events > 4096
        {
            return Err(String::from(
                "STS2_WORKFLOW_CONTEXT_OWNER_CONFIG limits are invalid",
            ));
        }
        Ok(value)
    }
}

impl Owner {
    pub(super) fn open(configuration: Configuration) -> Result<Self, String> {
        let mut key = key(&configuration.key_reference)?;
        let result = Self {
            configuration,
            key,
            current: Mutex::new(BTreeMap::new()),
        };
        key.zeroize();
        Ok(result)
    }

    fn descriptor(&self) -> Result<ContextBindingDescriptor, ManagementError> {
        ContextBindingDescriptor {
            schema_version: sts2_harness::management::CONTEXT_OWNER_BINDING_SCHEMA_VERSION.into(),
            binding_id: format!("{}.decide.v1", self.configuration.owner_id),
            version: 1,
            digest: String::new(),
            context_ref: self.configuration.context_ref.clone(),
            node_kinds: vec!["decide".into()],
            sources: Vec::new(),
            operations: Vec::new(),
            effective_limits: self.configuration.limits.clone(),
            continuity: ContextBindingContinuity {
                survives_controller_restart: true,
                receipt_recovery: false,
                provider_session_continuity: false,
            },
            grants: ContextBindingGrants {
                metadata_read: true,
                content_read: false,
                edit: false,
                control: false,
            },
            state: ContextBindingState::Available,
        }
        .seal()
    }
}

impl LiveContextObservationPort for Owner {
    fn record_observation(
        &self,
        actor: &AuthContext,
        request: &RunRequest,
        digest: &str,
        observation: &EpisodeObservation,
    ) -> Result<(), ManagementError> {
        let run_id = run_id(request, digest)?;
        let boundary = ContextBoundary {
            run_id: run_id.clone(),
            episode_id: request.request_id.clone(),
            agent_id: actor.subject.clone(),
            state_id: observation.state_id().into(),
            generation: observation.generation(),
            observation_sha256: sha256_hex(format!(
                "{}:{}",
                observation.state_id(),
                observation.generation()
            )),
            catalog_sha256: sha256_hex("runtime-v3-mcp-catalog"),
            adapter_revision: "runtime-v3-mcp".into(),
            model_revision: "exo.runtime-v3".into(),
            configuration_sha256: sha256_hex(self.configuration.owner_id.as_bytes()),
            output_schema_sha256: sha256_hex("context-owner-metadata-v1"),
            controller_epoch: 1,
            gate_epoch: 1,
            control_version: 1,
        };
        let mut current = self.current.lock().map_err(|_| {
            ManagementError::unavailable("context_owner_lock", "context owner is unavailable")
        })?;
        if let Some(entry) = current.get_mut(&run_id) {
            if entry.actor != actor.subject {
                return Err(ManagementError::forbidden(
                    "context_owner_actor",
                    "actor cannot replace this context authority",
                ));
            }
            entry
                .authority
                .record_observation_boundary(boundary)
                .map_err(|e| ManagementError::conflict("context_owner_observation_stale", e))?;
            entry
                .store
                .persist(&entry.authority, StoreMode::Enabled)
                .map_err(|e| {
                    ManagementError::unavailable("context_owner_persist", e.to_string())
                })?;
            return Ok(());
        }
        let authority = ControlAuthority::new(boundary, "context.revision.1")
            .with_max_control_events(self.configuration.limits.max_control_events)
            .map_err(|e| ManagementError::invalid("context_owner_limits", e))?;
        let store = ContextControlStore::create(
            &self.configuration.store_path,
            self.key,
            &run_id,
            &authority,
            StoreMode::Enabled,
        )
        .map_err(|e| ManagementError::unavailable("context_owner_store", e.to_string()))?;
        current.insert(
            run_id,
            Current {
                authority,
                store,
                actor: actor.subject.clone(),
            },
        );
        Ok(())
    }
    fn invalidate(&self, actor: &AuthContext, request: &RunRequest) {
        if let Ok(mut current) = self.current.lock() {
            current.retain(|_, entry| {
                entry.actor != actor.subject
                    || entry.authority.state().boundary.run_id != request.request_id
            });
        }
    }
}

impl ContextOwnerPort for Owner {
    fn catalog(&self, _actor: &AuthContext) -> Result<ContextBindingCatalog, ManagementError> {
        ContextBindingCatalog {
            schema_version: sts2_harness::management::CONTEXT_OWNER_CATALOG_SCHEMA_VERSION.into(),
            owner_id: self.configuration.owner_id.clone(),
            owner_version: self.configuration.owner_version.clone(),
            catalog_digest: String::new(),
            descriptors: vec![self.descriptor()?],
        }
        .seal()
    }
    fn bind(
        &self,
        actor: &AuthContext,
        request: &ContextBindingRequest,
    ) -> Result<ContextOwnerBinding, ManagementError> {
        let descriptor = self.descriptor()?;
        if request.context_ref != descriptor.context_ref
            || request.node_kind != "decide"
            || request.binding_id != descriptor.binding_id
            || request.binding_digest != descriptor.digest
        {
            return Err(ManagementError::conflict(
                "context_owner_binding_stale",
                "context binding is not current",
            ));
        }
        let current = self.current.lock().map_err(|_| {
            ManagementError::unavailable("context_owner_lock", "context owner is unavailable")
        })?;
        let entry = current.get(&request.workflow_run_id).ok_or_else(|| {
            ManagementError::unavailable(
                "context_owner_observation_missing",
                "current runtime observation is unavailable",
            )
        })?;
        if entry.actor != actor.subject {
            return Err(ManagementError::forbidden(
                "context_owner_actor",
                "actor cannot bind this context authority",
            ));
        }
        let state = entry.authority.state();
        Ok(ContextOwnerBinding {
            schema_version: sts2_harness::management::CONTEXT_OWNER_BINDING_SCHEMA_VERSION.into(),
            owner_id: self.configuration.owner_id.clone(),
            owner_version: self.configuration.owner_version.clone(),
            invocation_id: format!("{}.{}", request.workflow_run_id, request.node_execution_id),
            binding_id: descriptor.binding_id,
            binding_version: descriptor.version,
            binding_digest: descriptor.digest,
            context_ref: request.context_ref.clone(),
            instance_id: request.instance_id.clone(),
            node_kind: request.node_kind.clone(),
            state: ContextBindingState::Available,
            workflow_run_id: request.workflow_run_id.clone(),
            definition_digest: request.definition_digest.clone(),
            graph_id: request.graph_id.clone(),
            node_id: request.node_id.clone(),
            node_execution_id: request.node_execution_id.clone(),
            boundary: state.boundary.clone(),
            lease_epoch: state.boundary.controller_epoch,
            snapshot_id: format!("snapshot.{}", state.boundary.generation),
            approved_revision_id: state.active_revision_id.clone(),
            plan_epoch: state.plan_epoch,
            grants: descriptor.grants,
            continuity: descriptor.continuity,
        })
    }
}

fn run_id(request: &RunRequest, digest: &str) -> Result<String, ManagementError> {
    let bytes = serde_json::to_vec(&serde_json::json!({"request_id":request.request_id,"instance_id":request.instance_id,"definition_digest":digest})).map_err(|e| ManagementError::invalid("context_owner_run_id", e.to_string()))?;
    Ok(format!("run.live.{}", &sha256_hex(bytes)[..32]))
}
fn key(reference: &str) -> Result<[u8; 32], String> {
    let value = std::env::var(reference)
        .map_err(|_| format!("context-owner key reference {reference} is unavailable"))?;
    if value.len() != 64 {
        return Err(String::from(
            "context-owner key must be 64 hexadecimal characters",
        ));
    }
    let mut out = [0; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[i * 2..i * 2 + 2], 16)
            .map_err(|_| String::from("context-owner key must be hexadecimal"))?;
    }
    if out.iter().all(|b| *b == 0) {
        return Err(String::from("context-owner key must not be zero"));
    }
    Ok(out)
}
