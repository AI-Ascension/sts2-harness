// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use sts2_harness::context_memory::*;
use sts2_harness::effective_limits::*;
use sts2_harness::effective_limits_pins::*;

const CONSOLE: &str = "AI-Ascension/ascension-context-console";

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

fn aligned_console() -> PinMatrix {
    let mut matrix = PinMatrix::repository().expect("committed matrix");
    let surfaces = matrix
        .producer
        .surfaces
        .iter()
        .map(|surface| ConsumerSurface {
            surface: surface.surface.clone(),
            advertised_capability_schema: Some(surface.capability_schema.clone()),
            effective_limits_advertised: true,
        })
        .collect::<Vec<_>>();
    let artifacts = matrix
        .producer
        .surfaces
        .iter()
        .flat_map(|surface| &surface.artifacts)
        .map(|artifact| ConsumerArtifact {
            path: artifact.path.clone(),
            sha256: artifact.sha256.clone(),
        })
        .collect::<Vec<_>>();
    let consumer = matrix
        .consumers
        .iter_mut()
        .find(|entry| entry.repository == CONSOLE)
        .expect("console consumer");
    consumer.adoption = Adoption::Aligned;
    consumer.artifact_mode = ArtifactMode::CopiedContracts;
    consumer.surfaces = surfaces;
    consumer.artifacts = artifacts;
    matrix.validate().expect("aligned matrix validates");
    matrix
}

#[test]
fn incomplete_consumer_inventory_blocks_admission() {
    let mut matrix = aligned_console();
    matrix
        .consumers
        .iter_mut()
        .find(|entry| entry.repository == CONSOLE)
        .expect("console")
        .artifacts
        .pop();
    assert_eq!(
        matrix.admit_consumer(
            CONSOLE,
            "context-memory",
            &memory_record(),
            &memory_record(),
            "max_candidates",
            1
        ),
        Err(UnavailableReason::ConsumerPinNotAdopted)
    );
}

#[test]
fn producer_kind_path_mismatch_is_rejected() {
    let mut matrix = aligned_console();
    let surface = &mut matrix.producer.surfaces[0];
    surface.artifacts[1].path = surface.artifacts[0].path.clone();
    surface.artifacts[1].sha256 = surface.artifacts[0].sha256.clone();
    assert_eq!(matrix.validate(), Err(PinMatrixError::InvalidMatrix));
}
