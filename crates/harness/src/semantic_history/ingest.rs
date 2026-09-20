// SPDX-License-Identifier: MIT

//! Admission: what a batch must prove before any of it is retained.

use std::collections::BTreeSet;

use super::error::{
    SemanticHistoryError, SemanticHistoryRefusal as Refusal, SemanticHistoryResult,
};
use super::record::{
    SemanticCaptureWindow, SemanticCausalParent, SemanticEventBatch, SemanticEventInput,
    SemanticEventRecord,
};
use super::roles::{SemanticCausalProvenance, SemanticSubjectRole};
use super::scope::{
    SEMANTIC_MAX_EVENTS, SEMANTIC_MAX_HISTORY_BYTES, SEMANTIC_MAX_IDENTITY_BYTES,
    SEMANTIC_MAX_LABEL_BYTES, SEMANTIC_MAX_UNIT_BYTES, SemanticCatalogBinding,
};
use super::window::{inside_declared_gap, validate_gap, validate_window};

/// Validates a batch against a catalog binding and returns the records it proves.
///
/// Nothing is partially admitted: a batch that fails any check yields no records at all, so a
/// history can never hold the valid half of an invalid statement.
pub fn admit_batch(
    binding: &SemanticCatalogBinding,
    batch: &SemanticEventBatch,
) -> SemanticHistoryResult<Vec<SemanticEventRecord>> {
    if batch.events.len() > SEMANTIC_MAX_EVENTS {
        return Err(SemanticHistoryError::new(Refusal::TooManyEvents));
    }
    validate_binding(binding)?;
    validate_window(&batch.window)?;
    let mut seen_events = BTreeSet::new();
    let mut expected = batch.window.capture_start_sequence;
    let mut bytes = 0usize;
    let mut records = Vec::with_capacity(batch.events.len());
    for event in &batch.events {
        if event.sequence != expected {
            return Err(SemanticHistoryError::about(
                Refusal::NonMonotonicSequence,
                &event.event_id,
            ));
        }
        expected = expected.saturating_add(1);
        if !seen_events.insert(event.event_id.as_str()) {
            return Err(SemanticHistoryError::about(
                Refusal::DuplicateEvent,
                &event.event_id,
            ));
        }
        bytes = bytes.saturating_add(event.byte_len());
        validate_event(event, &batch.window)?;
        records.push(SemanticEventRecord {
            binding: binding.clone(),
            scope: batch.scope.clone(),
            event: event.clone(),
        });
    }
    if bytes > SEMANTIC_MAX_HISTORY_BYTES {
        return Err(SemanticHistoryError::new(Refusal::TooManyBytes));
    }
    validate_causality(&records)?;
    Ok(records)
}

fn validate_binding(binding: &SemanticCatalogBinding) -> SemanticHistoryResult<()> {
    if binding.manifest_digest.is_empty()
        || binding.manifest_digest.len() > SEMANTIC_MAX_IDENTITY_BYTES
        || binding.producer_version.is_empty()
        || binding.producer_version.len() > SEMANTIC_MAX_IDENTITY_BYTES
    {
        return Err(SemanticHistoryError::new(Refusal::BindingMismatch));
    }
    Ok(())
}

fn validate_event(
    event: &SemanticEventInput,
    window: &SemanticCaptureWindow,
) -> SemanticHistoryResult<()> {
    validate_identity(&event.event_id)?;
    if let Some(label) = &event.label {
        validate_label(label)?;
    }
    if let Some(coverage_label) = &event.coverage.label {
        validate_label(coverage_label)?;
    }
    if event.coverage.status.is_observed() {
        validate_captured(event, window)
    } else {
        validate_gap(event, window)
    }
}

fn validate_captured(
    event: &SemanticEventInput,
    window: &SemanticCaptureWindow,
) -> SemanticHistoryResult<()> {
    let Some(kind) = event.kind else {
        return Err(SemanticHistoryError::about(
            Refusal::CoverageShape,
            &event.event_id,
        ));
    };
    if event.origin.is_none() {
        return Err(SemanticHistoryError::about(
            Refusal::CoverageShape,
            &event.event_id,
        ));
    }
    if inside_declared_gap(window, event.sequence) {
        return Err(SemanticHistoryError::about(
            Refusal::CapturedInsideGap,
            &event.event_id,
        ));
    }
    validate_subjects(event, kind)?;
    if kind.requires_quantity() {
        let Some(value) = &event.value else {
            return Err(SemanticHistoryError::about(
                Refusal::MissingDetail,
                &event.event_id,
            ));
        };
        if value.unit.is_empty() || value.unit.len() > SEMANTIC_MAX_UNIT_BYTES {
            return Err(SemanticHistoryError::about(Refusal::Label, &event.event_id));
        }
    } else if event.value.is_some() {
        return Err(SemanticHistoryError::about(
            Refusal::UnexpectedDetail,
            &event.event_id,
        ));
    }
    if kind.requires_reference() {
        let Some(reference) = &event.reference else {
            return Err(SemanticHistoryError::about(
                Refusal::MissingDetail,
                &event.event_id,
            ));
        };
        validate_identity(&reference.entity_kind)?;
        validate_identity(&reference.namespaced_id)?;
    } else if event.reference.is_some() {
        return Err(SemanticHistoryError::about(
            Refusal::UnexpectedDetail,
            &event.event_id,
        ));
    }
    match (kind.admits_cause(), event.causal_parent.as_ref()) {
        (false, Some(_)) => Err(SemanticHistoryError::about(
            Refusal::CausalityNotAdmitted,
            &event.event_id,
        )),
        (true, None) => Err(SemanticHistoryError::about(
            Refusal::CausalShape,
            &event.event_id,
        )),
        (_, Some(parent)) => validate_parent_shape(event, parent),
        (false, None) => Ok(()),
    }
}

