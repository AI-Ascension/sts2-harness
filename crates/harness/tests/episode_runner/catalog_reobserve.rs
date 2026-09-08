// SPDX-License-Identifier: MIT

use super::scenarios::blocked_state;
use super::*;

#[test]
fn stale_catalog_refreshes_before_any_provider_choice_or_dispatch() {
    let mut runtime = FakeRuntime::new(vec![
        state(EpisodeStage::Map, 0),
        state(EpisodeStage::Combat, 1),
        state(EpisodeStage::Defeat, 2),
    ]);
    runtime.catalog_errors = vec![PortError::new("catalog_reobserve", "refresh", true)];
    runtime.advance_catalog = true;
    let mut model = FakeModel::default();
    let report = runner()
        .run(&mut runtime, &mut model)
        .expect("fresh catalog completes");
    assert_eq!(report.terminal_stage(), EpisodeStage::Defeat);
    assert_eq!(report.recoveries(), 1);
    assert_eq!(runtime.catalog_calls, 2);
    assert_eq!(runtime.dispatches, 1);
    assert_eq!(model.calls, 1);
    assert_eq!(model.completions, vec![true]);
    assert!(runtime.released && runtime.mcp_closed && runtime.gateway_closed);
}

#[test]
fn nonactionable_reobserve_uses_barrier_before_terminal_without_policy_or_dispatch() {
    let mut runtime = FakeRuntime::new(vec![
        state(EpisodeStage::Map, 0),
        blocked_state(1),
        state(EpisodeStage::Victory, 2),
    ]);
    runtime.catalog_errors = vec![PortError::new("catalog_reobserve", "refresh", true)];
    runtime.advance_catalog = true;
    runtime.advance_idle = true;
    let mut model = FakeModel::default();

    let report = runner()
        .run(&mut runtime, &mut model)
        .expect("terminal successor after an idle reobserve");

    assert_eq!(report.terminal_stage(), EpisodeStage::Victory);
    assert_eq!(runtime.catalog_calls, 1);
    assert_eq!(runtime.catalog_requests, vec![(String::from("map-0"), 0)]);
    assert_eq!(runtime.reobserve_calls, 1);
    assert_eq!(runtime.idle_waits, 1);
    assert_eq!(runtime.dispatches, 0);
    assert_eq!(model.calls, 0);
    assert!(runtime.released && runtime.mcp_closed && runtime.gateway_closed);
}

#[test]
fn nonactionable_reobserve_does_not_reset_catalog_refresh_budget() {
    let mut runtime = FakeRuntime::new(vec![
        state(EpisodeStage::Map, 0),
        blocked_state(1),
        state(EpisodeStage::Combat, 2),
        state(EpisodeStage::Combat, 3),
        state(EpisodeStage::Combat, 4),
        state(EpisodeStage::Combat, 5),
    ]);
    runtime.catalog_errors = (0..4)
        .map(|_| PortError::new("catalog_reobserve", "refresh", true))
        .collect();
    runtime.advance_catalog = true;
    runtime.advance_idle = true;
    let mut model = FakeModel::default();

    assert_eq!(
        runner().run(&mut runtime, &mut model),
        Err(EpisodeRunnerError::Recovery(RecoveryError::Exhausted))
    );
    assert_eq!(runtime.catalog_calls, 4);
    assert_eq!(
        runtime.catalog_requests,
        vec![
            (String::from("map-0"), 0),
            (String::from("combat-2"), 2),
            (String::from("combat-3"), 3),
            (String::from("combat-4"), 4),
        ]
    );
    assert_eq!(runtime.reobserve_calls, 3);
    assert_eq!(runtime.idle_waits, 1);
    assert_eq!(runtime.dispatches, 0);
    assert_eq!(model.calls, 0);
    assert!(runtime.released && runtime.mcp_closed && runtime.gateway_closed);
}

#[test]
fn three_catalog_refreshes_use_fresh_observations_before_policy() {
    let mut runtime = FakeRuntime::new(vec![
        state(EpisodeStage::Map, 0),
        state(EpisodeStage::Combat, 1),
        state(EpisodeStage::Combat, 2),
        state(EpisodeStage::Combat, 3),
        state(EpisodeStage::Defeat, 4),
    ]);
    runtime.catalog_errors = (0..3)
        .map(|_| PortError::new("catalog_reobserve", "refresh", true))
        .collect();
    runtime.advance_catalog = true;
    let mut model = FakeModel::default();

    let report = runner()
        .run(&mut runtime, &mut model)
        .expect("fresh catalog reaches policy");

    assert_eq!(report.terminal_stage(), EpisodeStage::Defeat);
    assert_eq!(runtime.catalog_calls, 4);
    assert_eq!(runtime.reobserve_calls, 3);
    assert_eq!(runtime.dispatches, 1);
    assert_eq!(model.calls, 1);
    assert_eq!(model.input_generations, vec![3]);
    assert_eq!(model.completions, vec![true]);
    assert!(runtime.released && runtime.mcp_closed && runtime.gateway_closed);
}

