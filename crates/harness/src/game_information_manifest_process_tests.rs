// SPDX-License-Identifier: MIT
//! Actual owned subprocess round trips; all producer/model bytes are original inert fixtures.
use super::*;
use crate::exo_lookup_process::{ExoLookupProcess, ExoLookupProfile};
use crate::{ActionKind, EpisodeLegalAction, EpisodeLegalActionSet, ExoProcessConfig};
use std::time::Duration;

fn process(
    profile: ExoLookupProfile,
    script: &str,
) -> Result<ExoLookupProcess, Box<dyn std::error::Error>> {
    let request: Value = serde_json::from_str(include_str!(
        "../../../protocol-artifact/exo-bridge-v1/golden/request.json"
    ))?;
    let config = ExoProcessConfig::new(
        "/usr/bin/python3",
        vec![
            "-c".into(),
            format!("exec({})", serde_json::to_string(script)?),
        ],
        None,
        vec![],
    )?;
    let timeout = Duration::from_secs(5);
    Ok(match profile {
        ExoLookupProfile::Terminal => ExoLookupProcess::new(
            config,
            "manifest-request".into(),
            "manifest-turn".into(),
            request,
            timeout,
        )?,
        ExoLookupProfile::Bootstrap => ExoLookupProcess::new_bootstrap(
            config,
            "manifest-request".into(),
            "manifest-turn".into(),
            request,
            timeout,
        )?,
        ExoLookupProfile::History => ExoLookupProcess::new_history(
            config,
            "manifest-request".into(),
            "manifest-turn".into(),
            request,
            timeout,
        )?,
        ExoLookupProfile::ContentManifest => ExoLookupProcess::new_content_manifest(
            config,
            "manifest-request".into(),
            "manifest-turn".into(),
            request,
            timeout,
        )?,
    })
}

fn legal() -> Result<EpisodeLegalActionSet, Box<dyn std::error::Error>> {
    let value: Value = serde_json::from_str(include_str!(
        "../../../protocol-artifact/exo-bridge-v1/golden/request.json"
    ))?;
    let action = value["legal_action_ids"][0]
        .as_str()
        .ok_or("fixture action")?;
    Ok(EpisodeLegalActionSet::new(
        value["state_id"].as_str().ok_or("state")?,
        value["generation"].as_u64().ok_or("generation")?,
        vec![EpisodeLegalAction::new(action, ActionKind::EndTurn)?],
    )?)
}

#[test]
fn manifest_subprocess_receives_complete_retention_and_correct_feedback_pin() -> TestResult {
    let (mut session, mut corpus) = manifest_session(7000)?;
    let raw = large_manifest()?;
    let tail = raw.len() - 19;
    let script = format!(
        r#"import json,sys
f=json.loads(input())
assert f['wire_version']=='sts2.exo-lookup-wire-v4-content-manifest'
action=f['payload']['request']['legal_action_ids'][0]
f['sequence']=1
f['payload']={{'kind':'content_manifest','operation_id':'manifest-1'}}
print(json.dumps(f),flush=True)
r=json.loads(input())
assert r['wire_version']==f['wire_version']
data=r['payload']['value']
assert data['manifest_record_ordinal']==0
assert data['content_manifest']['source_sha256']=='{digest}'
assert data['content_manifest']['delivery']=='retained'
f['sequence']=2
f['payload']={{'kind':'read_manifest','record_ordinal':0,'offset':{tail}}}
print(json.dumps(f),flush=True)
r=json.loads(input())
assert r['wire_version']==f['wire_version']
data=r['payload']['value']
assert data['bytes']=='{hex}' and data['next_offset']=={total}
f['sequence']=3
f['wire_version']='sts2.exo-lookup-wire-v1'
f['payload']={{'kind':'decision','action_id':action}}
print(json.dumps(f),flush=True)
"#,
        digest = crate::sha256_hex(&raw),
        hex = crate::hex_bytes(&raw[tail..]),
        total = raw.len()
    );
    let mut agent = process(ExoLookupProfile::ContentManifest, &script)?;
    let mut port = ManifestPort { raw, calls: 0 };
    let legal = legal()?;
    assert_eq!(
        run_lookup_tool_loop(&mut session, &mut corpus, &mut port, &mut agent, &legal, 3)?,
        legal.actions()[0].action_id()
    );
    assert_eq!(port.calls, 1);
    Ok(())
}

