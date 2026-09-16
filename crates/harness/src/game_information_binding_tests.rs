// SPDX-License-Identifier: MIT

use super::*;
use serde_json::Value;
use serde_json::json;
use std::path::{Path, PathBuf};

struct ScriptedPort {
    responses: Vec<Vec<u8>>,
    requests: Vec<LookupBindingRequest>,
}

impl LookupBindingPort for ScriptedPort {
    fn lookup_binding(
        &mut self,
        request: &LookupBindingRequest,
    ) -> Result<Vec<u8>, LookupBindingError> {
        self.requests.push(request.clone());
        self.responses.pop().ok_or(LookupBindingError::Transport)
    }
}

fn artifact_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../protocol-artifact/game-information-lookup-binding-v1")
}

fn read_json(path: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("read {}: {error}", path.display()))?;
    serde_json::from_str(&text).map_err(|error| format!("parse {}: {error}", path.display()))
}

fn golden(name: &str) -> Result<Value, LookupBindingError> {
    let name = match name {
        "discovery" => "discovery-response.json",
        "observation" => "observation-response.json",
        "required" => "reobserve-required-response.json",
        "reobserved" => "reobserved-response.json",
        "exhausted" => "reobserve-exhausted-response.json",
        "unavailable" => "reobserve-unavailable-response.json",
        _ => return Err(LookupBindingError::Invalid),
    };
    read_json(&artifact_root().join("golden").join(name)).map_err(|_| LookupBindingError::Invalid)
}

fn golden_bytes(name: &str, correlation_id: &str) -> Result<Vec<u8>, LookupBindingError> {
    let mut value = golden(name)?;
    value["correlation_id"] = json!(correlation_id);
    serde_json::to_vec(&value).map_err(|_| LookupBindingError::Invalid)
}

fn session() -> LookupBindingSession {
    LookupBindingSession::new(LookupBindingContext {
        instance_id: "instance-1".to_owned(),
        scope: LookupScope {
            project_id: "proj-1".to_owned(),
            run_id: "run-42".to_owned(),
            episode_id: "episode-7".to_owned(),
            agent_id: "agent-3".to_owned(),
        },
        authority_epoch: 7,
        supported_capabilities: vec![LOOKUP_BINDING_PROFILE.to_owned()],
    })
}

fn vector_session(context: &Value) -> Result<LookupBindingSession, String> {
    let scope: LookupScope = serde_json::from_value(context["scope"].clone())
        .map_err(|error| format!("conformance scope: {error}"))?;
    let supported_capabilities = context["negotiated_capabilities"]
        .as_array()
        .ok_or_else(|| String::from("conformance capability list is missing"))?
        .iter()
        .map(|capability| {
            capability
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| String::from("conformance capability is not a string"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let instance_id = context["instance_id"]
        .as_str()
        .ok_or_else(|| String::from("conformance instance is missing"))?
        .to_owned();
    let authority_epoch = 7;
    let mut session = LookupBindingSession::new(LookupBindingContext {
        instance_id,
        scope,
        authority_epoch,
        supported_capabilities,
    });
    if !context["retained_observation_id"].is_null() {
        session.observation = Some(LookupObservation {
            observation_id: context["retained_observation_id"]
                .as_str()
                .ok_or_else(|| String::from("retained observation id is invalid"))?
                .to_owned(),
            snapshot_id: "snapshot-41".to_owned(),
            state_generation: context["retained_state_generation"]
                .as_u64()
                .ok_or_else(|| String::from("retained generation is invalid"))?,
        });
    }
    Ok(session)
}

fn expected_error(code: &str) -> Result<LookupBindingError, String> {
    Ok(match code {
        "unsupported_version" => LookupBindingError::UnsupportedVersion,
        "invalid_identity" => LookupBindingError::InvalidIdentity,
        "denied_scope" => LookupBindingError::DeniedScope,
        "missing_capability" => LookupBindingError::MissingCapability,
        "mixed_binding" => LookupBindingError::MixedBinding,
        "stale_snapshot" => LookupBindingError::StaleSnapshot,
        "reobserve_unavailable" => LookupBindingError::ReobserveUnavailable,
        "malformed" => LookupBindingError::Invalid,
        _ => return Err(format!("unexpected shared error code {code}")),
    })
}

#[test]
fn discovery_then_retained_reobserve_uses_one_binding_and_new_observation_id()
-> Result<(), LookupBindingError> {
    let mut port = ScriptedPort {
        responses: vec![
            golden_bytes("reobserved", "game-information-binding-observe")?,
            golden_bytes("required", "game-information-binding-observe")?,
            golden_bytes("observation", "game-information-binding-observe")?,
            golden_bytes("discovery", "game-information-binding-discovery")?,
        ],
        requests: Vec::new(),
    };
    let mut session = session();
    let binding = session.discover(&mut port)?;
    assert_eq!(
        binding.binding_id,
        "58fea90991138ea6fb635df1f5eadd08973ec63eba456d135578677ffee61cfc"
    );
    assert_eq!(session.observe(&mut port)?.observation_id, "observation-1");
    assert_eq!(session.observe(&mut port)?.observation_id, "observation-2");
    assert_eq!(port.requests.len(), 4);
    assert_eq!(
        port.requests[0].operation,
        LookupBindingOperation::Discovery
    );
    assert!(
        port.requests[1..]
            .iter()
            .all(|request| request.operation == LookupBindingOperation::Observe)
    );
    assert_eq!(
        port.requests[0].correlation_id,
        "game-information-binding-discovery"
    );
    assert!(
        port.requests[1..]
            .iter()
            .all(|request| request.correlation_id == "game-information-binding-observe")
    );
    Ok(())
}

#[test]
fn discovery_required_and_terminal_states_fail_closed() -> Result<(), LookupBindingError> {
    let mut port = ScriptedPort {
        responses: vec![
            golden_bytes("unavailable", "game-information-binding-observe")?,
            golden_bytes("discovery", "game-information-binding-discovery")?,
        ],
        requests: Vec::new(),
    };
    let mut session = session();
    assert_eq!(
        session.observe(&mut port),
        Err(LookupBindingError::DiscoveryRequired)
    );
    session.discover(&mut port)?;
    assert_eq!(
        session.observe(&mut port),
        Err(LookupBindingError::ReobserveUnavailable)
    );
    assert!(session.observation().is_none());
    Ok(())
}

#[test]
fn forged_binding_and_wrong_response_correlation_are_rejected() -> Result<(), LookupBindingError> {
    let mut forged = golden("discovery")?;
    forged["binding"]["binding_id"] =
        json!("0000000000000000000000000000000000000000000000000000000000000000");
    forged["correlation_id"] = json!("game-information-binding-discovery");
    let forged = serde_json::to_vec(&forged).map_err(|_| LookupBindingError::Invalid)?;
    let mut port = ScriptedPort {
        responses: vec![forged],
        requests: Vec::new(),
    };
    assert_eq!(
        session().discover(&mut port),
        Err(LookupBindingError::InvalidIdentity)
    );

    let bytes = golden_bytes("discovery", "wrong-correlation")?;
    let request_correlation = "game-information-binding-discovery";
    assert_eq!(
        session()
            .decode(&bytes, request_correlation, None)
            .map(|_| ()),
        Err(LookupBindingError::Invalid)
    );
    Ok(())
}

#[path = "game_information_binding_conformance_tests.rs"]
mod conformance_tests;
