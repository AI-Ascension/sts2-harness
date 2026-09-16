// SPDX-License-Identifier: MIT

use sts2_harness::context_memory::policy_owner::ActivePolicyBinding;
use sts2_harness::game_information::{LookupAgentInput, LookupAgentPort, LookupError, LookupTurn};

pub(super) struct FencedLookupAgent<'a> {
    pub(super) owner:
        std::sync::Arc<super::super::game_information_owner::RuntimeGameInformationOwner>,
    pub(super) expected: ActivePolicyBinding,
    pub(super) agent: &'a mut dyn LookupAgentPort,
}

impl LookupAgentPort for FencedLookupAgent<'_> {
    fn next_turn(&mut self, input: LookupAgentInput<'_>) -> Result<LookupTurn, LookupError> {
        self.owner
            .call_with_lookup_revalidation(&self.expected, || self.agent.next_turn(input))
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use std::sync::{Arc, mpsc};
    use std::time::Duration;
    use sts2_harness::context_memory::MemoryScope;
    use sts2_harness::game_information::{LookupBinding, LookupFeedback};
    use sts2_harness::{ActionKind, EpisodeLegalAction, EpisodeLegalActionSet};

    struct BlockingAgent {
        started: Option<mpsc::SyncSender<()>>,
        release: mpsc::Receiver<()>,
    }

    impl LookupAgentPort for BlockingAgent {
        fn next_turn(&mut self, _input: LookupAgentInput<'_>) -> Result<LookupTurn, LookupError> {
            self.started
                .take()
                .ok_or(LookupError::Transport)?
                .send(())
                .map_err(|_| LookupError::Transport)?;
            self.release
                .recv_timeout(Duration::from_secs(3))
                .map_err(|_| LookupError::Transport)?;
            Ok(LookupTurn::Decide {
                action_id: String::from("play:card-17"),
            })
        }
    }

    #[test]
    fn blocked_agent_turn_releases_owner_lease_and_discards_stale_decision() {
        let fixture = super::super::super::game_information_owner::owner_management_tests::
            adopted_runtime_owner();
        let owner = Arc::clone(&fixture.owner);
        let expected = fixture.binding.clone();
        let (started_tx, started_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        let agent_call = std::thread::spawn(move || {
            let mut agent = BlockingAgent {
                started: Some(started_tx),
                release: release_rx,
            };
            let mut fenced = FencedLookupAgent {
                owner,
                expected,
                agent: &mut agent,
            };
            let binding = LookupBinding {
                scope: MemoryScope::new("project", "run", "episode", "agent"),
                game_profile: String::from("synthetic-profile"),
                content_manifest_id: String::from("a").repeat(64),
                locale: String::from("en-US"),
                authority_epoch: 1,
                snapshot: None,
            };
            let legal = EpisodeLegalActionSet::new(
                "state-42",
                42,
                vec![
                    EpisodeLegalAction::new("play:card-17", ActionKind::PlayCard)
                        .expect("valid action"),
                ],
            )
            .expect("valid action set");
            fenced.next_turn(LookupAgentInput {
                binding: &binding,
                legal_actions: &legal,
                feedback: &LookupFeedback::Start,
                remaining_turns: 1,
                optional_byte_budget: 0,
            })
        });
        started_rx
            .recv_timeout(Duration::from_secs(3))
            .expect("agent call reached its blocked external boundary");

        let (revoked_tx, revoked_rx) = mpsc::sync_channel(1);
        let authority = Arc::clone(&fixture.authority);
        let revoke = std::thread::spawn(move || {
            let result = authority.update(|state| {
                state.control_epoch = state.control_epoch.saturating_add(1);
                Ok(())
            });
            let _ = revoked_tx.send(result);
        });
        let revocation = revoked_rx.recv_timeout(Duration::from_millis(500));
        let _ = release_tx.send(());
        let agent_result = agent_call.join().expect("agent thread");
        revoke.join().expect("revocation thread");
        assert!(
            revocation.is_ok(),
            "owner revocation must proceed while the agent call is blocked"
        );
        assert!(revocation.expect("revocation completed").is_ok());
        assert!(
            matches!(agent_result, Err(LookupError::Scope)),
            "the returned decision must be discarded after authority changes in flight"
        );
    }
}
