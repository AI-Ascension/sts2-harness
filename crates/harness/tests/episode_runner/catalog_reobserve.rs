// SPDX-License-Identifier: MIT

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
fn persistent_or_untyped_catalog_failure_stops_with_owned_cleanup() {
    for (code, retryable, calls) in [
        ("catalog_reobserve", true, 4),
        ("catalog_reobserve", false, 1),
        ("legal_actions_failed", true, 1),
    ] {
        let mut runtime = FakeRuntime::new(vec![state(EpisodeStage::Combat, 0)]);
        runtime.catalog_errors = (0..4)
            .map(|_| PortError::new(code, "test", retryable))
            .collect();
        let mut model = FakeModel::default();
        assert!(runner().run(&mut runtime, &mut model).is_err());
        assert_eq!(runtime.catalog_calls, calls);
        assert_eq!(runtime.dispatches, 0);
        assert_eq!(model.calls, 0);
        assert!(runtime.released && runtime.mcp_closed && runtime.gateway_closed);
    }
}
