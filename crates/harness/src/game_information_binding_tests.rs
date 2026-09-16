// SPDX-License-Identifier: MIT

use super::*;

struct ScriptedPort {
    responses: Vec<Value>,
    requests: Vec<LookupBindingRequest>,
}

impl LookupBindingPort for ScriptedPort {
    fn lookup_binding(
        &mut self,
        request: &LookupBindingRequest,
    ) -> Result<Value, LookupBindingError> {
        self.requests.push(request.clone());
        self.responses.pop().ok_or(LookupBindingError::Transport)
    }
}

fn golden(name: &str) -> Result<Value, LookupBindingError> {
    let text = match name {
        "discovery" => include_str!(
            "../../../protocol-artifact/game-information-lookup-binding-v1/golden/discovery-response.json"
        ),
        "required" => include_str!(
            "../../../protocol-artifact/game-information-lookup-binding-v1/golden/reobserve-required-response.json"
        ),
        "reobserved" => include_str!(
            "../../../protocol-artifact/game-information-lookup-binding-v1/golden/reobserved-response.json"
        ),
        "exhausted" => include_str!(
            "../../../protocol-artifact/game-information-lookup-binding-v1/golden/reobserve-exhausted-response.json"
        ),
        _ => return Err(LookupBindingError::Invalid),
    };
    serde_json::from_str(text).map_err(|_| LookupBindingError::Invalid)
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
    })
}

#[test]
fn discovery_then_reobserve_uses_one_binding_and_new_observation_id()
-> Result<(), LookupBindingError> {
    let mut port = ScriptedPort {
        responses: vec![
            golden("reobserved")?,
            golden("required")?,
            golden("discovery")?,
        ],
        requests: Vec::new(),
    };
    let mut session = session();
    let binding = session.discover(&mut port)?;
    assert_eq!(
        binding.binding_id,
        "58fea90991138ea6fb635df1f5eadd08973ec63eba456d135578677ffee61cfc"
    );
    let observation = session.observe(&mut port)?;
    assert_eq!(observation.observation_id, "observation-2");
    assert_eq!(port.requests.len(), 3);
    assert_eq!(
        port.requests[0].operation,
        LookupBindingOperation::Discovery
    );
    assert_eq!(port.requests[1].operation, LookupBindingOperation::Observe);
    assert_eq!(port.requests[2].operation, LookupBindingOperation::Observe);
    Ok(())
}

#[test]
fn observation_before_discovery_and_exhaustion_fail_closed() -> Result<(), LookupBindingError> {
    let mut port = ScriptedPort {
        responses: vec![golden("exhausted")?, golden("discovery")?],
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
fn forged_binding_never_overrides_owner() -> Result<(), LookupBindingError> {
    let mut forged = golden("discovery")?;
    forged["binding"]["binding_id"] =
        json!("0000000000000000000000000000000000000000000000000000000000000000");
    let mut port = ScriptedPort {
        responses: vec![forged],
        requests: Vec::new(),
    };
    assert_eq!(
        session().discover(&mut port),
        Err(LookupBindingError::InvalidIdentity)
    );
    Ok(())
}
