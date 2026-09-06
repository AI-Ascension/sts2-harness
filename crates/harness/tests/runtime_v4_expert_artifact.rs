// SPDX-License-Identifier: MIT

use sts2_harness::{
    ExoDecisionRequest, ModelExecutionId, RUNTIME_V4_EXPERT_ARTIFACT,
    RUNTIME_V4_EXPERT_PROTOCOL_VERSION, RUNTIME_V4_EXPERT_SCHEMA_DIGEST,
    RuntimeV4ExpertObservation, SanitizedObservation, verify_runtime_v4_expert_artifact,
};

fn golden() -> serde_json::Value {
    serde_json::from_slice(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../protocol-artifact/runtime-v4-expert/golden/observation.json"
    )))
    .unwrap_or(serde_json::Value::Null)
}

#[test]
fn copied_expert_profile_artifact_reaches_the_harness_consumer_boundary()
-> Result<(), Box<dyn std::error::Error>> {
    verify_runtime_v4_expert_artifact()?;
    assert_eq!(
        RUNTIME_V4_EXPERT_ARTIFACT,
        "sts2-protocol/runtime-v4-expert"
    );
    assert_eq!(RUNTIME_V4_EXPERT_PROTOCOL_VERSION, "runtime-v4-expert");
    assert_eq!(RUNTIME_V4_EXPERT_SCHEMA_DIGEST.len(), 64);
    Ok(())
}

#[test]
fn serialized_expert_observation_reaches_the_provider_firewall()
-> Result<(), Box<dyn std::error::Error>> {
    let value = golden();
    let bytes = serde_json::to_vec(&value)?;
    let observation = RuntimeV4ExpertObservation::parse(&bytes)?;
    assert_eq!(observation.state_id(), "live:7");
    assert_eq!(observation.generation(), 7);
    assert_eq!(
        observation.legal_action_ids().collect::<Vec<_>>(),
        vec![
            "play:7:card:1:enemy:1",
            "potion:7:potion:fire:enemy:1",
            "end:7"
        ]
    );
    let sanitized = SanitizedObservation::new(value.clone())?;
    assert_eq!(sanitized.as_value(), &value);
    let seed_blind = sanitized.without_visible_seed();
    assert_eq!(
        seed_blind.as_value()["visible_seed"],
        serde_json::Value::Null
    );
    assert!(SanitizedObservation::new(seed_blind.as_value().clone()).is_ok());
    let request = ExoDecisionRequest::new(
        ModelExecutionId::new(1).ok_or("invalid model execution ID")?,
        "7801005e6a1ab77008a05dbba80e0a2a7a56e35d",
        observation.state_id(),
        observation.generation(),
        seed_blind,
        observation.legal_action_ids().map(str::to_owned).collect(),
        "survive the current combat",
        Vec::new(),
        8 * 1024,
    )?;
    let encoded = request.encode(64 * 1024)?;
    let provider_request: serde_json::Value = serde_json::from_slice(&encoded)?;
    assert_eq!(
        provider_request["observation"]["protocol_version"],
        "runtime-v4-expert"
    );
    assert_eq!(
        provider_request["observation"]["player"]["potions"][0]["slot"],
        0
    );
    assert_eq!(
        provider_request["observation"]["legal_actions"][1]["action"]["kind"],
        "use_potion"
    );
    Ok(())
}

#[test]
fn expert_parser_rejects_nested_unknown_fields_and_digest_changes() {
    let mut value = golden();
    value["player"]["hand"][0]["private"] = serde_json::Value::String("secret".to_owned());
    assert!(RuntimeV4ExpertObservation::from_value(value).is_err());

    let mut value = golden();
    value["schema_digest"] = serde_json::Value::String("0".repeat(64));
    assert!(RuntimeV4ExpertObservation::from_value(value).is_err());
}
