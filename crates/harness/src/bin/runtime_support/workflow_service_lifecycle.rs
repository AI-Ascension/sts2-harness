// SPDX-License-Identifier: MIT

//! Composes the served-live gateway process-lifecycle owner, when opted in.

use super::super::gateway_lifecycle::GatewayLifecyclePort;
use super::*;

/// Composes the gateway process-lifecycle owner, when the deployment opts in.
///
/// `STS2_LIFECYCLE_JOURNAL_DIR` names the owner-controlled directory holding the
/// durable intent journal. Opt-in rather than default-on because the directory
/// is a deployment decision: without it, the surface stays composed but
/// unavailable and refuses every lifecycle command instead of accepting an
/// operation it could not reconcile after a restart.
pub(super) fn owner() -> Result<Option<sts2_harness::management::ProcessLifecycleOwner>, String> {
    let Some(directory) = std::env::var("STS2_LIFECYCLE_JOURNAL_DIR")
        .ok()
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    let config = RuntimeConfig::from_environment()?;
    let port: Arc<dyn sts2_harness::management::ProcessLifecyclePort> =
        Arc::new(GatewayLifecyclePort::new(&config).map_err(|error| error.to_string())?);
    let intents = sts2_harness::management::LifecycleIntentStore::open(&directory)
        .map_err(|error| error.to_string())?;
    Ok(Some((port, Arc::new(std::sync::Mutex::new(intents)))))
}
