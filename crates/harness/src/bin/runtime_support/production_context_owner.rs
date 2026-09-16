// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde::Deserialize;
use sts2_harness::context_control::{ContextControlStore, ControlAuthority};
use sts2_harness::management::{
    AuthContext, ContextBindingCatalog, ContextBindingContinuity, ContextBindingDescriptor,
    ContextBindingGrants, ContextBindingOperation, ContextBindingRequest, ContextBindingState,
    ContextEffectiveLimits, ContextOwnerBinding, ContextOwnerPort, LiveContextObservationPort,
    ManagementError, RunRequest, RuntimeAuthorityBinding,
};
use sts2_harness::{EpisodeLegalActionSet, EpisodeObservation};
use zeroize::Zeroize;

const SCHEMA: &str = "ascension.workflow-context-owner-config.v1";

#[path = "production_context_owner/binding.rs"]
mod binding;
#[path = "production_context_owner/observation.rs"]
mod observation;

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

impl Configuration {
    pub(super) fn from_environment() -> Result<Self, String> {
        Owner::configuration_from_environment()
    }
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
    catalog_generation: Option<u64>,
    runtime_lease_epoch: u64,
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
    if out.iter().all(|byte| *byte == 0) {
        return Err(String::from("context-owner key must not be zero"));
    }
    Ok(out)
}

fn run_id(request: &RunRequest, digest: &str) -> Result<String, ManagementError> {
    let value = serde_json::json!({
        "request_id": request.request_id,
        "instance_id": request.instance_id,
        "definition_digest": digest,
    });
    let bytes = serde_json::to_vec(&value)
        .map_err(|error| ManagementError::invalid("context_owner_run_id", error.to_string()))?;
    Ok(format!(
        "run.live.{}",
        &sts2_harness::sha256_hex(bytes)[..32]
    ))
}

fn scoped_store_path(base: &std::path::Path, run_id: &str) -> std::path::PathBuf {
    let extension = base
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("sqlite3");
    let stem = base
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("context");
    base.with_file_name(format!("{stem}.{run_id}.{extension}"))
}

fn legal_catalog_digest(actions: &EpisodeLegalActionSet) -> Result<String, ManagementError> {
    let values = actions
        .actions()
        .iter()
        .map(|action| {
            serde_json::json!({
                "action_id": action.action_id(),
                "kind": format!("{:?}", action.kind()),
            })
        })
        .collect::<Vec<_>>();
    serde_json::to_vec(&values)
        .map(sts2_harness::sha256_hex)
        .map_err(|error| ManagementError::invalid("context_catalog_encode", error.to_string()))
}
