// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest as Sha2Digest, Sha256};

use crate::runtime_v2_artifact::{
    RUNTIME_V2_ARTIFACT, RUNTIME_V2_GENERATOR, RUNTIME_V2_MAX_GENERATION,
    RUNTIME_V2_MAX_IDENTITY_BYTES, RUNTIME_V2_MAX_LEASE_EPOCH, RUNTIME_V2_MAX_RECORDS,
    RUNTIME_V2_MAX_TURN_INDEX, RUNTIME_V2_PROTOCOL_VERSION, RUNTIME_V2_SCHEMA_DIGEST,
    RUNTIME_V2_SCHEMA_SOURCE, RuntimeV2ArtifactError, verify_runtime_v2_artifact,
};

const INSTANCE_ID: &str = "instance-1";
const SESSION_ID: &str = "session-1";
const LEASE_ID: &str = "lease-1";
const LEASE_EPOCH: u64 = 1;
const INITIAL_GENERATION: u64 = 4;
const INITIAL_TURN_INDEX: u64 = 2;
const TRAJECTORY_ID: &str = "trajectory-runtime-v2-0001";
const TRACE_ARTIFACT_ID: &str = "artifact-runtime-v2-trace-0001";
const ACTION_ID: &str = "end_turn";
const WITNESS_KIND: &str = "turn_end_settled";
const UNKNOWN_ERROR: &str = "sts2.runtime/unknown_after_disconnect";
const STALE_EPOCH_ERROR: &str = "sts2.gateway/stale_lease_epoch";

include!("types.rs");
include!("message_core.rs");
include!("message_validation.rs");
include!("records_core.rs");
include!("records_summary.rs");
include!("runner.rs");
include!("fake_types.rs");
include!("fake_engine_one.rs");
include!("fake_engine_two.rs");
include!("support.rs");
include!("error.rs");
include!("multi_instance_types.rs");
include!("multi_instance_coordinator.rs");
include!("multi_instance_support.rs");
include!("multi_instance_tests.rs");
include!("tests.rs");
