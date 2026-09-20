// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::path::PathBuf;

use sts2_harness::semantic_history::{
    SemanticAppendOutcome, SemanticCaptureWindow, SemanticCatalogBinding, SemanticCausalParent,
    SemanticCausalProvenance, SemanticCoverageInterval, SemanticCoverageStatus, SemanticEventBatch,
    SemanticEventCoverage, SemanticEventInput, SemanticEventKind, SemanticEventListQuery,
    SemanticEventOrigin, SemanticEventScope, SemanticEventSubject, SemanticHistoryAppend,
    SemanticHistoryError, SemanticHistoryFence, SemanticHistoryFork, SemanticHistoryRefusal,
    SemanticHistoryStore, SemanticIdentityNamespace, SemanticQuantity, SemanticReference,
    SemanticSubjectRole, page_history, traverse_causes,
};

fn binding() -> SemanticCatalogBinding {
    SemanticCatalogBinding {
        manifest_digest: "manifest-digest-a".to_owned(),
        producer_version: "game-semantic-event-reference-producer-v1".to_owned(),
    }
}

fn scope(branch: &str) -> SemanticEventScope {
    SemanticEventScope {
        run_id: "run-1".to_owned(),
        branch_id: branch.to_owned(),
        episode: 1,
        epoch: 4,
    }
}

fn window(start: u64) -> SemanticCaptureWindow {
    SemanticCaptureWindow {
        capture_start_sequence: start,
        history_before_capture: start != 1,
        intervals: Vec::new(),
    }
}

fn dropped(first: u64, last: u64) -> SemanticCoverageInterval {
    SemanticCoverageInterval {
        status: SemanticCoverageStatus::Dropped,
        first_sequence: first,
        last_sequence: last,
    }
}

fn actor(id: &str) -> SemanticEventSubject {
    SemanticEventSubject {
        role: SemanticSubjectRole::Actor,
        namespace: SemanticIdentityNamespace::LiveInstance,
        subject_id: id.to_owned(),
    }
}

fn target(id: &str) -> SemanticEventSubject {
    SemanticEventSubject {
        role: SemanticSubjectRole::Target,
        namespace: SemanticIdentityNamespace::LiveInstance,
        subject_id: id.to_owned(),
    }
}

fn damage(event_id: &str, sequence: u64, parent: Option<&str>) -> SemanticEventInput {
    SemanticEventInput {
        event_id: event_id.to_owned(),
        sequence,
        coverage: SemanticEventCoverage {
            status: SemanticCoverageStatus::Captured,
            label: None,
        },
        kind: Some(SemanticEventKind::Damage),
        origin: Some(SemanticEventOrigin::Native),
        subjects: vec![actor("player-1"), target("enemy-1")],
        causal_parent: Some(match parent {
            Some(parent) => SemanticCausalParent {
                parent_event_id: Some(parent.to_owned()),
                provenance: SemanticCausalProvenance::Stated,
            },
            None => SemanticCausalParent::not_stated(),
        }),
        value: Some(SemanticQuantity {
            amount: 6,
            unit: "health".to_owned(),
        }),
        reference: None,
        label: None,
    }
}

fn card_played(event_id: &str, sequence: u64) -> SemanticEventInput {
    SemanticEventInput {
        event_id: event_id.to_owned(),
        sequence,
        coverage: SemanticEventCoverage {
            status: SemanticCoverageStatus::Captured,
            label: None,
        },
        kind: Some(SemanticEventKind::CardPlayed),
        origin: Some(SemanticEventOrigin::Native),
        subjects: vec![actor("player-1")],
        causal_parent: None,
        value: None,
        reference: Some(SemanticReference {
            entity_kind: "card".to_owned(),
            namespaced_id: "card.strike".to_owned(),
        }),
        label: None,
    }
}

fn gap(event_id: &str, sequence: u64, status: SemanticCoverageStatus) -> SemanticEventInput {
    SemanticEventInput {
        event_id: event_id.to_owned(),
        sequence,
        coverage: SemanticEventCoverage {
            status,
            label: Some("capture dropped".to_owned()),
        },
        kind: None,
        origin: None,
        subjects: Vec::new(),
        causal_parent: None,
        value: None,
        reference: None,
        label: None,
    }
}

fn batch(branch: &str, start: u64, events: Vec<SemanticEventInput>) -> SemanticEventBatch {
    SemanticEventBatch {
        scope: scope(branch),
        window: window(start),
        events,
    }
}

fn append(
    operation: &str,
    branch: &str,
    start: u64,
    events: Vec<SemanticEventInput>,
) -> SemanticHistoryAppend {
    SemanticHistoryAppend {
        operation_id: operation.to_owned(),
        binding: binding(),
        batch: batch(branch, start, events),
    }
}

fn path(name: &str) -> PathBuf {
    let mut root = std::env::temp_dir();
    root.push(format!(
        "semantic-history-{name}-{}.json",
        std::process::id()
    ));
    root
}

fn error(batch: SemanticEventBatch) -> SemanticHistoryRefusal {
    sts2_harness::semantic_history::admit_batch(&binding(), &batch)
        .expect_err("batch must be refused")
        .refusal
}

#[path = "semantic_history/admission.rs"]
mod admission;
#[path = "semantic_history/backfill.rs"]
mod backfill;
#[path = "semantic_history/branch_retention.rs"]
mod branch_retention;
#[path = "semantic_history/lookup.rs"]
mod lookup;
#[path = "semantic_history/persistence.rs"]
mod persistence;
#[path = "semantic_history/query.rs"]
mod query;
#[path = "semantic_history/retention.rs"]
mod retention;