fn validate_subjects(
    event: &SemanticEventInput,
    kind: super::vocabulary::SemanticEventKind,
) -> SemanticHistoryResult<()> {
    let mut actors = 0usize;
    let mut targets = 0usize;
    for subject in &event.subjects {
        if !subject.namespace.admits_subject_role() {
            return Err(SemanticHistoryError::about(
                Refusal::SubjectNamespace,
                &event.event_id,
            ));
        }
        validate_identity(&subject.subject_id)?;
        match subject.role {
            SemanticSubjectRole::Actor => actors += 1,
            SemanticSubjectRole::Target => targets += 1,
        }
    }
    if actors > 1 || targets > 1 {
        return Err(SemanticHistoryError::about(
            Refusal::SubjectRole,
            &event.event_id,
        ));
    }
    if kind.requires_actor() && actors != 1 {
        return Err(SemanticHistoryError::about(
            Refusal::SubjectRole,
            &event.event_id,
        ));
    }
    if kind.requires_target() && targets != 1 {
        return Err(SemanticHistoryError::about(
            Refusal::MissingTarget,
            &event.event_id,
        ));
    }
    if !kind.requires_target() && targets > 0 && !kind.admits_target() {
        return Err(SemanticHistoryError::about(
            Refusal::UnexpectedDetail,
            &event.event_id,
        ));
    }
    Ok(())
}

fn validate_parent_shape(
    event: &SemanticEventInput,
    parent: &SemanticCausalParent,
) -> SemanticHistoryResult<()> {
    let agrees = match parent.provenance {
        SemanticCausalProvenance::Stated => parent.parent_event_id.is_some(),
        SemanticCausalProvenance::NotStated => parent.parent_event_id.is_none(),
    };
    if !agrees {
        return Err(SemanticHistoryError::about(
            Refusal::CausalShape,
            &event.event_id,
        ));
    }
    if let Some(parent_id) = parent.stated_parent() {
        validate_identity(parent_id)?;
        let imported = event
            .origin
            .is_some_and(|origin| !origin.admits_stated_parent());
        if imported {
            return Err(SemanticHistoryError::about(
                Refusal::ImportedCausality,
                &event.event_id,
            ));
        }
    }
    Ok(())
}

fn validate_causality(records: &[SemanticEventRecord]) -> SemanticHistoryResult<()> {
    let positions = records
        .iter()
        .map(|record| (record.event_id(), record.event.sequence))
        .collect::<std::collections::BTreeMap<_, _>>();
    for record in records {
        let Some(parent) = record.event.causal_parent.as_ref() else {
            continue;
        };
        let Some(parent_id) = parent.stated_parent() else {
            continue;
        };
        let Some(parent_sequence) = positions.get(parent_id) else {
            return Err(SemanticHistoryError::about(
                Refusal::StatedParentUnknown,
                record.event_id(),
            ));
        };
        if *parent_sequence >= record.event.sequence {
            return Err(SemanticHistoryError::about(
                Refusal::StatedParentNotBefore,
                record.event_id(),
            ));
        }
    }
    Ok(())
}

fn validate_identity(value: &str) -> SemanticHistoryResult<()> {
    let bounded = !value.is_empty()
        && value.len() <= SEMANTIC_MAX_IDENTITY_BYTES
        && !value.contains('/')
        && !value.contains('\\')
        && !value.contains('\0')
        && value != "."
        && value != ".."
        && !value.split('.').any(|segment| segment == "..")
        && !value.chars().any(|character| character.is_control());
    if bounded {
        Ok(())
    } else {
        Err(SemanticHistoryError::about(Refusal::Identity, value))
    }
}

fn validate_label(value: &str) -> SemanticHistoryResult<()> {
    if value.len() <= SEMANTIC_MAX_LABEL_BYTES {
        Ok(())
    } else {
        Err(SemanticHistoryError::new(Refusal::Label))
    }
}
