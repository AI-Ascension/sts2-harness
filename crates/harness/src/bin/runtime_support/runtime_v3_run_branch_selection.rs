// SPDX-License-Identifier: MIT

fn select_branch_continuation(
    selector: Option<sts2_harness::BranchContinuationSelector>,
    resume_requested: bool,
    config: &mut RuntimeConfig,
) -> Result<Option<super::branch_continuation_runtime::SelectedBranchContinuation>, String> {
    let Some(selector) = selector else {
        return Ok(None);
    };
    if std::env::var("STS2_REPLAY_TRAJECTORY").is_ok_and(|value| !value.is_empty()) {
        return Err(String::from(
            "a selected branch uses its retained replay prefix, not STS2_REPLAY_TRAJECTORY",
        ));
    }
    let artifact_path = super::branch_continuation_runtime::artifact_store_path()?;
    let branch_store = super::continuation_branch_store_path()?;
    let selected = if resume_requested {
        super::branch_continuation_runtime::SelectedBranchContinuation::load_for_resume(
            &selector,
            &branch_store,
            &artifact_path,
        )?
    } else {
        super::branch_continuation_runtime::SelectedBranchContinuation::load(
            &selector,
            &branch_store,
            &artifact_path,
        )?
    };
    if matches!(
        selected.strategy(),
        sts2_harness::BranchContinuationStrategyPlan::ExactRestore { .. }
    ) {
        return Err(String::from(
            "exact branch continuation is unavailable: no fixed game-mod/MCP/gateway restore route is installed",
        ));
    }
    super::branch_continuation_runtime::bind_branch_identities(&selected, config)?;
    Ok(Some(selected))
}
