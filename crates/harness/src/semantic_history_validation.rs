// SPDX-License-Identifier: MIT

//! One event's shape: coverage decides which fields exist, and the kind decides the detail.

use super::{
    Error, SemanticHistoryCausalParent, SemanticHistoryEventInput, SemanticHistorySubjectRole,
    validate_history_identity,
};
use std::collections::BTreeSet;

/// Validates one supplied event against its kind, its subjects and the scope it belongs to.
///
/// The rules are refusals rather than defaults. A detail the kind requires and the host did not
/// supply is refused, because an absent actor, target or content reference would otherwise be read
/// as "there was none"; a detail the kind does not admit is refused for the same reason.
pub(super) fn validate_input(
    input: &SemanticHistoryEventInput,
    causal_parent: &SemanticHistoryCausalParent,
    branch_id: &str,
    run_id: &str,
) -> Result<(), Error> {
    validate_history_identity(&input.event_id, "event_id")?;
    input.coverage.validate()?;
    if let Some(value) = &input.value {
        value.validate()?;
    }
    if let Some(reference) = &input.reference {
        reference.validate()?;
    }
    if !input.coverage.status.is_observed() {
        return validate_gap(input, causal_parent);
    }
    validate_detail(input)?;
    if causal_parent.is_stated() && !input.kind.admits_cause() {
        // Only an effect can have a cause. A room transition or an offer that names one would record
        // causality the host never stated, which is the inference this boundary refuses.
        return Err(Error::Causality);
    }
    validate_subjects(input, branch_id, run_id)
}

/// Validates that a disclosed gap carries no observed detail.
///
/// A gap is the honest disclosure that an interval could not be observed. Attaching a subject, a
/// value, a content reference or a causal parent to it would close the gap with detail the capture
/// explicitly said it could not obtain.
fn validate_gap(
    input: &SemanticHistoryEventInput,
    causal_parent: &SemanticHistoryCausalParent,
) -> Result<(), Error> {
    if !input.subjects.is_empty() {
        return Err(Error::InvalidField("subjects"));
    }
    if input.value.is_some() {
        return Err(Error::InvalidField("value"));
    }
    if input.reference.is_some() {
        return Err(Error::InvalidField("reference"));
    }
    if causal_parent.is_stated() {
        return Err(Error::Causality);
    }
    Ok(())
}

/// Validates the detail one kind requires against what the event states, and vice versa.
fn validate_detail(input: &SemanticHistoryEventInput) -> Result<(), Error> {
    let kind = input.kind;
    if kind.requires_quantity() != input.value.is_some() {
        // A kind that reports how much happened must state it, and a kind that does not report a
        // quantity must not carry one.
        return Err(Error::InvalidField("value"));
    }
    if kind.requires_reference() != input.reference.is_some() {
        return Err(Error::InvalidField("reference"));
    }
    Ok(())
}

/// Validates who an event is about: the roles it must name, and the namespaces it may not alias.
fn validate_subjects(
    input: &SemanticHistoryEventInput,
    branch_id: &str,
    run_id: &str,
) -> Result<(), Error> {
    let mut seen: BTreeSet<SemanticHistorySubjectRole> = BTreeSet::new();
    for subject in &input.subjects {
        subject.validate("subject")?;
        if !seen.insert(subject.role) {
            // One end of an event is named once; a second subject for the same role would make the
            // event's actor or target ambiguous.
            return Err(Error::DuplicateSubjectRole(subject.role.name()));
        }
        if subject.identity == input.event_id
            || subject.identity == branch_id
            || subject.identity == run_id
        {
            // A live instance that shares a token with the event, the branch or the run is an
            // aliasing this boundary refuses rather than resolves.
            return Err(Error::IdentityNamespaceCollision(subject.role.name()));
        }
    }
    require_role(
        input,
        input.kind.requires_actor(),
        SemanticHistorySubjectRole::Actor,
    )?;
    require_role(
        input,
        input.kind.requires_target(),
        SemanticHistorySubjectRole::Target,
    )
}

/// Requires a role the kind names, and refuses a target the kind does not act on.
fn require_role(
    input: &SemanticHistoryEventInput,
    required: bool,
    role: SemanticHistorySubjectRole,
) -> Result<(), Error> {
    let present = input.subjects.iter().any(|subject| subject.role == role);
    if required && !present {
        return Err(Error::MissingSubject(role.name()));
    }
    if !required && present && role == SemanticHistorySubjectRole::Target {
        return Err(Error::UnexpectedSubjectRole(role.name()));
    }
    Ok(())
}
