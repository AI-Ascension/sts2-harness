// SPDX-License-Identifier: MIT
//! Synthetic subprocess probes exercise the production lookup agent and MCP dispatch seams.
use super::*;
use crate::exo_lookup_process::ExoLookupProcess;
use crate::{ActionKind, EpisodeLegalAction, EpisodeLegalActionSet, ExoProcessConfig};
use std::time::{Duration, Instant};
#[path = "game_information_maximum_process_tests.rs"]
mod maximum;

fn decision_request() -> Value {
    json!({"schema":"sts2.exo-decision-v1","provider_revision":crate::EXO_SOURCE_REVISION,
        "model_execution_id":"execution-1","state_id":"state-42","generation":42,
        "observation":{"state_id":"state-42","generation":42,"visible_seed":null,
            "player":{"hp":10,"max_hp":10,"energy":3,"gold":0,"hand":[],"deck":[],"discard":[],"exhaust":[]},
            "state":{"state":"combat","turn_index":1,"enemies":[]},
            "legal_actions":[{"action_id":"play:card-17","action":{"kind":"play_card","card_id":"card-17","target_id":null}}]},
        "legal_action_ids":["play:card-17"],"objective":"survive","hard_constraints":[],
        "max_response_bytes":8192})
}

fn agent(
    script: &str,
    extra: Vec<String>,
    timeout: Duration,
) -> Result<ExoLookupProcess, LookupError> {
    let mut args = vec![
        "-c".into(),
        format!(
            "exec({})",
            serde_json::to_string(script).map_err(|_| LookupError::Invalid)?
        ),
    ];
    args.extend(extra);
    let config = ExoProcessConfig::new("/usr/bin/python3", args, None, vec![])
        .map_err(|_| LookupError::Invalid)?;
    ExoLookupProcess::new(
        config,
        "request-1".into(),
        "turn-1".into(),
        decision_request(),
        timeout,
    )
}

fn legal() -> Result<EpisodeLegalActionSet, crate::ActionSetError> {
    EpisodeLegalActionSet::new(
        "state-42",
        42,
        vec![EpisodeLegalAction::new(
            "play:card-17",
            ActionKind::PlayCard,
        )?],
    )
}

#[test]
fn duplex_process_consumes_static_and_live_data_before_legal_action() -> TestResult {
    let (mut session, mut corpus) = setup(8192)?;
    negotiate(&mut session)?;
    session.observe_snapshot(request(true, "1")["query"]["binding"]["snapshot_ref"].clone());
    let arguments = [false, true]
        .into_iter()
        .map(|live| {
            let mut query = request(live, "1")["query"].clone();
            query
                .as_object_mut()
                .ok_or(LookupError::Invalid)?
                .remove("binding");
            query
                .as_object_mut()
                .ok_or(LookupError::Invalid)?
                .remove("parent_observation");
            query["target"]
                .as_object_mut()
                .ok_or(LookupError::Invalid)?
                .remove("instance_ref");
            serde_json::to_string(&json!({"operation_id":if live {"live"}else{"static"},
            "mode":if live {"live"}else{"static"},"query":query}))
            .map_err(|_| LookupError::Invalid)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let script = r#"import json,sys
f=json.loads(input())
good=True
for i in range(2):
 f['sequence']=i+1
 f['payload']={'kind':'query','arguments':json.loads(sys.argv[i+1])}
 print(json.dumps(f),flush=True)
 r=json.loads(input())['payload']['value']['data']
 good=good and r['authority']=='untrusted_game_information_data'
 if i==1: good=good and r['result']['page']['items'][0]['fields'][0]['value']==1
f['sequence']=3
f['payload']={'kind':'decision','action_id':'play:card-17' if good else 'cheat'}
print(json.dumps(f),flush=True)
"#;
    let mut agent = agent(script, arguments, Duration::from_secs(3))?;
    let mut mcp = SyntheticMcpPort { id: 20 };
    assert_eq!(
        run_lookup_tool_loop(
            &mut session,
            &mut corpus,
            &mut mcp,
            &mut agent,
            &legal()?,
            3
        )?,
        "play:card-17"
    );
    assert_eq!(session.records.len(), 2);
    assert_eq!(mcp.id, 22);
    let binding = session.binding.clone();
    assert_eq!(binding.scope, corpus.scope().clone());
    Ok(())
}

#[test]
fn duplex_rejects_crash_stall_bounds_correlation_and_forged_action() -> TestResult {
    let scripts = [
        "import sys; sys.exit(1)",
        "import time; input(); time.sleep(10)",
        "input(); print('x'*196609,flush=True)",
        "import json; f=json.loads(input()); f['sequence']=1; f['turn_id']='wrong'; f['payload']={'kind':'decision','action_id':'play:card-17'}; print(json.dumps(f),flush=True)",
        "import json; f=json.loads(input()); f['sequence']=1; f['payload']={'kind':'decision','action_id':'cheat'}; print(json.dumps(f),flush=True)",
        "import json; f=json.loads(input()); f['sequence']=2; f['payload']={'kind':'decision','action_id':'play:card-17'}; print(json.dumps(f),flush=True)",
        "import sys,time; input(); sys.stdout.write('{}'); sys.stdout.flush(); time.sleep(10)",
    ];
    for script in scripts {
        let (session, _) = setup(8192)?;
        let legal = legal()?;
        let start = Instant::now();
        let mut process = agent(script, vec![], Duration::from_millis(150))?;
        assert!(
            process
                .next_turn(LookupAgentInput {
                    binding: &session.binding,
                    legal_actions: &legal,
                    feedback: &LookupFeedback::Start,
                    remaining_turns: 2,
                    optional_byte_budget: session.policy.optional_byte_budget,
                })
                .is_err()
        );
        drop(process);
        assert!(start.elapsed() < Duration::from_secs(2));
    }
    Ok(())
}

#[test]
fn duplex_pins_owner_scope_between_tool_round_trips() -> TestResult {
    let (session, _) = setup(8192)?;
    let legal = legal()?;
    let script = "import json; f=json.loads(input()); f['sequence']=1; f['payload']={'kind':'read_retained','record_ordinal':0,'offset':0}; print(json.dumps(f),flush=True); input()";
    let mut process = agent(script, vec![], Duration::from_secs(2))?;
    assert!(matches!(
        process.next_turn(LookupAgentInput {
            binding: &session.binding,
            legal_actions: &legal,
            feedback: &LookupFeedback::Start,
            remaining_turns: 2,
            optional_byte_budget: session.policy.optional_byte_budget,
        })?,
        LookupTurn::ReadRetained { .. }
    ));
    let mut foreign = session.binding.clone();
    foreign.scope = MemoryScope::new("project-1", "run-1", "episode-1", "other-agent");
    assert_eq!(
        process
            .next_turn(LookupAgentInput {
                binding: &foreign,
                legal_actions: &legal,
                feedback: &LookupFeedback::Error(LookupError::MissingRetention),
                remaining_turns: 1,
                optional_byte_budget: session.policy.optional_byte_budget,
            })
            .err(),
        Some(LookupError::Scope)
    );
    Ok(())
}
