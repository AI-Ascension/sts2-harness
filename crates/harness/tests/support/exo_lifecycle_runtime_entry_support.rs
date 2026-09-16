// SPDX-License-Identifier: MIT

const INSTANCE_ID: &str = "entry-instance";
const CALLER_ID: &str = "entry-caller";
const SESSION_ID: &str = "entry-session";
const LEASE_ID: &str = "entry-lease";
const RUN_ID: &str = "entry-run";
const EPISODE_ID: &str = "entry-episode";
const ATTEMPT_ID: &str = "entry-attempt";
const TRAJECTORY_ID: &str = "entry-trajectory";
const REQUEST_ID: &str = "entry-request";
const TURN_ID: &str = "entry-turn";

#[path = "exo_test_source.rs"]
mod exo_test_source;
#[path = "exo_lifecycle_runtime_entry_fixture.rs"]
mod fixture;
#[path = "exo_lifecycle_runtime_entry_peers.rs"]
mod peers;
#[path = "exo_lifecycle_runtime_entry_receipt.rs"]
mod receipt;
#[path = "exo_lifecycle_runtime_entry_runner.rs"]
mod runner;

pub fn run_offline_lifecycle_entry() -> Result<(), String> {
    runner::run_offline_lifecycle_entry()
}

pub(super) use exo_test_source::pinned_exo_test_source;
