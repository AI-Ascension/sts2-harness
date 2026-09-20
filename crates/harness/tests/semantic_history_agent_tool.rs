// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

//! `sts2-harness#128` requirement 4 at the seams a provider actually reaches.
//!
//! History is served to an agent through one harness-owned port, and requirement 4 says the MCP
//! game adapter must not bypass it to arbitrary artifact storage or reverse-call the harness. The
//! closed vocabulary and the answers are covered inside the crate
//! (`game_information::tests::history_tool_tests`); what is only visible from outside is the
//! additive profile: what it advertises, which frame pin carries one of its turns, and whether a
//! relay built for another profile can answer one anyway.
//!
//! The published surfaces are pinned by value here rather than by "the two profiles differ",
//! because both additive profiles exist to stay byte-compatible with the shipped one. Selecting
//! history has to widen the surface, so every inherited axis is asserted equal to the v1 profile's
//! and every v2 tool is asserted still present.
//!
//! Widening the profile must not widen the pin: each additive turn still travels on the pin its own
//! selector opened. The additivity itself is asserted where it is guaranteed, at the process gate.

use serde_json::{Value, json};
use std::time::Duration;
use sts2_harness::context_memory::MemoryScope;
use sts2_harness::exo_bridge_configuration as config;
use sts2_harness::exo_lookup_process::{ExoLookupProcess, ExoLookupProfile};
use sts2_harness::exo_lookup_wire::{
    EXO_LOOKUP_BOOTSTRAP_WIRE, EXO_LOOKUP_FEEDBACK_BYTES, EXO_LOOKUP_HISTORY_WIRE, EXO_LOOKUP_WIRE,
    ExoLookupFrame, ExoLookupPayload,
};
use sts2_harness::game_information::{
    LookupAgentInput, LookupAgentPort, LookupBinding, LookupError, LookupFeedback, LookupTurn,
};
use sts2_harness::{
    ActionKind, EXO_SOURCE_REVISION, EpisodeLegalAction, EpisodeLegalActionSet, ExoProcessConfig,
    sha256_hex,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// The reviewed deployment the bridge loads, without reading any of its files.
fn loaded() -> config::Loaded {
    config::Loaded {
        config: config::Configuration {
            schema: "sts2.exo-lookup-config-v1".to_owned(),
            executor: "/executor".into(),
            executor_sha256: "0".repeat(64),
            source_root: "/source".into(),
            extension: "/extension".into(),
            extension_sha256: "0".repeat(64),
            node: "/node".into(),
            node_sha256: "0".repeat(64),
            model: "o3-pro".to_owned(),
            endpoint: "http://127.0.0.1:8080".to_owned(),
        },
        digest: "0".repeat(64),
    }
}

#[test]
fn the_history_profile_widens_the_shipped_surfaces_instead_of_replacing_them() -> TestResult {
    let loaded = loaded();
    let terminal = loaded.lookup_description().expect("v1 is advertised");
    let bootstrap = loaded
        .lookup_bootstrap_description()
        .expect("v2 is advertised");
    let history = loaded
        .lookup_history_description()
        .expect("v3 is advertised");

    // Both shipped profiles keep the exact surface they published before history existed.
    assert_eq!(terminal["schema"], json!("sts2.exo-lookup-capability-v1"));
    assert_eq!(terminal["wire_version"], json!(EXO_LOOKUP_WIRE));
    assert_eq!(
        terminal["tools"],
        json!(["sts2_lookup_query", "sts2_lookup_read"])
    );
    assert_eq!(
        terminal["tool_digest"],
        json!(sha256_hex(b"sts2_lookup_query\nsts2_lookup_read\n"))
    );
    assert!(terminal.get("profile").is_none());
    assert_eq!(
        bootstrap["schema"],
        json!("sts2.exo-lookup-capability-v2-bootstrap")
    );
    assert_eq!(bootstrap["wire_version"], json!(EXO_LOOKUP_BOOTSTRAP_WIRE));
    assert_eq!(bootstrap["profile"], json!("bootstrap"));
    assert_eq!(
        bootstrap["tools"],
        json!([
            "sts2_lookup_query",
            "sts2_lookup_read",
            "sts2_lookup_bootstrap"
        ])
    );
    assert_eq!(
        bootstrap["tool_digest"],
        json!(sha256_hex(
            b"sts2_lookup_query\nsts2_lookup_read\nsts2_lookup_bootstrap\n"
        ))
    );

    // History is a third pin with its own schema, tool set and digest.
    assert_eq!(
        history["schema"],
        json!("sts2.exo-lookup-capability-v3-history")
    );
    assert_eq!(history["wire_version"], json!(EXO_LOOKUP_HISTORY_WIRE));
    assert_eq!(history["profile"], json!("history"));
    assert_eq!(
        history["tools"],
        json!([
            "sts2_lookup_query",
            "sts2_lookup_read",
            "sts2_lookup_bootstrap",
            "sts2_lookup_history"
        ])
    );
    assert_eq!(
        history["tool_digest"],
        json!(sha256_hex(
            b"sts2_lookup_query\nsts2_lookup_read\nsts2_lookup_bootstrap\nsts2_lookup_history\n"
        ))
    );
    let tools = history["tools"]
        .as_array()
        .ok_or("the history profile advertises a tool list")?;
    let shipped = bootstrap["tools"]
        .as_array()
        .ok_or("the bootstrap profile advertises a tool list")?;
    assert_eq!(tools.len(), shipped.len() + 1);
    for tool in shipped {
        assert!(
            tools.contains(tool),
            "selecting history withdrew the shipped tool {tool}"
        );
    }
    // It adds a tool rather than an authority: every axis it inherits is the one v1 published.
    for inherited in [
        "decisions",
        "decision_support",
        "profiles",
        "profile_support",
        "unsupported_profile_code",
        "max_tool_round_trips",
        "max_model_writes",
    ] {
        assert_eq!(
            bootstrap[inherited], terminal[inherited],
            "the bootstrap profile changed {inherited}"
        );
        assert_eq!(
            history[inherited], terminal[inherited],
            "the history profile changed {inherited}"
        );
    }
    Ok(())
}

/// A frame carrying `payload` on `wire`, as a relay would send it back.
fn frame(wire: &str, payload: Value) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&json!({
        "wire_version": wire,
        "request_id": "request-1",
        "turn_id": "turn-1",
        "sequence": 1,
        "payload": payload
    }))
}

