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
fn bootstrap_transcript_replays_without_an_mcp_callback() -> TestResult {
    struct BootstrapAgent {
        sent: bool,
    }
    impl LookupAgentPort for BootstrapAgent {
        fn next_turn(&mut self, input: LookupAgentInput<'_>) -> Result<LookupTurn, LookupError> {
            if !self.sent {
                self.sent = true;
                let definition = serde_json::json!({
                    "content_manifest_id":"synthetic-content-1",
                    "entity_kind":"card","namespaced_id":"synthetic:strike","variant":null
                });
                let request = crate::game_information_binding::game_information_bootstrap::request(
                    "pending",
                    Value::Null,
                    definition,
                    None,
                );
                return Ok(LookupTurn::Bootstrap {
                    operation_id: "bootstrap".to_owned(),
                    request: serde_json::to_vec(&request).map_err(|_| LookupError::Invalid)?,
                });
            }
            if matches!(input.feedback, LookupFeedback::Bootstrap { .. }) {
                return Ok(LookupTurn::Decide {
                    action_id: "play:card-17".to_owned(),
                });
            }
            Err(LookupError::Divergence)
        }
    }

    let (mut session, corpus) = setup(8192)?;
    let instance = serde_json::json!({
        "instance_id":"instance-1","run_id":"run-1","epoch":7,
        "entity_kind":"card","entity_id":"card-17"
    });
    let snapshot = serde_json::json!({
        "snapshot_id":"snapshot-42","instance_ref":instance,"state_generation":42
    });
    let definition = serde_json::json!({
        "content_manifest_id":"synthetic-content-1",
        "entity_kind":"card","namespaced_id":"synthetic:strike","variant":null
    });
    let request = crate::game_information_binding::game_information_bootstrap::request(
        "pending",
        Value::Null,
        definition.clone(),
        None,
    );
    let response = serde_json::json!({
        "protocol_version":crate::game_information_binding::game_information_bootstrap::PROFILE,
        "schema_digest":crate::game_information_binding::game_information_bootstrap::SCHEMA_DIGEST,
        "provenance":{"artifact":"sts2-protocol/game-information-live-observation-bootstrap-v1",
            "source":"schemas/game-information-live-observation-bootstrap-v1.schema.json","generator":"hand-authored"},
        "correlation_id":"pending","kind":"bootstrap_response",
        "scope":{"instance_id":"instance-1","run_id":"run-1","authority_epoch":1,
            "content_manifest_id":"synthetic-content-1","locale":"en"},
        "selector":{"definition_ref":definition,"instance_ref":null},
        "limits":{"max_visible_entities":64,"max_item_bytes":65536,"max_message_bytes":262144},
        "parent_observation":{"instance_ref":instance,"snapshot_ref":snapshot,"state_generation":42},
        "visible_entities":[{"definition_ref":definition,"instance_ref":instance,"snapshot_ref":snapshot}],
        "owner_provenance":{"native_snapshot_owner":"sts2-game-mod","content_manifest_owner":"sts2-game-mod",
            "instance_fence_owner":"sts2-gateway","authority_epoch_owner":"sts2-harness",
            "instance_ref_epoch_owner":"sts2-game-mod","transport_lease_epoch_role":"fence_only"},
        "error":null
    });
    session.install_bootstrap("bootstrap", request, response, snapshot)?;
    let legal = crate::EpisodeLegalActionSet::new(
        "state-42",
        42,
        vec![crate::EpisodeLegalAction::new(
            "play:card-17",
            crate::ActionKind::PlayCard,
        )?],
    )?;
    let mut agent = BootstrapAgent { sent: false };
    assert_eq!(
        run_lookup_replay_tool_loop(&mut session, &corpus, &mut agent, &legal, 2)?,
        "play:card-17"
    );
    Ok(())
}

