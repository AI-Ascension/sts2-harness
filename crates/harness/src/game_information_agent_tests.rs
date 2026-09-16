// SPDX-License-Identifier: MIT
use super::*;

struct ScriptedAgent {
    turn: usize,
    saw_static: bool,
    saw_live_cost: bool,
}
impl LookupAgentPort for ScriptedAgent {
    fn next_turn(&mut self, input: LookupAgentInput<'_>) -> Result<LookupTurn, LookupError> {
        if let LookupFeedback::Data { delivery, .. } = input.feedback {
            assert_eq!(
                delivery.data["authority"],
                "untrusted_game_information_data"
            );
            if self.turn == 1 {
                self.saw_static = true;
            }
            if self.turn == 2 {
                self.saw_live_cost =
                    delivery.data["result"]["page"]["items"][0]["fields"][0]["value"] == 1;
            }
        }
        let turn = match self.turn {
            0 => LookupTurn::Query {
                operation_id: "agent-static".to_owned(),
                request: serde_json::to_vec(&request(false, "1"))
                    .map_err(|_| LookupError::Invalid)?,
            },
            1 => LookupTurn::Query {
                operation_id: "agent-live".to_owned(),
                request: serde_json::to_vec(&request(true, "1"))
                    .map_err(|_| LookupError::Invalid)?,
            },
            _ => LookupTurn::Decide {
                action_id: if self.saw_static && self.saw_live_cost {
                    "play:card-17"
                } else {
                    "cheat-action"
                }
                .to_owned(),
            },
        };
        self.turn += 1;
        Ok(turn)
    }
}

#[test]
fn supported_bounded_agent_loop_queries_both_families_before_deciding() -> TestResult {
    let (mut session, mut corpus) = setup(8192)?;
    negotiate(&mut session)?;
    session.observe_snapshot(request(true, "1")["query"]["binding"]["snapshot_ref"].clone());
    let mut mcp = SyntheticMcpPort { id: 20 };
    let mut agent = ScriptedAgent {
        turn: 0,
        saw_static: false,
        saw_live_cost: false,
    };
    let legal = crate::EpisodeLegalActionSet::new(
        "state-42",
        42,
        vec![crate::EpisodeLegalAction::new(
            "play:card-17",
            crate::ActionKind::PlayCard,
        )?],
    )?;
    assert_eq!(
        run_lookup_tool_loop(&mut session, &mut corpus, &mut mcp, &mut agent, &legal, 3)?,
        "play:card-17"
    );
    assert_eq!(session.records.len(), 2);
    assert_eq!(mcp.id, 22);
    session.replay_cursor = 0;
    let mut replay_agent = ScriptedAgent {
        turn: 0,
        saw_static: false,
        saw_live_cost: false,
    };
    assert_eq!(
        run_lookup_replay_tool_loop(&mut session, &corpus, &mut replay_agent, &legal, 3)?,
        "play:card-17"
    );
    assert_eq!(session.replay_cursor, 2);
    Ok(())
}

#[test]
fn replay_rejects_a_different_query_before_returning_a_decision() -> TestResult {
    struct ChangedQueryAgent {
        turns: usize,
    }
    impl LookupAgentPort for ChangedQueryAgent {
        fn next_turn(&mut self, _input: LookupAgentInput<'_>) -> Result<LookupTurn, LookupError> {
            self.turns += 1;
            Ok(if self.turns == 1 {
                let mut changed = request(false, "untrusted-correlation");
                changed["query"]["filters"]["display_name"] =
                    serde_json::json!("different valid search");
                LookupTurn::Query {
                    operation_id: "agent-static".to_owned(),
                    request: serde_json::to_vec(&changed).map_err(|_| LookupError::Invalid)?,
                }
            } else {
                LookupTurn::Decide {
                    action_id: "play:card-17".to_owned(),
                }
            })
        }
    }

    let (mut session, mut corpus) = setup(8192)?;
    negotiate(&mut session)?;
    session.observe_snapshot(request(true, "snapshot")["query"]["binding"]["snapshot_ref"].clone());
    let mut mcp = SyntheticMcpPort { id: 20 };
    let legal = crate::EpisodeLegalActionSet::new(
        "state-42",
        42,
        vec![crate::EpisodeLegalAction::new(
            "play:card-17",
            crate::ActionKind::PlayCard,
        )?],
    )?;
    let mut live_agent = ScriptedAgent {
        turn: 0,
        saw_static: false,
        saw_live_cost: false,
    };
    run_lookup_tool_loop(
        &mut session,
        &mut corpus,
        &mut mcp,
        &mut live_agent,
        &legal,
        3,
    )?;
    session.replay_cursor = 0;
    let mut changed = ChangedQueryAgent { turns: 0 };
    assert_eq!(
        run_lookup_replay_tool_loop(&mut session, &corpus, &mut changed, &legal, 2),
        Err(LookupError::Divergence)
    );
    assert_eq!(
        changed.turns, 1,
        "mismatched replay must fail before action"
    );
    Ok(())
}

#[test]
fn action_injection_and_loop_exhaustion_cannot_escape_legal_set() -> TestResult {
    let (mut session, mut corpus) = setup(8192)?;
    negotiate(&mut session)?;
    let legal = crate::EpisodeLegalActionSet::new("state-42", 42, vec![])?;
    let mut mcp = SyntheticMcpPort { id: 1 };
    let mut agent = ScriptedAgent {
        turn: 2,
        saw_static: false,
        saw_live_cost: false,
    };
    assert_eq!(
        run_lookup_tool_loop(&mut session, &mut corpus, &mut mcp, &mut agent, &legal, 1),
        Err(LookupError::Invalid)
    );
    let mut agent = ScriptedAgent {
        turn: 0,
        saw_static: false,
        saw_live_cost: false,
    };
    assert_eq!(
        run_lookup_tool_loop(&mut session, &mut corpus, &mut mcp, &mut agent, &legal, 1),
        Err(LookupError::Bounds)
    );
    let mut old_snapshot = request(true, "1")["query"]["binding"]["snapshot_ref"].clone();
    old_snapshot["state_generation"] = json!(41);
    session.observe_snapshot(old_snapshot);
    assert_eq!(
        run_lookup_tool_loop(&mut session, &mut corpus, &mut mcp, &mut agent, &legal, 1),
        Err(LookupError::Reobserve)
    );
    Ok(())
}
