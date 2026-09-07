// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use serde_json::{Value, json};
use sts2_harness::{
    ActionIdentity, ActionKind, DispatchStatus, EpisodeLegalAction, EpisodeLegalActionSet,
    EpisodeObservation, EpisodeStage, RuntimeV4ExpertActionRequest, RuntimeV4ExpertActionResult,
    RuntimeV4ExpertActionStatus, RuntimeV4ExpertObservation, TransitionReceipt, WaitOutcome,
    WaitSample,
};

use super::{RuntimeV3Port, RuntimeV3ToolError, wire};

const PROFILE: &str = "runtime-v4-expert";
const STATE_TOOL: &str = "sts2.expert_state";
const ACTION_TOOL: &str = "sts2.expert_action";
const RECONCILE_TOOL: &str = "sts2.expert_reconcile";
const PROTOCOL_VERSION: &str = "runtime-v4-expert-action";
const PROFILE_NAME: &str = "expert-action";

#[derive(Clone, Debug)]
pub(super) struct ComposedExpertObservation {
    pub(super) observation: EpisodeObservation,
    pub(super) actions: EpisodeLegalActionSet,
    pub(super) payloads: BTreeMap<String, Value>,
}

include!("runtime_v4_expert_port_transport.rs");
include!("runtime_v4_expert_port_composition.rs");
include!("runtime_v4_expert_port_tests.rs");