#[test]
fn agent_bootstrap_uses_mcp_port_and_rejects_mixed_generation() -> TestResult {
    struct BootstrapMcp {
        response: Value,
        calls: usize,
    }
    impl LookupMcpPort for BootstrapMcp {
        fn information_correlation(&self) -> Result<String, LookupError> {
            Ok("1".to_owned())
        }
        fn call_information(
            &mut self,
            _tool: &str,
            _request: &Value,
        ) -> Result<Vec<u8>, LookupError> {
            Err(LookupError::MissingCapability)
        }
        fn call_live_observation_bootstrap(
            &mut self,
            request: &Value,
        ) -> Result<Vec<u8>, LookupError> {
            assert!(request["scope"].is_null());
            self.calls += 1;
            let mut response = self.response.clone();
            response["correlation_id"] = request["correlation_id"].clone();
            serde_json::to_vec(&response).map_err(|_| LookupError::Invalid)
        }
    }
    struct BootstrapAgent {
        sent: bool,
    }
    impl LookupAgentPort for BootstrapAgent {
        fn next_turn(&mut self, input: LookupAgentInput<'_>) -> Result<LookupTurn, LookupError> {
            if !self.sent {
                self.sent = true;
                let definition = json!({
                    "content_manifest_id":"synthetic-content-1",
                    "entity_kind":"card","namespaced_id":"synthetic:strike","variant":null
                });
                let request = crate::game_information_binding::game_information_bootstrap::request(
                    "pending",
                    Value::Null,
                    definition,
                    None,
                );
                return Ok(LookupTurn::Bootstrap {
                    operation_id: "bootstrap".to_owned(),
                    request: serde_json::to_vec(&request).map_err(|_| LookupError::Invalid)?,
                });
            }
            if matches!(input.feedback, LookupFeedback::Bootstrap { .. }) {
                return Ok(LookupTurn::Decide {
                    action_id: "play:card-17".to_owned(),
                });
            }
            Err(LookupError::Divergence)
        }
    }
    let (mut session, mut corpus) = setup(8192)?;
    let instance = json!({
        "instance_id":"instance-1","run_id":"run-1","epoch":7,
        "entity_kind":"card","entity_id":"card-17"
    });
    let snapshot = json!({
        "snapshot_id":"snapshot-42","instance_ref":instance,"state_generation":42
    });
    let definition = json!({
        "content_manifest_id":"synthetic-content-1",
        "entity_kind":"card","namespaced_id":"synthetic:strike","variant":null
    });
    let response = json!({
        "protocol_version":crate::game_information_binding::game_information_bootstrap::PROFILE,
        "schema_digest":crate::game_information_binding::game_information_bootstrap::SCHEMA_DIGEST,
        "provenance":{"artifact":"sts2-protocol/game-information-live-observation-bootstrap-v1",
            "source":"schemas/game-information-live-observation-bootstrap-v1.schema.json","generator":"hand-authored"},
        "correlation_id":"pending","kind":"bootstrap_response",
        "scope":{"instance_id":"instance-1","run_id":"run-1","authority_epoch":1,
            "content_manifest_id":"synthetic-content-1","locale":"en"},
        "selector":{"definition_ref":definition,"instance_ref":null},
        "limits":{"max_visible_entities":64,"max_item_bytes":65536,"max_message_bytes":262144},
        "parent_observation":{"instance_ref":instance,"snapshot_ref":snapshot,"state_generation":42},
        "visible_entities":[{"definition_ref":definition,"instance_ref":instance,"snapshot_ref":snapshot}],
        "owner_provenance":{"native_snapshot_owner":"sts2-game-mod","content_manifest_owner":"sts2-game-mod",
            "instance_fence_owner":"sts2-gateway","authority_epoch_owner":"sts2-harness",
            "instance_ref_epoch_owner":"sts2-game-mod","transport_lease_epoch_role":"fence_only"},
        "error":null
    });
    let legal = crate::EpisodeLegalActionSet::new(
        "state-42",
        42,
        vec![crate::EpisodeLegalAction::new(
            "play:card-17",
            crate::ActionKind::PlayCard,
        )?],
    )?;
    let mut port = BootstrapMcp {
        response: response.clone(),
        calls: 0,
    };
    let mut agent = BootstrapAgent { sent: false };
    assert_eq!(
        run_lookup_tool_loop(&mut session, &mut corpus, &mut port, &mut agent, &legal, 2)?,
        "play:card-17"
    );
    assert_eq!(port.calls, 1);
    let mut stale = response;
    stale["parent_observation"]["state_generation"] = json!(41);
    stale["parent_observation"]["snapshot_ref"]["state_generation"] = json!(41);
    stale["visible_entities"][0]["snapshot_ref"]["state_generation"] = json!(41);
    let mut stale_port = BootstrapMcp {
        response: stale,
        calls: 0,
    };
    let mut stale_session = setup(8192)?.0;
    let mut stale_corpus = setup(8192)?.1;
    let mut stale_agent = BootstrapAgent { sent: false };
    assert_eq!(
        run_lookup_tool_loop(
            &mut stale_session,
            &mut stale_corpus,
            &mut stale_port,
            &mut stale_agent,
            &legal,
            2
        ),
        Err(LookupError::Reobserve)
    );
    assert_eq!(stale_port.calls, 1);
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
