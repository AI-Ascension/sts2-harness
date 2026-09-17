// SPDX-License-Identifier: MIT
use super::*;
use std::path::Path;

fn golden(name: &str) -> Result<Value, String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../protocol-artifact/game-information-live-observation-bootstrap-v1/golden");
    let bytes = std::fs::read(root.join(name)).map_err(|error| error.to_string())?;
    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}

fn scope() -> Value {
    json!({
        "instance_id":"instance-1","run_id":"run-42","authority_epoch":7,
        "content_manifest_id":"content-1","locale":"en-US"
    })
}

fn definition() -> Value {
    json!({"content_manifest_id":"content-1","entity_kind":"card",
        "namespaced_id":"ironclad:strike","variant":null})
}

fn request(instance_ref: Option<Value>) -> Value {
    super::request("corr-bootstrap-1", scope(), definition(), instance_ref)
}

#[test]
fn unique_occurrence_is_admitted_and_ambiguous_wildcard_is_refused() -> Result<(), String> {
    let response = golden("bootstrap-response.json")?;
    assert_eq!(
        select_snapshot(&request(None), &response),
        Err(BootstrapError::Ambiguous)
    );
    let occurrence = json!({"instance_id":"instance-1","run_id":"run-42","epoch":7,
        "entity_kind":"card","entity_id":"card-17"});
    let mut selected_response = response;
    selected_response["selector"]["instance_ref"] = occurrence.clone();
    let snapshot = select_snapshot(&request(Some(occurrence)), &selected_response)
        .map_err(|error| error.to_string())?;
    assert_eq!(snapshot["instance_ref"]["entity_id"], "card-17");
    Ok(())
}

#[test]
fn foreign_scope_selector_and_oversize_response_fail_closed() -> Result<(), String> {
    let request = request(None);
    let mut foreign = golden("bootstrap-response.json")?;
    foreign["scope"]["run_id"] = json!("other-run");
    assert_eq!(
        select_snapshot(&request, &foreign),
        Err(BootstrapError::Scope)
    );
    let mut malformed = golden("bootstrap-response.json")?;
    malformed["visible_entities"][0]["instance_ref"]["instance_id"] = json!("other-instance");
    assert_eq!(
        select_snapshot(&request, &malformed),
        Err(BootstrapError::Scope)
    );
    let mut oversized = golden("bootstrap-response.json")?;
    oversized["visible_entities"][0]["definition_ref"]["namespaced_id"] =
        json!("ironclad:".to_owned() + &"x".repeat(MAX_MESSAGE_BYTES));
    assert_eq!(
        select_snapshot(&request, &oversized),
        Err(BootstrapError::Bounds)
    );
    let mut unknown = golden("bootstrap-response.json")?;
    unknown["selector"]["unexpected"] = json!(true);
    assert_eq!(
        select_snapshot(&request, &unknown),
        Err(BootstrapError::Invalid)
    );
    Ok(())
}