#[test]
fn persistent_or_untyped_catalog_failure_stops_with_owned_cleanup() {
    for (code, retryable, calls, reobserves) in [
        ("catalog_reobserve", true, 4, 3),
        ("catalog_reobserve", false, 1, 0),
        ("legal_actions_failed", true, 1, 0),
    ] {
        let mut runtime = FakeRuntime::new(vec![state(EpisodeStage::Combat, 0)]);
        runtime.catalog_errors = (0..4)
            .map(|_| PortError::new(code, "test", retryable))
            .collect();
        let mut model = FakeModel::default();
        let result = runner().run(&mut runtime, &mut model);
        if code == "catalog_reobserve" && retryable {
            assert_eq!(
                result,
                Err(EpisodeRunnerError::Recovery(RecoveryError::Exhausted))
            );
        } else {
            assert!(result.is_err());
        }
        assert_eq!(runtime.catalog_calls, calls);
        assert_eq!(runtime.reobserve_calls, reobserves);
        assert_eq!(runtime.dispatches, 0);
        assert_eq!(model.calls, 0);
        assert!(runtime.released && runtime.mcp_closed && runtime.gateway_closed);
    }
}

#[test]
fn terminal_reobserve_reports_validated_observation_without_second_observe() {
    let mut runtime = FakeRuntime::new(vec![
        state(EpisodeStage::Combat, 0),
        state(EpisodeStage::Victory, 1),
    ]);
    runtime.catalog_errors = vec![PortError::new("catalog_reobserve", "refresh", true)];
    runtime.advance_catalog = true;
    runtime.observe_after_reobserve = Some(Err(PortError::new(
        "post_reobserve_observe",
        "observe must not be called after terminal reobserve",
        false,
    )));
    let mut model = FakeModel::default();

    let report = runner()
        .run(&mut runtime, &mut model)
        .expect("validated terminal reobserve completes");

    assert_eq!(report.terminal_stage(), EpisodeStage::Victory);
    assert_eq!(runtime.catalog_calls, 1);
    assert_eq!(runtime.reobserve_calls, 1);
    assert_eq!(runtime.dispatches, 0);
    assert_eq!(model.calls, 0);
    assert!(runtime.released && runtime.mcp_closed && runtime.gateway_closed);
}

#[test]
fn terminal_schema_or_auth_catalog_failures_never_reobserve() {
    for code in ["legal_actions_invalid", "unauthorized"] {
        let mut runtime = FakeRuntime::new(vec![state(EpisodeStage::Combat, 0)]);
        runtime.catalog_errors = vec![PortError::new(code, "terminal", true)];
        let mut model = FakeModel::default();

        assert!(runner().run(&mut runtime, &mut model).is_err());
        assert_eq!(runtime.catalog_calls, 1);
        assert_eq!(runtime.reobserve_calls, 0);
        assert_eq!(runtime.dispatches, 0);
        assert_eq!(model.calls, 0);
        assert!(runtime.released && runtime.mcp_closed && runtime.gateway_closed);
    }
}

#[test]
fn failed_reobserve_is_bounded_at_three_attempts_without_dispatch() {
    let mut runtime = FakeRuntime::new(vec![state(EpisodeStage::Combat, 0)]);
    runtime.catalog_errors = vec![PortError::new("catalog_reobserve", "refresh", true)];
    runtime.reobserve_errors = vec![
        RecoveryError::PortFailure,
        RecoveryError::PortFailure,
        RecoveryError::PortFailure,
    ];
    let mut model = FakeModel::default();

    assert_eq!(
        runner().run(&mut runtime, &mut model),
        Err(EpisodeRunnerError::Recovery(RecoveryError::Exhausted))
    );
    assert_eq!(runtime.catalog_calls, 1);
    assert_eq!(runtime.reobserve_calls, 3);
    assert_eq!(runtime.dispatches, 0);
    assert_eq!(model.calls, 0);
    assert!(runtime.released && runtime.mcp_closed && runtime.gateway_closed);
}
