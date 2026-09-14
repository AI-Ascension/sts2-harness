// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::process::{Command, Stdio};
use sts2_harness::context_memory::*;
use sts2_harness::effective_limits::*;
use sts2_harness::effective_limits_pins::*;
use sts2_harness::provider_session::NativeCapabilities;

const STUDIO: &str = "AI-Ascension/ascension-workflow-studio";
const CONSOLE: &str = "AI-Ascension/ascension-context-console";

fn matrix() -> PinMatrix {
    let matrix = PinMatrix::repository().expect("committed matrix");
    matrix.validate().expect("committed matrix validates");
    matrix
}

fn memory_record() -> EffectiveLimitRecord {
    let scope = MemoryScope::new(
        "project-fixture",
        "run-fixture",
        "episode-fixture",
        "agent-fixture",
    );
    MemoryCorpus::with_limits(scope, 16, 4096)
        .expect("corpus")
        .capabilities()
        .effective_limit_record()
}

fn session_record() -> EffectiveLimitRecord {
    NativeCapabilities::fixture().effective_limit_record()
}

fn aligned_consumer(matrix: &PinMatrix, repository: &str) -> ConsumerPin {
    let mut consumer = matrix
        .consumers
        .iter()
        .find(|entry| entry.repository == repository)
        .expect("recorded consumer")
        .clone();
    consumer.adoption = Adoption::Aligned;
    consumer.surfaces = matrix
        .producer
        .surfaces
        .iter()
        .map(|surface| ConsumerSurface {
            surface: surface.surface.clone(),
            advertised_capability_schema: Some(surface.capability_schema.clone()),
            effective_limits_advertised: true,
        })
        .collect();
    if consumer.artifact_mode == ArtifactMode::CopiedContracts {
        consumer.artifacts = matrix
            .producer
            .surfaces
            .iter()
            .flat_map(|surface| &surface.artifacts)
            .map(|artifact| ConsumerArtifact {
                path: artifact.path.clone(),
                sha256: artifact.sha256.clone(),
            })
            .collect();
    }
    consumer
}

fn with_consumer(matrix: &PinMatrix, consumer: ConsumerPin) -> PinMatrix {
    let mut replaced = matrix.clone();
    replaced
        .consumers
        .retain(|entry| entry.repository != consumer.repository);
    replaced.consumers.push(consumer);
    replaced
}

#[test]
fn committed_matrix_records_pending_consumers_and_never_treats_absent_as_unlimited() {
    let matrix = matrix();
    assert_eq!(matrix.producer.surfaces.len(), 2);
    for repository in [CONSOLE, STUDIO] {
        let consumer = matrix
            .consumers
            .iter()
            .find(|entry| entry.repository == repository)
            .expect("recorded consumer");
        assert_eq!(consumer.adoption, Adoption::Pending);
        assert!(!matrix.alignment_holds(consumer));
        for surface in &consumer.surfaces {
            assert!(!surface.effective_limits_advertised);
        }
        assert_eq!(
            matrix.admit_consumer(
                repository,
                "context-memory",
                &memory_record(),
                "max_candidates",
                1
            ),
            Err(UnavailableReason::ConsumerPinNotAdopted)
        );
    }
    assert_eq!(
        matrix.admit_consumer(
            "AI-Ascension/unknown",
            "context-memory",
            &memory_record(),
            "max_candidates",
            1
        ),
        Err(UnavailableReason::ConsumerNotRecorded)
    );
    assert_eq!(
        matrix.admit_consumer(CONSOLE, "runtime-v4", &memory_record(), "max_candidates", 1),
        Err(UnavailableReason::FieldNotAdvertised)
    );
}

#[test]
fn consumer_cannot_present_a_schema_valid_value_the_runtime_rejects() {
    let matrix = matrix();
    for repository in [CONSOLE, STUDIO] {
        let aligned = with_consumer(&matrix, aligned_consumer(&matrix, repository));
        aligned.validate().expect("aligned matrix");
        for record in [memory_record(), session_record()] {
            for row in &record.rows {
                assert_eq!(
                    aligned.admit_consumer(repository, &record.surface, &record, &row.field, 1),
                    Ok(()),
                    "{}: {}",
                    repository,
                    row.field
                );
                assert_eq!(
                    aligned.admit_consumer(
                        repository,
                        &record.surface,
                        &record,
                        &row.field,
                        row.executable_ceiling
                    ),
                    Ok(()),
                    "{}: {}",
                    repository,
                    row.field
                );
                assert_eq!(
                    aligned.admit_consumer(
                        repository,
                        &record.surface,
                        &record,
                        &row.field,
                        row.executable_ceiling.saturating_add(1)
                    ),
                    Err(UnavailableReason::EffectiveLimitExceeded),
                    "{}: {}",
                    repository,
                    row.field
                );
            }
        }
        let record = memory_record();
        let over_schema_valid = MemoryPolicy {
            schema: MEMORY_POLICY_SCHEMA.to_owned(),
            policy_id: "policy-consumer".to_owned(),
            version: 1,
            scope: MemoryScope::new("a", "b", "c", "d"),
            mode: PolicyMode::ManualSnapshot,
            status: PolicyStatus::Approved,
            phase2_revision_id: Some("revision-1".to_owned()),
            corpus_generation: 1,
            rolling_same_episode_sources: false,
            cross_scope: false,
            approved_summary_catalog: Vec::new(),
            ranker_version: "lexical-v1".to_owned(),
            query_derivation_version: "derive-v1".to_owned(),
            max_candidates: MAX_CANDIDATES,
            max_results: MAX_RESULTS,
            max_selected: MAX_SELECTED,
            optional_byte_budget: MEMORY_POLICY_SCHEMA_MAX_OPTIONAL_BYTES,
            fallback: SelectionFallback::Block,
            automatic_summary_activation: false,
            generate_during_selection: false,
            authorization_policy_version: "auth-v1".to_owned(),
        };
        over_schema_valid
            .validate_schema()
            .expect("portable schema");
        assert_eq!(
            aligned.admit_consumer(
                repository,
                "context-memory",
                &record,
                "optional_byte_budget",
                over_schema_valid.optional_byte_budget as u64
            ),
            Err(UnavailableReason::EffectiveLimitExceeded)
        );
    }
}

