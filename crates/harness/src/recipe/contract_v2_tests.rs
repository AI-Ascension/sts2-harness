// SPDX-License-Identifier: MIT

use super::{
    RecipeAdmissionErrorV2, RecipeContractVersionV2, RecipeDefinitionV2, RecipeOperationV2,
    RecipeOutputV2, admit_recipe_v2,
};
use sts2_protocol::{
    RUNTIME_MAP_V1_PROTOCOL_VERSION, RUNTIME_MAP_V1_SNAPSHOT_SCHEMA_VERSION,
    RuntimeMapV1Availability, RuntimeMapV1Completeness, RuntimeMapV1Context, RuntimeMapV1Freshness,
    RuntimeMapV1Message, RuntimeMapV1Position, RuntimeMapV1Snapshot, canonical_map_message_json,
    decode_runtime_map_message,
};

const VALID_DEFINITION: &str = r#"{
    "schema_version": "ascension.recipe/v2",
    "id": "map-read",
    "revision": 1,
    "operation": { "kind": "map_snapshot" }
}"#;

const INVALID_DEFINITIONS: [&str; 7] = [
    r#"{
        "schema_version":"ascension.recipe/v2",
        "id":"x","revision":1,
        "operation":{"kind":"map_snapshot"},
        "tool":"sts2.map_snapshot"
    }"#,
    r#"{
        "schema_version":"ascension.recipe/v2",
        "id":"x","revision":1,
        "operation":{"kind":"map_snapshot","url":"https://example.invalid"}
    }"#,
    r#"{
        "schema_version":"ascension.recipe/v2",
        "id":"x","revision":1,
        "operation":{"kind":"map_snapshot","endpoint":"/map"}
    }"#,
    r#"{
        "schema_version":"ascension.recipe/v2",
        "id":"x","revision":1,
        "operation":{"kind":"map_snapshot","arguments":{"instance":"caller"}}
    }"#,
    r#"{
        "schema_version":"ascension.recipe/v2",
        "id":"x","revision":1,
        "operation":{"kind":"map_snapshot","schema":"arbitrary-map-schema"}
    }"#,
    r#"{
        "schema_version":"ascension.recipe/v2",
        "id":"x","revision":1,
        "operation":{"kind":"map_snapshot","template":"${caller_input}"}
    }"#,
    r#"{
        "schema_version":"ascension.recipe/v2",
        "id":"x","revision":1,
        "operation":{"kind":"map_snapshot"},"steps":[]
    }"#,
];

fn definition_from_json(json: &str) -> Result<RecipeDefinitionV2, serde_json::Error> {
    serde_json::from_str(json)
}

#[test]
fn admits_one_fixed_map_snapshot_read() -> Result<(), Box<dyn std::error::Error>> {
    let definition = definition_from_json(VALID_DEFINITION)?;
    let admitted = admit_recipe_v2(definition)?;

    assert_eq!(admitted.id().as_str(), "map-read");
    assert_eq!(admitted.revision().get(), 1);
    assert_eq!(admitted.operation(), RecipeOperationV2::MapSnapshot {});
    Ok(())
}

#[test]
fn only_the_fixed_contract_version_deserializes() -> Result<(), Box<dyn std::error::Error>> {
    let unsupported = VALID_DEFINITION.replace("ascension.recipe/v2", "ascension.recipe/v9");
    assert!(definition_from_json(&unsupported).is_err());

    let parsed = definition_from_json(VALID_DEFINITION)?;
    assert_eq!(parsed.schema_version, RecipeContractVersionV2::V2);
    Ok(())
}

#[test]
fn rejects_unknown_definition_and_operation_fields() {
    for json in INVALID_DEFINITIONS {
        assert!(
            definition_from_json(json).is_err(),
            "unexpected acceptance: {json}"
        );
    }
}

#[test]
fn rejects_unknown_operation_kind() {
    let invalid = VALID_DEFINITION.replace("map_snapshot", "arbitrary_tool");
    assert!(definition_from_json(&invalid).is_err());
}

#[test]
fn admission_rejects_invalid_ids_and_zero_revision() {
    let empty_id = RecipeDefinitionV2::new("", 1, RecipeOperationV2::MapSnapshot {});
    assert_eq!(
        admit_recipe_v2(empty_id),
        Err(RecipeAdmissionErrorV2::InvalidRecipeId)
    );

    let path_like_id = RecipeDefinitionV2::new("../map", 1, RecipeOperationV2::MapSnapshot {});
    assert_eq!(
        admit_recipe_v2(path_like_id),
        Err(RecipeAdmissionErrorV2::InvalidRecipeId)
    );

    let long_id = "x".repeat(super::super::MAX_IDENTIFIER_LEN + 1);
    let oversized_id = RecipeDefinitionV2::new(long_id, 1, RecipeOperationV2::MapSnapshot {});
    assert_eq!(
        admit_recipe_v2(oversized_id),
        Err(RecipeAdmissionErrorV2::InvalidRecipeId)
    );

    let zero_revision = RecipeDefinitionV2::new("map-read", 0, RecipeOperationV2::MapSnapshot {});
    assert_eq!(
        admit_recipe_v2(zero_revision),
        Err(RecipeAdmissionErrorV2::ZeroRevision)
    );
}

fn empty_pre_start_snapshot() -> RuntimeMapV1Snapshot {
    RuntimeMapV1Snapshot {
        state_id: "state-1".to_owned(),
        generation: 7,
        schema_version: RUNTIME_MAP_V1_SNAPSHOT_SCHEMA_VERSION.to_owned(),
        projection_version: RUNTIME_MAP_V1_PROTOCOL_VERSION.to_owned(),
        game_build: "test-build".to_owned(),
        mod_version: "test-mod".to_owned(),
        map_instance_id: Some("map-1".to_owned()),
        act_id: Some(1),
        scope_id: Some("scope-1".to_owned()),
        availability: RuntimeMapV1Availability::Available,
        completeness: RuntimeMapV1Completeness::Complete,
        freshness: RuntimeMapV1Freshness::Current,
        reason: None,
        nodes: vec![],
        edges: vec![],
        position: RuntimeMapV1Position::PreStart {},
        history: vec![],
        terminal_node_ids: vec![],
        bindings: vec![],
    }
}

#[test]
fn map_output_is_a_typed_protocol_snapshot() -> Result<(), Box<dyn std::error::Error>> {
    let snapshot = empty_pre_start_snapshot();
    let envelope = RuntimeMapV1Message::snapshot_response(
        RuntimeMapV1Context::new("corr-1", "instance-1", "session-1", "lease-1", 2),
        snapshot.clone(),
        None,
    );
    let canonical = canonical_map_message_json(&envelope)?;
    let decoded = decode_runtime_map_message(canonical.as_bytes())?;

    assert_eq!(decoded, envelope);
    assert_eq!(decoded.generation, snapshot.generation);
    assert_eq!(decoded.snapshot.as_ref(), Some(&snapshot));

    let output: RecipeOutputV2 = RecipeOutputV2::MapSnapshot(snapshot);
    let RecipeOutputV2::MapSnapshot(typed_snapshot) = output;
    assert_eq!(typed_snapshot.state_id, "state-1");
    assert_eq!(
        typed_snapshot.projection_version.as_str(),
        RUNTIME_MAP_V1_PROTOCOL_VERSION
    );
    Ok(())
}
