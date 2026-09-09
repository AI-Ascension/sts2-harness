// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn terminal_completion_runs_once_before_any_cleanup() {
    let mut runtime = FakeRuntime::new(complete_states());
    let mut model = FakeModel::default();
    let mut completion_count = 0;
    let report = runner()
        .run_with_completion(&mut runtime, &mut model, |port, _, report| {
            assert!(!port.released && !port.mcp_closed && !port.gateway_closed);
            assert_eq!(report.terminal_stage(), EpisodeStage::Victory);
            completion_count += 1;
            Ok(())
        })
        .expect("terminal report should complete before cleanup");
    assert_eq!(completion_count, 1);
    assert_eq!(report.terminal_stage(), EpisodeStage::Victory);
    assert!(runtime.released && runtime.mcp_closed && runtime.gateway_closed);
}

#[test]
fn failed_episode_never_invokes_terminal_completion() {
    let mut runtime = FakeRuntime::new(complete_states());
    let mut model = FakeModel {
        unavailable: true,
        ..FakeModel::default()
    };
    let mut completed = false;
    let result = runner().run_with_completion(&mut runtime, &mut model, |_, _, _| {
        completed = true;
        Ok(())
    });
    assert!(matches!(result, Err(EpisodeRunnerError::Policy(_))));
    assert!(!completed);
    assert!(runtime.released && runtime.mcp_closed && runtime.gateway_closed);
}

#[test]
fn terminal_persistence_failure_still_runs_all_cleanup() {
    for fail_release in [false, true] {
        let mut runtime = FakeRuntime::new(complete_states());
        runtime.fail_release = fail_release;
        let mut model = FakeModel::default();
        let error = runner()
            .run_with_completion(&mut runtime, &mut model, |port, _, _| {
                assert!(!port.released && !port.mcp_closed && !port.gateway_closed);
                Err(PortError::new(
                    "synthetic_storage_failure",
                    "completion failed",
                    false,
                ))
            })
            .expect_err("completion must not succeed after persistence failure");
        if fail_release {
            assert!(matches!(
                error,
                EpisodeRunnerError::FailureWithCleanup { failure, cleanup }
                    if matches!(*failure, EpisodeRunnerError::Completion(_))
                        && cleanup == ShutdownError::ReleaseFailed
            ));
        } else {
            assert!(matches!(error, EpisodeRunnerError::Completion(_)));
        }
        assert!(runtime.released && runtime.mcp_closed && runtime.gateway_closed);
    }
}

#[test]
fn cleanup_failure_is_reported_after_completion_has_committed() {
    let mut runtime = FakeRuntime::new(complete_states());
    runtime.fail_release = true;
    let mut model = FakeModel::default();
    let mut completed = false;
    let result = runner().run_with_completion(&mut runtime, &mut model, |port, _, _| {
        assert!(!port.released && !port.mcp_closed && !port.gateway_closed);
        completed = true;
        Ok(())
    });
    assert!(completed);
    assert_eq!(
        result,
        Err(EpisodeRunnerError::Shutdown(ShutdownError::ReleaseFailed))
    );
    assert!(runtime.released && runtime.mcp_closed && runtime.gateway_closed);
}

#[test]
fn episode_failure_and_cleanup_failure_preserve_both_causes() {
    let mut runtime = FakeRuntime::new(complete_states());
    runtime.fail_release = true;
    let mut model = FakeModel {
        unavailable: true,
        ..FakeModel::default()
    };
    let mut completion_count = 0;
    let result = runner().run_with_completion(&mut runtime, &mut model, |_, _, _| {
        completion_count += 1;
        Ok(())
    });
    assert!(matches!(
        result,
        Err(EpisodeRunnerError::FailureWithCleanup { failure, cleanup })
            if matches!(*failure, EpisodeRunnerError::Policy(_))
                && cleanup == ShutdownError::ReleaseFailed
    ));
    assert_eq!(completion_count, 0);
    assert!(runtime.released && runtime.mcp_closed && runtime.gateway_closed);
}
