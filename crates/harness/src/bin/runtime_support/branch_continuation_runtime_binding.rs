// SPDX-License-Identifier: MIT

/// Applies the durable branch's independently allocated run identities to the runtime config.
pub(crate) fn bind_branch_identities(
    selected: &SelectedBranchContinuation,
    config: &mut super::RuntimeConfig,
) -> Result<(), String> {
    let branch = selected.branch();
    let episode_id = branch
        .episode_id
        .as_deref()
        .ok_or_else(|| String::from("selected branch has no episode identity"))?;
    let trajectory_id = branch
        .trajectory_id
        .as_deref()
        .ok_or_else(|| String::from("selected branch has no trajectory identity"))?;
    let context_id = branch
        .context_id
        .as_deref()
        .ok_or_else(|| String::from("selected branch has no context identity"))?;
    for (name, value) in [
        ("branch run_id", branch.run_id.as_str()),
        ("branch episode_id", episode_id),
        ("branch trajectory_id", trajectory_id),
        ("branch context_id", context_id),
    ] {
        if !runtime_safe_identity(value) {
            return Err(format!("selected {name} is invalid"));
        }
    }
    if matches!(
        selected.strategy(),
        BranchContinuationStrategyPlan::PrefixReplay { .. }
    ) {
        let branch_seed = branch
            .effective_seed
            .as_deref()
            .ok_or_else(|| String::from("selected replay branch has no effective seed"))?;
        // A new Ready branch needs an explicit plan to start its destination. A Running branch
        // is resuming that existing destination, so its effective seed is checked against the
        // persisted child execution fingerprint after that exact execution record is opened.
        if !selected.is_resuming() {
            let runtime_seed = config
                .seed_transport
                .as_ref()
                .map(super::seed_transport::SeedTransportConfig::requested_seed)
                .ok_or_else(|| {
                    String::from("prefix continuation requires an explicit seed plan")
                })?;
            if branch_seed != runtime_seed {
                return Err(String::from(
                    "runtime seed does not match the selected branch effective seed",
                ));
            }
        }
    }
    config.run_id.clone_from(&branch.run_id);
    config.episode_id = episode_id.to_owned();
    config.trajectory_id = trajectory_id.to_owned();
    let branch_scope = format!("{}\0{}", branch.experiment_id, branch.branch_id);
    let scope_digest = sts2_harness::sha256_hex(branch_scope.as_bytes());
    config.trace_id = format!("branch-trace:{scope_digest}");
    config.artifact_id = format!("branch-artifact:{scope_digest}");
    config.validate()
}

fn runtime_safe_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value.contains("..")
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
        })
}