/// One bounded history question, as the relay's `sts2_lookup_history` tool receives it.
fn history_turn() -> Value {
    json!({"kind":"history","arguments":{
        "operation":"summary","operation_id":"history_1","branch_id":"branch_root"
    }})
}

/// One bootstrap turn, as the shipped profile's `sts2_lookup_bootstrap` tool receives it.
fn bootstrap_turn() -> Value {
    json!({"kind":"bootstrap","arguments":{"operation_id":"bootstrap_1","definition_ref":{
        "content_manifest_id":"content-1","entity_kind":"card",
        "namespaced_id":"ironclad:strike","variant":null
    },"instance_ref":null}})
}

#[test]
fn a_history_turn_only_rides_the_pin_that_profile_opened() -> TestResult {
    assert_eq!(ExoLookupProfile::Terminal.wire(), EXO_LOOKUP_WIRE);
    assert_eq!(
        ExoLookupProfile::Bootstrap.wire(),
        EXO_LOOKUP_BOOTSTRAP_WIRE
    );
    assert_eq!(ExoLookupProfile::History.wire(), EXO_LOOKUP_HISTORY_WIRE);

    // A relay holding one of the older pins cannot relabel a frame into a history question.
    for wire in [EXO_LOOKUP_WIRE, EXO_LOOKUP_BOOTSTRAP_WIRE] {
        assert_eq!(
            ExoLookupFrame::parse(&frame(wire, history_turn())?).err(),
            Some(LookupError::Invalid),
            "a history turn was admitted on {wire}"
        );
    }
    // The same payload is well formed on its own pin, so the refusals above were about the pin.
    let admitted = ExoLookupFrame::parse(&frame(EXO_LOOKUP_HISTORY_WIRE, history_turn())?)?;
    assert!(matches!(admitted.payload, ExoLookupPayload::History { .. }));
    // The rule is symmetric, so the pin a profile added is not a pin the shipped turn may borrow.
    assert_eq!(
        ExoLookupFrame::parse(&frame(EXO_LOOKUP_HISTORY_WIRE, bootstrap_turn())?).err(),
        Some(LookupError::Invalid)
    );
    let shipped = ExoLookupFrame::parse(&frame(EXO_LOOKUP_BOOTSTRAP_WIRE, bootstrap_turn())?)?;
    assert!(matches!(
        shipped.payload,
        ExoLookupPayload::Bootstrap { .. }
    ));
    Ok(())
}

fn decision_request() -> Value {
    json!({"schema":"sts2.exo-decision-v1","provider_revision":EXO_SOURCE_REVISION,
        "model_execution_id":"execution-1","state_id":"state-42","generation":42,
        "observation":{"state_id":"state-42","generation":42,"visible_seed":null,
            "player":{"hp":10,"max_hp":10,"energy":3,"gold":0,"hand":[],"deck":[],"discard":[],"exhaust":[]},
            "state":{"state":"combat","turn_index":1,"enemies":[]},
            "legal_actions":[{"action_id":"play:card-17","action":{"kind":"play_card","card_id":"card-17","target_id":null}}]},
        "legal_action_ids":["play:card-17"],"objective":"survive","hard_constraints":[],
        "max_response_bytes":8192})
}

