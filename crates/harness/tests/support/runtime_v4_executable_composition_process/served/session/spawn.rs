// SPDX-License-Identifier: MIT

//! Every path that spawns the served workflow service. Refs sts2-harness#673.
//!
//! Split out of `session.rs` only for file size. These three share one rule — the workflow
//! address's reservation is given up last, immediately before the fork, and never earlier — so
//! they live together where that rule is stated once.

use std::process::{Child, Command};

use super::WorkflowServiceConfig;
use super::workflow_service_command;

/// Spawn the served workflow service, giving up the workflow address's reservation first.
///
/// The command is built first because everything a scenario adds to it after
/// `workflow_service_command` — the cancellation barrier bounds, the receipt scenario's foreign
/// auth profile — is part of what gets spawned. Releasing inside `workflow_service_command`
/// would reopen the window for each of those mutations. Refs sts2-harness#673.
pub(crate) fn spawn_workflow_service(
    command: &mut Command,
    address: &super::super::super::ReservedAddress,
) -> Result<Child, Box<dyn std::error::Error>> {
    // The child binds this port, so the reservation has to be given up first. Everything the
    // scenario put on `command` is already set, so this is the last moment before the fork at
    // which nothing else can claim the port.
    command.env("STS2_WORKFLOW_LISTEN", address.release().to_string());
    Ok(command.spawn()?)
}

/// The same spawn for a scenario that has nothing further to add to the command.
///
/// Exists so those call sites cannot forget the release by building and spawning the command
/// themselves, which is the shape this file had before #673. Refs sts2-harness#673.
pub(crate) fn spawn_workflow_service_from(
    config: &WorkflowServiceConfig<'_>,
    address: &super::super::super::ReservedAddress,
) -> Result<Child, Box<dyn std::error::Error>> {
    spawn_workflow_service(&mut workflow_service_command(config)?, address)
}

/// [`spawn_workflow_service`] for the scenario that swaps the auth profile, which it does by
/// setting two variables on the command it is about to spawn.
///
/// The extra variables are applied by this function rather than at the call site so the release
/// still happens last, after everything the scenario wanted on the command is already there.
/// Refs sts2-harness#673.
pub(crate) fn spawn_workflow_service_as_foreign(
    config: &WorkflowServiceConfig<'_>,
    address: &super::super::super::ReservedAddress,
) -> Result<Child, Box<dyn std::error::Error>> {
    let mut command = workflow_service_command(config)?;
    command
        .env("STS2_WORKFLOW_AUTH_PROFILE", "foreign")
        .env("STS2_WORKFLOW_TOKEN_FOREIGN", "foreign-workflow-token");
    spawn_workflow_service(&mut command, address)
}
