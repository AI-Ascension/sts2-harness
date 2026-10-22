// SPDX-License-Identifier: MIT

//! Harness-owned submission recovery. Claims never expire or grant permission to relaunch.

use serde::{Deserialize, Serialize};

use super::{ErrorBody, RunSubmissionResponse};

pub const MANAGEMENT_ERROR_V2: &str = "ascension.management-error/v2";
pub const SUBMISSION_SCHEMA_VERSION: &str = "ascension.workflow-submission/v1";
pub const MAX_SUBMISSION_CLAIMS: usize = 4096;

/// Private durable metadata: digests bind the authenticated subject and exact request.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SubmissionClaim {
    pub actor_digest: String,
    pub request_digest: String,
    pub outcome: Option<SubmissionClaimOutcome>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum SubmissionClaimOutcome {
    Submitted {
        response: RunSubmissionResponse,
    },
    Failed {
        workflow_run_id: Option<String>,
        error: ErrorBody,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubmissionClaimAcceptance {
    Acquired,
    Existing(SubmissionClaim),
    Conflict,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubmissionRecoveryState {
    Missing,
    Pending,
    Submitted,
    Failed,
}

/// Read-only recovery; `missing` allows only the same request identity to be submitted.
/// A pending/failed claim never authorizes a new request ID or a repeated launch.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SubmissionRecoveryResponse {
    pub schema_version: String,
    pub request_id: String,
    pub state: SubmissionRecoveryState,
    pub workflow_run_id: Option<String>,
    pub response: Option<RunSubmissionResponse>,
    pub error: Option<ErrorBody>,
}