#[test]
fn tampered_or_stale_pins_and_records_cannot_authorize_a_larger_limit() {
    let matrix = matrix();

    let mut drifted = matrix.clone();
    drifted.producer.surfaces[0].artifacts[0].sha256 = "0".repeat(64);
    assert_eq!(
        drifted.validate(),
        Err(PinMatrixError::ProducerArtifactDrift)
    );

    let mut unknown = matrix.clone();
    unknown.producer.surfaces[0].artifacts[1].path =
        "contracts/context-memory/unknown.json".to_owned();
    assert_eq!(
        unknown.validate(),
        Err(PinMatrixError::MissingProducerArtifact)
    );

    let mut duplicated = matrix.clone();
    duplicated.consumers.push(duplicated.consumers[0].clone());
    assert_eq!(
        duplicated.validate(),
        Err(PinMatrixError::DuplicateConsumer)
    );

    let mut undated = matrix.clone();
    undated.recorded_on = "2026-9-14".to_owned();
    assert_eq!(undated.validate(), Err(PinMatrixError::InvalidMatrix));

    let mut repinned = matrix.clone();
    let pin = repinned
        .consumers
        .iter_mut()
        .find(|entry| entry.repository == STUDIO)
        .expect("studio");
    pin.harness_ci_pin = Some(HarnessCiPin {
        workflow: STUDIO_CONTRACT_WORKFLOW_PATH.to_owned(),
        revision: "0".repeat(40),
    });
    assert_eq!(repinned.validate(), Err(PinMatrixError::ConsumerPinDrift));

    assert_eq!(
        with_consumer(&matrix, {
            let mut consumer = matrix
                .consumers
                .iter()
                .find(|entry| entry.repository == CONSOLE)
                .expect("console")
                .clone();
            consumer.adoption = Adoption::Aligned;
            consumer
        })
        .validate(),
        Err(PinMatrixError::ConsumerAlignmentMismatch)
    );

    let mut stale_label = aligned_consumer(&matrix, STUDIO);
    stale_label.adoption = Adoption::Pending;
    assert_eq!(
        with_consumer(&matrix, stale_label).validate(),
        Err(PinMatrixError::AdoptionLabelStale)
    );

    let aligned = with_consumer(&matrix, aligned_consumer(&matrix, STUDIO));
    let mut legacy_record = memory_record();
    legacy_record.capability_schema = "ascension.context-memory.capabilities.v1".to_owned();
    assert_eq!(
        aligned.admit_consumer(
            STUDIO,
            "context-memory",
            &legacy_record,
            "max_candidates",
            1
        ),
        Err(UnavailableReason::FieldNotAdvertised)
    );
    assert_eq!(
        aligned.validate_record(&legacy_record),
        Err(UnavailableReason::ProfileMismatch)
    );
}

#[test]
fn tampered_record_ceilings_and_stale_descriptors_fail_closed() {
    let record = memory_record();
    let mut raised = record.clone();
    raised
        .rows
        .iter_mut()
        .find(|row| row.field == "optional_byte_budget")
        .expect("row")
        .executable_ceiling = MEMORY_POLICY_SCHEMA_MAX_OPTIONAL_BYTES as u64;
    assert_eq!(raised.validate(), Err(LimitRecordError::InvalidRecord));
    assert_eq!(
        raised.admit_authorized(
            "context-memory",
            "harness-context-memory-v3",
            &record.capability_descriptor_sha256,
            "optional_byte_budget",
            65_536
        ),
        Err(UnavailableReason::DescriptorTampered)
    );

    let trusted_digest = record.capability_descriptor_sha256.clone();
    assert_eq!(
        record.admit_authorized(
            "context-memory",
            "harness-context-memory-v3",
            &trusted_digest,
            "optional_byte_budget",
            8_192
        ),
        Ok(())
    );
    assert_eq!(
        record.admit_authorized(
            "context-memory",
            "harness-context-memory-v3",
            &"0".repeat(64),
            "optional_byte_budget",
            8_192
        ),
        Err(UnavailableReason::DescriptorStale)
    );
    assert_eq!(
        record.admit_authorized(
            "provider-session",
            "harness-context-memory-v3",
            &trusted_digest,
            "optional_byte_budget",
            8_192
        ),
        Err(UnavailableReason::ProfileMismatch)
    );
    assert_eq!(
        record.admit_authorized(
            "context-memory",
            "harness-context-memory-v3",
            &trusted_digest,
            "optional_byte_budget",
            8_193
        ),
        Err(UnavailableReason::EffectiveLimitExceeded)
    );
}

#[test]
fn executable_cli_publishes_the_effective_limit_record() {
    let output = Command::new(env!("CARGO_BIN_EXE_context-memory-cli"))
        .arg("limits")
        .stdin(Stdio::null())
        .output()
        .expect("run memory cli");
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("record json");
    assert_eq!(value["schema"], "ascension.harness.effective-limits.v1");
    assert_eq!(value["surface"], "context-memory");
    assert_eq!(
        value["rows"]
            .as_array()
            .expect("rows")
            .iter()
            .find(|row| row["field"] == "optional_byte_budget")
            .expect("optional row")["class"],
        "schema_broader_than_executable"
    );
}
