// SPDX-License-Identifier: MIT

use super::*;
use crate::management::{CONTEXT_OWNER_PAUSED, ErrorClass};

/// Inject a typed owner-pause refusal at an exact production session fence. Actual owner gate
/// admission and trusted preview capture are tested against the runtime's encrypted owner fixture.
struct PauseAtFence {
    inner: Arc<dyn LiveContextRenderPort>,
    checks: AtomicUsize,
    pause_at: usize,
}

impl LiveContextRenderPort for PauseAtFence {
    fn render_source_for_decision(
        &self,
        actor: &AuthContext,
        request: &RunRequest,
        digest: &str,
        binding: &RuntimeAuthorityBinding,
        limits: &ContextOwnerControlLimits,
        input: &DecisionInput,
        context_ref: &str,
    ) -> Result<ContextRenderSource, ManagementError> {
        self.inner.render_source_for_decision(
            actor,
            request,
            digest,
            binding,
            limits,
            input,
            context_ref,
        )
    }

    fn assert_render_source_current(
        &self,
        actor: &AuthContext,
        request: &RunRequest,
        digest: &str,
        binding: &RuntimeAuthorityBinding,
        limits: &ContextOwnerControlLimits,
        input: &DecisionInput,
        context_ref: &str,
        expected: &ContextRenderSourceIdentity,
    ) -> Result<(), ManagementError> {
        if self.checks.fetch_add(1, Ordering::SeqCst) + 1 == self.pause_at {
            return Err(ManagementError::conflict(
                CONTEXT_OWNER_PAUSED,
                "owner pause fence",
            ));
        }
        self.inner.assert_render_source_current(
            actor,
            request,
            digest,
            binding,
            limits,
            input,
            context_ref,
            expected,
        )
    }

    fn render_required(&self) -> bool {
        true
    }
}

#[test]
fn context_pause_before_exchange_refuses_cleanly_and_after_exchange_remains_unresolved() {
    for (pause_at, class, writes) in [(1, ErrorClass::Conflict, 0), (2, ErrorClass::Unresolved, 1)]
    {
        let (source, config) = render_fixture();
        let (mut session, _, exchanges, _) =
            render_test_session(source, config, selected_limits(2), Default::default(), None);
        let inner = session
            .context_render
            .take()
            .expect("admitted owner render port");
        let gate = Arc::new(PauseAtFence {
            inner,
            checks: AtomicUsize::new(0),
            pause_at,
        });
        session.context_render = Some(Arc::clone(&gate) as Arc<dyn LiveContextRenderPort>);
        let error = session
            .decide_for(&input(), "decision.live.v1", "context.live.v1")
            .expect_err("pause fences the served decision");
        assert_eq!(error.code, CONTEXT_OWNER_PAUSED);
        assert_eq!(error.class, class);
        assert_eq!(exchanges.load(Ordering::SeqCst), writes);
        assert_eq!(gate.checks.load(Ordering::SeqCst), pause_at);
    }
}
