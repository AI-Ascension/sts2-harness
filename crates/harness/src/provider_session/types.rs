// SPDX-License-Identifier: MIT

//! Versioned persistent-provider records and validation helpers.
//!
//! The records are split by invariant family so policy, binding, preparation, maintenance, and
//! telemetry changes remain independently reviewable.

#[path = "binding_operation.rs"]
mod binding_operation;
#[path = "capabilities.rs"]
mod capabilities;
#[path = "common.rs"]
mod common;
#[path = "history_prepared.rs"]
mod history_prepared;
#[path = "maintenance.rs"]
mod maintenance;
#[path = "scope_policy.rs"]
mod scope_policy;
#[path = "telemetry.rs"]
mod telemetry;

pub use binding_operation::*;
pub use capabilities::*;
pub use common::*;
pub use history_prepared::*;
pub use maintenance::*;
pub use scope_policy::*;
pub use telemetry::*;
