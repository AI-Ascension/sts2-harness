// SPDX-License-Identifier: MIT

use super::*;
use sts2_harness::HarnessError;

struct MismatchedRouter {
    inner: FakeRouter,
    fail_cleanup: bool,
}

impl InstanceRouter for MismatchedRouter {
    fn bind(&mut self, request: &RouteRequest) -> Result<RouteBinding, PortError> {
        let other = RunId::new(request.run_id().get() + 1)
            .ok_or_else(|| PortError::new("invalid_test_id", "invalid synthetic ID", false))?;
        self.inner
            .bind(&RouteRequest::new(other, request.episode_id(), None))
    }

    fn unbind(&mut self, binding: &RouteBinding) -> Result<(), PortError> {
        self.inner.unbind(binding)?;
        if self.fail_cleanup {
            Err(PortError::new(
                "cleanup_failed",
                "synthetic cleanup failure",
                false,
            ))
        } else {
            Ok(())
        }
    }

    fn close(&mut self) -> Result<(), PortError> {
        self.inner.close()
    }
}

#[test]
fn mismatched_binding_is_released_and_cleanup_failure_is_visible() -> Result<(), Box<dyn Error>> {
    for fail_cleanup in [false, true] {
        let router = MismatchedRouter {
            inner: FakeRouter::new(),
            fail_cleanup,
        };
        let mut harness = Harness::new(
            router,
            FakeProvider::with_failures(0),
            FakeStorage::new(),
            FakeArtifacts::new(),
            FakeReplay::new(),
        );
        let run = harness.start_run()?;
        let result = harness.start_episode(run, None);
        if fail_cleanup {
            assert!(
                matches!(result, Err(HarnessError::Routing(error)) if error.code() == "cleanup_failed")
            );
        } else {
            assert!(matches!(result, Err(HarnessError::Invalid(_))));
        }
        let report = harness.close()?;
        assert_eq!(report.unbound_episodes, 0);
        let parts = harness.into_parts();
        assert_eq!(parts.router.inner.bindings, parts.router.inner.unbindings);
        assert_eq!(parts.router.inner.unbindings.len(), 1);
    }
    Ok(())
}
