// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use std::collections::BTreeMap;
use sts2_harness::{
    EpisodeLegalActionSet, EpisodeObservation, EpisodeRunner, ExoDecisionSource, ExoProvider,
    ExoSession, ResumeState, TransitionReceipt,
};

use super::config::RuntimeConfig;
use super::http::GatewayClient;
use super::mcp::{McpProcess, identity_headers, release_correlation};
use super::runtime_v3_parse as parse;
use super::runtime_v3_settings::RuntimeV3Settings;
use super::runtime_v3_telemetry::{
    CleanupStatus, GameOutcome, ObservationSource, RuntimeV3Telemetry, TelemetryContext,
    TelemetryContextInput, TelemetryHandle, TelemetryStage,
};
use super::runtime_v3_wire as wire;

#[path = "runtime_allocation_context.rs"]
mod allocation_context;
#[path = "runtime_v3_completed_resume.rs"]
mod completed_resume;
#[path = "runtime_v3_decision_admission.rs"]
mod decision_admission;
#[path = "runtime_v3_decision_replay.rs"]
mod decision_replay;
#[path = "runtime_v3_durable.rs"]
mod durable;
#[path = "runtime_v3_lifecycle.rs"]
mod lifecycle;
#[path = "runtime_v3_lifecycle_authority.rs"]
mod lifecycle_authority;

#[path = "runtime_v3_combat_demo.rs"]
pub(crate) mod combat_demo;
#[path = "runtime_v3_episode.rs"]
mod episode;
#[path = "runtime_v3_episode_replay.rs"]
mod episode_replay;
#[path = "runtime_v4_expert_port.rs"]
mod expert;
#[path = "runtime_v3_ledger.rs"]
mod ledger;
#[path = "runtime_v3_receipt_query.rs"]
#[allow(dead_code)]
mod receipt_query;
#[path = "runtime_v3_recording.rs"]
mod recording;
#[path = "runtime_v3_recovery.rs"]
mod recovery;
#[path = "runtime_map.rs"]
mod runtime_map;
#[path = "runtime_v3_seeded.rs"]
mod seeded;
#[path = "runtime_v3_seeded_receipt.rs"]
mod seeded_receipt;
#[path = "runtime_v3_seeded_validation.rs"]
mod seeded_validation;
#[path = "runtime_v3_shutdown.rs"]
mod shutdown;
#[path = "runtime_v3_wait.rs"]
mod wait;

#[cfg(test)]
#[path = "runtime_v3_lifecycle_test.rs"]
mod lifecycle_tests;

include!("runtime_v3_run_combat.rs");

include!("runtime_v3_run.rs");

pub(super) struct RuntimeV3Port {
    config: RuntimeConfig,
    gateway: GatewayClient,
    mcp: Option<McpProcess>,
    seeded_mcp: Option<McpProcess>,
    seeded_receipt: Option<Value>,
    expert_mcp: Option<McpProcess>,
    allocated: bool,
    released: bool,
    next_rpc_id: u64,
    expert_next_rpc_id: u64,
    generation: u64,
    current_state: Option<String>,
    current_actions: Option<EpisodeLegalActionSet>,
    catalog: Option<Value>,
    catalog_raw: Option<Vec<u8>>,
    payloads: BTreeMap<String, Value>,
    rest_selector_actions: Option<EpisodeLegalActionSet>,
    rest_selector_payloads: BTreeMap<String, Value>,
    rest_selector_value: Option<Value>,
    operations: BTreeMap<String, ledger::OperationRecord>,
    reconnect_attempts: u8,
    telemetry: TelemetryHandle,
    durable: Option<durable::DurableHandle>,
    last_response_text: Option<String>,
    recovery_authority: Option<allocation_context::RecoveryAuthority>,
    recovery: Option<McpProcess>,
    recovery_context: Option<recovery::RecoveryContext>,
    recovery_rpc_id: u64,
    lifecycle_authority: lifecycle_authority::RuntimeLifecycleAuthorityState,
}

include!("runtime_v3_port.rs");
include!("runtime_v3_observation.rs");
