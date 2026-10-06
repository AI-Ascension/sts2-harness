// SPDX-License-Identifier: MIT

use super::*;
use crate::management::AuthContext;
use crate::management::contract::{
    ExecutionMode, RunTargetConfiguration, TARGET_ADMISSION_SCHEMA_VERSION,
};
use crate::management::seed_key::Keyring;

fn keyring() -> Result<Keyring, Box<dyn std::error::Error>> {
    let text = format!(
        "schema=ascension.seed-keyring/v1\nauthority_id=workflow-service\ncurrent_version=key-1\nkey.key-1={}\n",
        "11".repeat(32)
    );
    Ok(Keyring::parse(text.as_bytes())?)
}

fn admission(catalog_revision: &str) -> TargetAdmissionBinding {
    TargetAdmissionBinding {
        schema_version: TARGET_ADMISSION_SCHEMA_VERSION.to_owned(),
        request_id: "seed-request-1".to_owned(),
        workflow_definition_digest: "a".repeat(64),
        target: RunTargetConfiguration {
            instance_id: "instance-1".to_owned(),
            execution_profile: "synthetic.workflow.v1".to_owned(),
            execution_mode: ExecutionMode::Synthetic,
            workflow_revision: "workflow-1".to_owned(),
            compatibility_revision: "compat-1".to_owned(),
            capability_revision: "capability-1".to_owned(),
            game_profile: "game-1".to_owned(),
            save_profile: None,
            inference_profile: None,
            context_capability: None,
            provider_capability: None,
        },
        descriptor_digest: "b".repeat(64),
        catalog_revision: catalog_revision.to_owned(),
    }
}

fn derive_request(
    binding: TargetAdmissionBinding,
) -> Result<WorkflowRunRequestV2, Box<dyn std::error::Error>> {
    Ok(serde_json::from_value(serde_json::json!({
        "schema_version": super::super::contract_seed_v2::WORKFLOW_RUN_REQUEST_V2_SCHEMA,
        "request_id": "seed-request-1",
        "definition": {"nodes": []},
        "artifact_id": null,
        "instance_id": "instance-1",
        "profile": "synthetic.workflow.v1",
        "admission": binding,
        "seed": {
            "schema_version": super::super::contract_seed_v2::WORKFLOW_SEED_REQUEST_V2_SCHEMA,
            "mode": "derive_once"
        }
    }))?)
}

#[test]
fn candidate_retains_exact_owner_admission_for_restart_verification()
-> Result<(), Box<dyn std::error::Error>> {
    let actor = AuthContext::new("owner-a", ["workflow:*".to_owned()])?;
    let admitted = admission("catalog-1");
    let request = derive_request(admitted.clone())?;
    let request_digest = request_digest(&request, &actor.subject)?;
    let keyring = keyring()?;
    let record = candidate_record(
        &request,
        &actor,
        &request_digest,
        "run-seed-1",
        &admitted,
        Some(&keyring),
    )?;

    assert_eq!(record.admitted_configuration, admitted);
    assert!(verify_candidate(
        &record,
        &request,
        &actor,
        &request_digest,
        "run-seed-1",
        Some(&keyring),
    )?);

    let mut evolved = record.clone();
    evolved.admitted_configuration.catalog_revision = "catalog-2".to_owned();
    assert!(matches!(
        validate_candidate_record(&evolved),
        Err(SeedKeyError::InvalidIdentity)
    ));
    assert!(matches!(
        verify_candidate(
            &evolved,
            &request,
            &actor,
            &request_digest,
            "run-seed-1",
            Some(&keyring),
        ),
        Err(SeedKeyError::InvalidIdentity)
    ));
    Ok(())
}

#[test]
fn pinned_seed_replay_uses_historical_key_after_current_rotation()
-> Result<(), Box<dyn std::error::Error>> {
    let actor = AuthContext::new("owner-a", ["workflow:*".to_owned()])?;
    let admitted = admission("catalog-1");
    let request = derive_request(admitted.clone())?;
    let request_digest = request_digest(&request, &actor.subject)?;
    let old_keyring = keyring()?;
    let record = candidate_record(
        &request,
        &actor,
        &request_digest,
        "run-seed-1",
        &admitted,
        Some(&old_keyring),
    )?;

    let rotated_text = format!(
        "schema=ascension.seed-keyring/v1\nauthority_id=workflow-service\ncurrent_version=key-2\nkey.key-1={}\nkey.key-2={}\n",
        "11".repeat(32),
        "22".repeat(32)
    );
    let rotated = Keyring::parse(rotated_text.as_bytes())?;
    assert!(verify_candidate(
        &record,
        &request,
        &actor,
        &request_digest,
        "run-seed-1",
        Some(&rotated),
    )?);

    let missing_old_text = format!(
        "schema=ascension.seed-keyring/v1\nauthority_id=workflow-service\ncurrent_version=key-2\nkey.key-2={}\n",
        "22".repeat(32)
    );
    let missing_old = Keyring::parse(missing_old_text.as_bytes())?;
    assert!(matches!(
        verify_candidate(
            &record,
            &request,
            &actor,
            &request_digest,
            "run-seed-1",
            Some(&missing_old),
        ),
        Err(SeedKeyError::KeyVersionUnavailable)
    ));
    Ok(())
}