/// A relay that answers the frame it was sent with the pin and turn the scenario chose.
const RELAY: &str = "import json,sys\nf=json.loads(input())\nf['wire_version']=sys.argv[1]\nf['payload']=json.loads(sys.argv[2])\nf['sequence']=f['sequence']+1\nprint(json.dumps(f),flush=True)\n";

/// One relay turn under an explicitly selected profile, and the turn it was admitted as.
///
/// Every profile is constructed through the constructor it ships with, so a relay built for one
/// profile can only answer what that constructor selected.
fn one_turn(
    profile: ExoLookupProfile,
    wire: &str,
    payload: &Value,
) -> Result<Result<LookupTurn, LookupError>, &'static str> {
    let config = ExoProcessConfig::new(
        "/usr/bin/python3",
        vec![
            "-c".to_owned(),
            format!(
                "exec({})",
                serde_json::to_string(RELAY).map_err(|_| "exo_lookup_payload")?
            ),
            wire.to_owned(),
            serde_json::to_string(payload).map_err(|_| "exo_lookup_payload")?,
        ],
        None,
        vec![],
    )
    .map_err(|_| "exo_lookup_config")?;
    let legal = EpisodeLegalActionSet::new(
        "state-42",
        42,
        vec![
            EpisodeLegalAction::new("play:card-17", ActionKind::PlayCard)
                .map_err(|_| "exo_lookup_legal")?,
        ],
    )
    .map_err(|_| "exo_lookup_legal")?;
    let binding = LookupBinding {
        scope: MemoryScope::new("project-1", "run-1", "episode-1", "agent-1"),
        game_profile: "fair-play-v1".to_owned(),
        content_manifest_id: "synthetic-content-1".to_owned(),
        locale: "en".to_owned(),
        authority_epoch: 1,
        snapshot: None,
    };
    let request_id = "request-1".to_owned();
    let turn_id = "turn-1".to_owned();
    let timeout = Duration::from_secs(5);
    let mut process = match profile {
        ExoLookupProfile::Terminal => {
            ExoLookupProcess::new(config, request_id, turn_id, decision_request(), timeout)
        }
        ExoLookupProfile::Bootstrap => ExoLookupProcess::new_bootstrap(
            config,
            request_id,
            turn_id,
            decision_request(),
            timeout,
        ),
        ExoLookupProfile::History => {
            ExoLookupProcess::new_history(config, request_id, turn_id, decision_request(), timeout)
        }
    }
    .map_err(|_| "exo_lookup_process")?;
    Ok(process.next_turn(LookupAgentInput {
        binding: &binding,
        legal_actions: &legal,
        feedback: &LookupFeedback::Start,
        remaining_turns: 3,
        optional_byte_budget: EXO_LOOKUP_FEEDBACK_BYTES,
    }))
}

#[test]
fn a_relay_cannot_answer_a_history_turn_its_caller_did_not_select() -> TestResult {
    // A bootstrap turn is what both additive profiles admit, so this is the control that the probe
    // itself works: without it the refusals below would pass on a relay that answers nothing.
    for profile in [ExoLookupProfile::Bootstrap, ExoLookupProfile::History] {
        assert!(
            matches!(
                one_turn(profile, EXO_LOOKUP_BOOTSTRAP_WIRE, &bootstrap_turn())?,
                Ok(LookupTurn::Bootstrap { .. })
            ),
            "the {profile:?} profile did not answer the shipped bootstrap turn"
        );
    }
    // Only the profile that selected history answers one, whatever pin the relay claims.
    for profile in [ExoLookupProfile::Terminal, ExoLookupProfile::Bootstrap] {
        assert_eq!(
            one_turn(profile, EXO_LOOKUP_HISTORY_WIRE, &history_turn())?.err(),
            Some(LookupError::Invalid),
            "a relay selected for {profile:?} answered a history turn"
        );
    }
    match one_turn(
        ExoLookupProfile::History,
        EXO_LOOKUP_HISTORY_WIRE,
        &history_turn(),
    )? {
        Ok(LookupTurn::History {
            operation_id,
            request,
        }) => {
            assert_eq!(operation_id, "history_1");
            // The question travels as the boundary's own canonical request, not as the raw tool
            // arguments, so a relay cannot smuggle a second vocabulary across this seam.
            let request: Value = serde_json::from_slice(&request)?;
            assert_eq!(
                request["profile"],
                json!("ascension.semantic-history-agent-tool.v1")
            );
            assert_eq!(request["ask"]["ask"], json!("summary"));
            assert_eq!(request["ask"]["branch_id"], json!("branch_root"));
            // The boundary writes the request, so the tool arguments do not survive as free-form
            // data: the only keys present are the ones the canonical vocabulary declares.
            let ask = request["ask"]
                .as_object()
                .ok_or("the canonical request carries an ask object")?;
            assert_eq!(ask.len(), 2, "the ask carried an undeclared field: {ask:?}");
        }
        _ => return Err("a history turn the selected profile admitted was not served".into()),
    }
    Ok(())
}
