// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::json::MANAGEMENT_SCHEMA_VERSION;
use super::types::{CommandResponse, RunEvent, RunSnapshot};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PersistedStore {
    pub schema_version: String,
    pub submissions: BTreeMap<String, SubmissionIndex>,
    pub runs: BTreeMap<String, PersistedRun>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SubmissionIndex {
    pub request_digest: String,
    pub workflow_run_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PersistedRun {
    pub request_id: String,
    pub request_digest: String,
    pub snapshot: RunSnapshot,
    pub events: Vec<RunEvent>,
    pub oldest_sequence: Option<u64>,
    pub commands: BTreeMap<String, PersistedCommand>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PersistedCommand {
    pub request_digest: String,
    pub application_in_flight: bool,
    pub response: Option<CommandResponse>,
}

impl PersistedStore {
    pub fn empty() -> Self {
        Self {
            schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
            submissions: BTreeMap::new(),
            runs: BTreeMap::new(),
        }
    }
}
