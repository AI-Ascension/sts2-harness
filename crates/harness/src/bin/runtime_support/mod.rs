// SPDX-License-Identifier: MIT

use sts2_harness::DurableBranchStatus;

pub(super) mod branch_continuation_runtime;
mod config;
mod continuation_branches;
mod gateway_json;
mod http;
mod mcp;
mod mcp_process;
mod production_context_owner;
mod response_validation;
mod runtime_v3;
mod runtime_v3_admission;
mod runtime_v3_lifecycle_config;
mod runtime_v3_parse;
mod runtime_v3_settings;
mod runtime_v3_telemetry;
mod runtime_v3_wire;
mod seed_transport;
mod v1_projection;
mod workflow_service;

pub(crate) use config::RuntimeConfig;

pub(crate) fn run(config: RuntimeConfig) -> Result<(), String> {
    let selector = RuntimeConfig::branch_continuation_selector()?;
    reconcile_continuation_startup(&config, selector.as_ref())?;
    if selector.is_some()
        && !matches!(
            config.runtime_profile.as_str(),
            "runtime-v3-gameplay"
                | "negotiated-composition-v1"
                | "runtime-v4-expert"
                | "runtime-v4-expert-rest-action"
        )
    {
        return Err(String::from(
            "durable branch continuation requires a gameplay runtime profile",
        ));
    }
    if matches!(
        config.runtime_profile.as_str(),
        "runtime-v3-gameplay"
            | "negotiated-composition-v1"
            | "runtime-v4-expert"
            | "runtime-v4-expert-rest-action"
    ) {
        runtime_v3::run(config, selector)
    } else {
        mcp::run(config)
    }
}

pub(crate) fn serve_workflow() -> Result<(), String> {
    workflow_service::serve()
}

/// Reconciles half-created durable continuation branches before any episode is admitted or resumed.
///
/// The runtime binary owns its experiment scope: it constructs the durable branch store, resolves
/// every branch that never reached a terminal outcome, and refuses to continue if reconciliation
/// leaves a half-created branch behind.
fn reconcile_continuation_startup(
    config: &RuntimeConfig,
    selector: Option<&sts2_harness::BranchContinuationSelector>,
) -> Result<(), String> {
    let default_experiment_id = format!("experiment:{}", config.episode_id);
    let experiment_id = selector
        .map(sts2_harness::BranchContinuationSelector::experiment_id)
        .unwrap_or(&default_experiment_id);
    let reconciled = continuation_branches::reconcile_continuation_branches(
        &continuation_branch_store_path()?,
        continuation_branches::STARTUP_RECONCILE_OPERATION_PREFIX,
        experiment_id,
    )?;
    let unresolved = reconciled
        .iter()
        .filter(|branch| {
            matches!(
                branch.status,
                DurableBranchStatus::Pending
                    | DurableBranchStatus::Restoring
                    | DurableBranchStatus::Replaying
                    | DurableBranchStatus::Unknown
            )
        })
        .count();
    if unresolved != 0 {
        return Err(format!(
            "startup reconciliation left {unresolved} half-created continuation branch(es)"
        ));
    }
    Ok(())
}

/// Resolves the durable continuation branch store path.
///
/// `STS2_BRANCH_STORE_PATH` takes precedence; otherwise the store is a sibling of the execution
/// store so one owner-controlled directory holds every durable runtime database.
fn continuation_branch_store_path() -> Result<std::path::PathBuf, String> {
    if let Some(path) = optional_path("STS2_BRANCH_STORE_PATH")? {
        return Ok(path);
    }
    let execution = optional_path("STS2_EXECUTION_STORE_PATH")?
        .unwrap_or_else(|| std::path::PathBuf::from("harness-execution.sqlite3"));
    Ok(execution.with_file_name("harness-continuation-branches.sqlite3"))
}

fn optional_path(name: &str) -> Result<Option<std::path::PathBuf>, String> {
    match std::env::var(name) {
        Ok(value) if value.is_empty() => Err(format!("{name} must not be empty")),
        Ok(value) => Ok(Some(std::path::PathBuf::from(value))),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} is not valid UTF-8")),
    }
}
