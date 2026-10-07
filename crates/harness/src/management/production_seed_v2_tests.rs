// SPDX-License-Identifier: MIT

#[cfg(target_os = "linux")]
#[path = "production_seed_v2_arbitration_tests.rs"]
mod arbitration;

#[cfg(target_os = "linux")]
#[path = "production_seed_v2_replay_tests.rs"]
mod replay;

#[cfg(target_os = "linux")]
#[path = "production_seed_v2_process_tests.rs"]
mod process;

#[cfg(target_os = "linux")]
#[path = "production_seed_v2_reservation_tests.rs"]
mod reservation;

#[cfg(target_os = "linux")]
#[path = "production_seed_v2_reservation_process_tests.rs"]
mod reservation_process;

#[cfg(test)]
#[path = "production_seed_v2_policy_tests.rs"]
mod policy;
