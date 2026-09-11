// SPDX-License-Identifier: MIT

use std::sync::Arc;

use serde_json::{Value, json};
use sts2_harness::management::{
    AuthContext, MANAGEMENT_SCHEMA_VERSION, MemoryWorkflowStore, ValidateRequest, synthetic_store,
};

fn actor() -> Result<AuthContext, sts2_harness::management::AuthError> {
    AuthContext::new("operator", ["workflow:*".to_owned()])
}

fn definition() -> Value {
    json!({
        "schema_version": "ascension.workflow/v1",
        "workflow_id": "compiler.identity",
        "version": "1.0.0",
        "mode": "strict",
        "game_profile": "test",
        "policy_ref": "test.policy",
        "capabilities": { "required": [], "optional": [] },
        "limits": {
            "max_steps": 4,
            "max_subworkflow_depth": 1,
            "max_provider_calls": 0,
            "max_parallel_analyses": 1,
            "max_output_tokens": 128
        },
        "entry_graph": "main",
        "graphs": [{
            "id": "main",
            "entry_node": "start",
            "nodes": [{ "id": "start", "kind": "terminal", "config": { "outcome": "completed" } }],
            "edges": []
        }]
    })
}

#[test]
fn compiler_identity_is_reported_separately_from_the_definition_digest()
-> Result<(), Box<dyn std::error::Error>> {
    let service = synthetic_store(Arc::new(MemoryWorkflowStore::new()));
    let actor = actor()?;
    let mut document = definition();
    let validate = |definition: &Value| {
        service.validate(
            &actor,
            ValidateRequest {
                schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
                definition: definition.clone(),
                capabilities: json!({ "capabilities": [] }),
            },
        )
    };
    let first = validate(&document)?;
    assert_eq!(first.compiler, sts2_harness::workflow::WORKFLOW_COMPILER_ID);
    assert!(sts2_harness::workflow::CompilerId::new(&first.compiler).is_ok());
    assert!(!first.definition_digest.is_empty());
    assert_ne!(first.definition_digest, first.compiler);

    document["limits"]["max_steps"] = json!(9);
    let second = validate(&document)?;
    assert_ne!(second.definition_digest, first.definition_digest);
    assert_eq!(second.compiler, first.compiler);
    Ok(())
}