#[test]
fn old_profiles_cannot_request_manifest_even_with_the_new_wire_pin() -> TestResult {
    let script = r#"import json
f=json.loads(input())
f['wire_version']='sts2.exo-lookup-wire-v4-content-manifest'
f['sequence']=1
f['payload']={'kind':'content_manifest','operation_id':'manifest-1'}
print(json.dumps(f),flush=True)
input()
"#;
    let (session, _) = manifest_session(7000)?;
    let legal = legal()?;
    for profile in [
        ExoLookupProfile::Terminal,
        ExoLookupProfile::Bootstrap,
        ExoLookupProfile::History,
    ] {
        let mut agent = process(profile, script)?;
        assert!(matches!(
            agent.next_turn(LookupAgentInput {
                binding: &session.binding,
                legal_actions: &legal,
                feedback: &LookupFeedback::Start,
                remaining_turns: 2,
                optional_byte_budget: 7000
            }),
            Err(LookupError::Invalid)
        ));
    }
    Ok(())
}

#[test]
fn bootstrap_and_history_feedback_keep_the_answered_pin_inside_newer_profiles() -> TestResult {
    let bootstrap = json!({"kind":"bootstrap","arguments":{"operation_id":"bootstrap-1",
        "definition_ref":{"content_manifest_id":"synthetic-content-1","entity_kind":"card","namespaced_id":"card-1","variant":null},"instance_ref":null}});
    let history = json!({"kind":"history","arguments":{"operation":"summary","operation_id":"history-1","branch_id":"branch"}});
    for (profile, wire, payload) in [
        (
            ExoLookupProfile::Bootstrap,
            "sts2.exo-lookup-wire-v2-bootstrap",
            &bootstrap,
        ),
        (
            ExoLookupProfile::History,
            "sts2.exo-lookup-wire-v2-bootstrap",
            &bootstrap,
        ),
        (
            ExoLookupProfile::History,
            "sts2.exo-lookup-wire-v3-history",
            &history,
        ),
        (
            ExoLookupProfile::ContentManifest,
            "sts2.exo-lookup-wire-v2-bootstrap",
            &bootstrap,
        ),
        (
            ExoLookupProfile::ContentManifest,
            "sts2.exo-lookup-wire-v3-history",
            &history,
        ),
    ] {
        let script = format!(
            r#"import json
f=json.loads(input())
action=f['payload']['request']['legal_action_ids'][0]
f['sequence']=1
f['wire_version']={wire:?}
f['payload']=json.loads({payload:?})
print(json.dumps(f),flush=True)
r=json.loads(input())
assert r['wire_version']==f['wire_version']
f['sequence']=2
f['wire_version']='sts2.exo-lookup-wire-v1'
f['payload']={{'kind':'decision','action_id':action}}
print(json.dumps(f),flush=True)
"#,
            payload = serde_json::to_string(payload)?
        );
        let mut agent = process(profile, &script)?;
        let (session, _) = manifest_session(7000)?;
        let legal = legal()?;
        agent.next_turn(LookupAgentInput {
            binding: &session.binding,
            legal_actions: &legal,
            feedback: &LookupFeedback::Start,
            remaining_turns: 2,
            optional_byte_budget: 7000,
        })?;
        assert!(matches!(
            agent.next_turn(LookupAgentInput {
                binding: &session.binding,
                legal_actions: &legal,
                feedback: &LookupFeedback::Error(LookupError::MissingCapability),
                remaining_turns: 1,
                optional_byte_budget: 7000
            })?,
            LookupTurn::Decide { .. }
        ));
    }
    Ok(())
}
