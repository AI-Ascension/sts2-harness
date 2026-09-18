// SPDX-License-Identifier: MIT

use executor::{
    AgentHarnessKind, BasicToolRuntime, CreateAgentRequest, CreateConversationRequest, Harness,
    SendRequest, SendResult, TypeScriptHarness, TypeScriptHarnessConfig,
};
use exoharness::{
    BasicExoHarness, BasicExoHarnessConfig, Binding, Event, EventQuery, EventQueryDirection,
    ExoHarness, PutSecretRequest, SandboxBackendRegistration, SandboxProvider, Secret,
    SecretBackendChoice,
};
use lingua::Message;
use lingua::universal::UserContent;
use std::sync::Arc;

pub(super) use crate::turn_evidence::assistant_text;
use crate::turn_evidence::{forbidden_tool_denials, guard_evidence, terminal};
use crate::{Invocation, Receipt};

async fn create_harness(
    invocation: &mut Invocation,
) -> Result<TypeScriptHarness<BasicToolRuntime>, &'static str> {
    let mut encryption_key = [0_u8; 32];
    getrandom::fill(&mut encryption_key).map_err(|_| "exo_executor_entropy")?;
    let root = Arc::new(
        BasicExoHarness::new(BasicExoHarnessConfig {
            root: invocation.state_root.clone(),
            secret_backend: SecretBackendChoice::Static(encryption_key),
            sandbox_default: SandboxProvider::LocalProcess,
            // Required by upstream's configuration validator; no model tool is registered.
            sandbox_backends: vec![SandboxBackendRegistration::local_process()],
        })
        .await
        .map_err(|_| "exo_executor_state")?,
    );
    bind_model(
        root.as_ref(),
        &invocation.model,
        &invocation.endpoint,
        std::mem::take(&mut invocation.credential),
    )
    .await
    .map_err(|_| "exo_executor_binding")?;
    Ok(TypeScriptHarness::new(
        root as Arc<dyn ExoHarness>,
        invocation.source_root.clone(),
        Arc::new(BasicToolRuntime),
    ))
}

pub(super) async fn execute(mut invocation: Invocation) -> Result<Receipt, &'static str> {
    let harness = create_harness(&mut invocation).await?;
    let agent = harness
        .create_agent(
            agent_request(
                &invocation.module_path,
                &invocation.model,
                invocation.max_output_tokens,
            )
            .map_err(|_| "exo_executor_module")?,
        )
        .await
        .map_err(|_| "exo_executor_agent")?;
    let conversation = agent
        .create_conversation(CreateConversationRequest::default())
        .await
        .map_err(|_| "exo_executor_conversation")?;
    let agent_id = agent.record().id.to_string();
    let conversation_id = conversation.record().id.to_string();
    let result = conversation
        .send(SendRequest {
            input: vec![Message::User {
                content: UserContent::String(
                    serde_json::to_string(&invocation.input)
                        .map_err(|_| "exo_executor_projection")?,
                ),
            }],
            session_id: None,
        })
        .await;
    let events = conversation
        .exoharness_handle()
        .get_events(Some(EventQuery {
            direction: Some(EventQueryDirection::Asc),
            limit: Some(65),
            ..EventQuery::default()
        }))
        .await
        .map_err(|_| "exo_executor_events")?;
    receipt(
        invocation,
        &agent_id,
        &conversation_id,
        &events.events,
        result.ok(),
    )
}

pub(super) fn receipt(
    invocation: Invocation,
    agent_id: &str,
    conversation_id: &str,
    events: &[Event],
    result: Option<SendResult>,
) -> Result<Receipt, &'static str> {
    let (event, attempts, forwarded, denied) = guard_evidence(events)?;
    let forbidden = forbidden_tool_denials(events)?;
    let turn_id = event.turn_id.ok_or("exo_executor_turn_identity")?;
    let session_id = event.session_id.ok_or("exo_executor_session_identity")?;
    let event_cursor = event.id.to_string();
    // A denied tool registration or dispatch is never a completed turn, whatever else happened.
    let decision = if forbidden == 0
        && result
            .as_ref()
            .is_some_and(|result| result.turn_id == turn_id && result.session_id == session_id)
        && attempts == 1
        && forwarded == 1
        && denied == 0
    {
        terminal(events.iter().map(|event| event.data.clone())).ok()
    } else {
        None
    };
    let error_code = if forbidden != 0 {
        Some("exo_forbidden_tool")
    } else {
        decision.is_none().then_some("exo_turn_failed")
    };
    Ok(Receipt {
        version: "sts2.exo-executor-receipt-v2",
        request_id: invocation.request_id,
        host_turn_id: invocation.host_turn_id,
        exo_agent_id: agent_id.to_owned(),
        exo_conversation_id: conversation_id.to_owned(),
        exo_turn_id: turn_id.to_string(),
        exo_session_id: session_id.to_string(),
        event_cursor,
        error_code,
        decision,
        fetch_attempts: attempts,
        forwarded_requests: forwarded,
        denied_requests: denied,
    })
}

pub(super) async fn bind_model(
    root: &dyn ExoHarness,
    model: &str,
    endpoint: &str,
    secret: String,
) -> Result<(), ()> {
    if secret.is_empty() {
        return Err(());
    }
    let secret_id = root
        .put_secret(PutSecretRequest {
            name: String::from("sts2-model"),
            secret: Secret::Key { value: secret },
        })
        .await
        .map_err(|_| ())?;
    root.put_binding(Binding::Llm {
        name: model.to_owned(),
        model: model.to_owned(),
        base_url: Some(endpoint.to_owned()),
        secret_id: Some(secret_id),
    })
    .await
    .map(|_| ())
    .map_err(|_| ())
}

pub(super) fn agent_request(
    module: &std::path::Path,
    model: &str,
    tokens: u32,
) -> Result<CreateAgentRequest, ()> {
    Ok(CreateAgentRequest {
        slug: String::from("sts2"),
        name: None,
        harness: AgentHarnessKind::TypeScript,
        typescript: Some(TypeScriptHarnessConfig {
            module_path: module.to_str().ok_or(())?.to_owned(),
            tool_module_paths: Vec::new(),
        }),
        enable_agent_tool_creation: false,
        sandbox_image: None,
        sandbox_provider: SandboxProvider::LocalProcess,
        sandbox_scope: None,
        enable_networking: false,
        model: model.to_owned(),
        max_output_tokens: Some(i64::from(tokens)),
        max_tool_round_trips: Some(0),
        braintrust: None,
    })
}
