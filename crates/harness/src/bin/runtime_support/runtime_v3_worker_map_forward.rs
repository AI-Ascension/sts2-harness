// SPDX-License-Identifier: MIT

use serde_json::Value;
use sts2_harness::{EpisodeRuntimePort, ModelExecutionId, PortError};

pub(super) fn forward_map_snapshot<P: EpisodeRuntimePort>(
    port: &mut P,
    state_id: &str,
    generation: u64,
    execution_id: ModelExecutionId,
) -> Result<Option<Value>, PortError> {
    port.map_snapshot(state_id, generation, execution_id)
}

#[cfg(test)]
#[path = "runtime_v3_worker_map_snapshot_tests.rs"]
mod tests;
