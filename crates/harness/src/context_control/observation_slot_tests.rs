// SPDX-License-Identifier: MIT

//! Positive and negative fixtures for identity-bound latest-only observation slots (issue #112).
//!
//! Each test drives the public slot surface — admit, read, lineage, history, capture and
//! restore — rather than an internal helper, so the accepted and refused states are the ones a
//! production caller observes.

use super::*;
use crate::context_control::{ObservationSelectionSlot, ObservationSlotStore};

fn digest(seed: char) -> String {
    std::iter::repeat_n(seed, 64).collect()
}

fn boundary(
    state: &str,
    generation: u64,
    epoch: u64,
    gate: u64,
    observation: char,
) -> ContextBoundary {
    ContextBoundary {
        run_id: "run-1".to_owned(),
        episode_id: "episode-1".to_owned(),
        agent_id: "agent-1".to_owned(),
        state_id: state.to_owned(),
        generation,
        observation_sha256: digest(observation),
        catalog_sha256: digest('c'),
        adapter_revision: "adapter-1".to_owned(),
        model_revision: "model-1".to_owned(),
        configuration_sha256: digest('d'),
        output_schema_sha256: digest('e'),
        controller_epoch: epoch,
        gate_epoch: gate,
        control_version: 3,
    }
}

fn key() -> ObservationSlotKey {
    ObservationSlotKey::new("run-1", "episode-1", "agent-1", "host-observe", "adapter-1")
}

fn candidate(
    boundary: ContextBoundary,
    catalog_generation: u64,
    pinned: bool,
) -> ObservationSlotAdmission {
    ObservationSlotAdmission {
        key: key(),
        boundary,
        catalog_generation,
        admitted_at: "2026-09-23T10:00:00Z".to_owned(),
        expires_at: "2026-09-23T11:00:00Z".to_owned(),
        pinned,
    }
}

const NOW: &str = "2026-09-23T10:30:00Z";

fn slot() -> ObservationSelectionSlot {
    ObservationSelectionSlot::new(key()).unwrap_or_else(|_| unreachable!())
}

#[test]
fn a_slot_at_lineage_capacity_refuses_further_admissions_with_capacity() {
    let mut slot = slot_at_lineage_capacity();
    let next = u64::try_from(MAX_SLOT_LINEAGE).unwrap_or_else(|_| unreachable!()) + 1;

    assert_eq!(
        slot.admit(&generation_candidate(next), NOW),
        Err(ObservationSlotRefusal::Capacity)
    );
    assert_eq!(
        slot.lineage().len(),
        MAX_SLOT_LINEAGE,
        "a refused admission must not extend the lineage"
    );
}

/// Fills a slot's lineage to exactly `MAX_SLOT_LINEAGE` admitted values.
///
/// The first admission is `Inserted` and each later one supersedes, so the
/// lineage grows by one per call. Ordering is `(controller_epoch, gate_epoch)`,
/// so both epochs rise with the generation to keep each candidate strictly
/// newer than the effective value.
fn slot_at_lineage_capacity() -> ObservationSelectionSlot {
    let mut slot = slot();
    for index in 0..MAX_SLOT_LINEAGE {
        let generation = u64::try_from(index).unwrap_or_else(|_| unreachable!()) + 1;
        assert!(
            slot.admit(&generation_candidate(generation), NOW).is_ok(),
            "lineage entry {index} must be admitted"
        );
    }
    assert_eq!(slot.lineage().len(), MAX_SLOT_LINEAGE);
    slot
}

/// One candidate for a generation, with a distinct but valid lowercase-hex
/// observation digest per generation.
fn generation_candidate(generation: u64) -> ObservationSlotAdmission {
    let state = format!("s{generation}");
    // `valid_digest` accepts only lowercase hex, so the digest varies over
    // a-f rather than over the whole alphabet.
    let observation =
        char::from(b'a' + u8::try_from(generation % 6).unwrap_or_else(|_| unreachable!()));
    candidate(
        boundary(&state, generation, generation, generation, observation),
        generation,
        false,
    )
}

/// `Capacity` and `InvalidIdentity` are different refusals and must stay so.
///
/// A caller distinguishes "this slot is full" from "this candidate is
/// malformed" by the variant, so collapsing them is a real behaviour change
/// even though both are errors.
#[test]
fn lineage_capacity_is_never_reported_as_a_different_refusal() {
    let mut slot = slot_at_lineage_capacity();
    let next = u64::try_from(MAX_SLOT_LINEAGE).unwrap_or_else(|_| unreachable!()) + 1;

    assert_eq!(
        slot.admit(&generation_candidate(next), NOW),
        Err(ObservationSlotRefusal::Capacity),
        "a slot at capacity must refuse with Capacity"
    );
    assert_ne!(
        ObservationSlotRefusal::Capacity,
        ObservationSlotRefusal::InvalidIdentity
    );
}

/// The last admission up to the bound still succeeds, so the refusal above is
/// a real boundary and not an off-by-one that refuses one entry early.
#[test]
fn the_final_admission_up_to_lineage_capacity_still_succeeds() {
    let mut slot = slot();
    for index in 0..MAX_SLOT_LINEAGE - 1 {
        let generation = u64::try_from(index).unwrap_or_else(|_| unreachable!()) + 1;
        assert!(slot.admit(&generation_candidate(generation), NOW).is_ok());
    }
    assert_eq!(slot.lineage().len(), MAX_SLOT_LINEAGE - 1);

    let last = u64::try_from(MAX_SLOT_LINEAGE).unwrap_or_else(|_| unreachable!());
    assert!(
        slot.admit(&generation_candidate(last), NOW).is_ok(),
        "the entry that reaches the bound must be admitted"
    );
    assert_eq!(slot.lineage().len(), MAX_SLOT_LINEAGE);
}

/// A malformed candidate is refused as `InvalidIdentity`, which stays
/// distinguishable from `Capacity` on a slot that has never filled.
#[test]
fn a_malformed_expiry_is_refused_as_invalid_identity_not_capacity() {
    let mut slot = slot();
    let mut malformed = candidate(boundary("s1", 1, 1, 1, 'a'), 1, false);
    malformed.expires_at = "not-a-timestamp".to_owned();

    assert_eq!(
        slot.admit(&malformed, NOW),
        Err(ObservationSlotRefusal::InvalidIdentity)
    );
    assert!(slot.lineage().is_empty());
}

#[test]
fn successive_invocations_keep_only_the_latest_with_deterministic_lineage() {
    let mut slot = slot();
    assert_eq!(
        slot.admit(&candidate(boundary("s1", 1, 1, 1, 'a'), 1, false), NOW),
        Ok(SlotAdmission::Inserted { approval_fence: 1 })
    );
    assert_eq!(
        slot.admit(&candidate(boundary("s2", 2, 1, 2, 'b'), 2, false), NOW),
        Ok(SlotAdmission::Superseded { approval_fence: 2 })
    );
    assert_eq!(
        slot.admit(&candidate(boundary("s3", 3, 2, 1, 'f'), 3, false), NOW),
        Ok(SlotAdmission::Superseded { approval_fence: 3 })
    );
    let effective = slot.read(NOW).unwrap_or_else(|_| unreachable!());
    assert_eq!(effective.boundary.state_id, "s3");
    assert_eq!(slot.lineage().len(), 3);
    assert_eq!(slot.lineage()[0].reason, SupersessionReason::Initial);
    assert_eq!(
        slot.lineage()[2].reason,
        SupersessionReason::NewerGeneration
    );
    assert_eq!(slot.lineage()[2].superseded_state_id.as_deref(), Some("s2"));
    assert_eq!(slot.lineage()[2].admitted_state_id, "s3");
    assert_eq!(slot.lineage()[2].approval_fence, 3);
    assert_eq!(slot.approval_fence(), 3);
}

#[test]
fn out_of_order_partial_and_foreign_candidates_cannot_replace_the_slot() {
    let mut slot = slot();
    assert!(
        slot.admit(&candidate(boundary("s2", 2, 1, 2, 'b'), 2, false), NOW)
            .is_ok()
    );
    assert_eq!(
        slot.admit(&candidate(boundary("s0", 1, 1, 1, 'a'), 1, false), NOW),
        Err(ObservationSlotRefusal::OutOfOrder)
    );
    let mut foreign = candidate(boundary("s9", 9, 3, 1, '9'), 9, false);
    foreign.key =
        ObservationSlotKey::new("run-2", "episode-1", "agent-1", "host-observe", "adapter-1");
    assert_eq!(
        slot.admit(&foreign, NOW),
        Err(ObservationSlotRefusal::ForeignIdentity)
    );
    let mut other_projection = candidate(boundary("s9", 9, 3, 1, '9'), 9, false);
    other_projection.boundary.adapter_revision = "adapter-2".to_owned();
    assert_eq!(
        slot.admit(&other_projection, NOW),
        Err(ObservationSlotRefusal::ForeignIdentity)
    );
    let mut partial = candidate(boundary("s9", 9, 3, 1, '9'), 9, false);
    partial.boundary.observation_sha256 = String::new();
    assert_eq!(
        slot.admit(&partial, NOW),
        Err(ObservationSlotRefusal::PartialStateCatalog)
    );
    assert_eq!(
        slot.read(NOW)
            .unwrap_or_else(|_| unreachable!())
            .boundary
            .state_id,
        "s2"
    );
}

#[test]
fn conflicting_same_sequence_is_refused_but_an_identical_replay_is_unchanged() {
    let mut slot = slot();
    let effective = candidate(boundary("s2", 2, 1, 2, 'b'), 2, false);
    assert!(slot.admit(&effective, NOW).is_ok());
    assert_eq!(slot.admit(&effective, NOW), Ok(SlotAdmission::Unchanged));
    assert_eq!(
        slot.admit(&candidate(boundary("sX", 2, 1, 2, 'b'), 2, false), NOW),
        Err(ObservationSlotRefusal::ConflictingSameSequence)
    );
    assert_eq!(slot.lineage().len(), 1);
}

#[test]
fn state_catalog_mismatch_is_refused_and_expired_candidates_never_land() {
    let mut slot = slot();
    assert_eq!(
        slot.admit(&candidate(boundary("s1", 1, 1, 1, 'a'), 2, false), NOW),
        Err(ObservationSlotRefusal::StateCatalogMismatch)
    );
    assert_eq!(
        slot.admit(&candidate(boundary("s1", 1, 1, 1, 'a'), 0, false), NOW),
        Err(ObservationSlotRefusal::PartialStateCatalog)
    );
    let mut expired = candidate(boundary("s1", 1, 1, 1, 'a'), 1, false);
    expired.admitted_at = "2026-09-23T09:00:00Z".to_owned();
    expired.expires_at = "2026-09-23T09:30:00Z".to_owned();
    assert_eq!(
        slot.admit(&expired, NOW),
        Err(ObservationSlotRefusal::Expired)
    );
    assert!(slot.read(NOW).is_err());
}

#[test]
fn required_refresh_failure_blocks_reads_instead_of_serving_stale_state() {
    let mut store = ObservationSlotStore::new();
    let first = candidate(boundary("s1", 1, 1, 1, 'a'), 1, false);
    assert!(store.admit(&first, NOW).is_ok());
    assert!(store.slot(&key()).is_some());
    store
        .record_refresh_failure(&key())
        .unwrap_or_else(|_| unreachable!());
    assert_eq!(
        store
            .slot(&key())
            .unwrap_or_else(|| unreachable!())
            .read(NOW),
        Err(ObservationSlotRefusal::RefreshRequired)
    );
    let second = candidate(boundary("s2", 2, 1, 2, 'b'), 2, false);
    assert!(store.admit(&second, NOW).is_ok());
    assert_eq!(
        store
            .slot(&key())
            .unwrap_or_else(|| unreachable!())
            .read(NOW)
            .unwrap_or_else(|_| unreachable!())
            .boundary
            .state_id,
        "s2"
    );
}

#[test]
fn restart_keeps_one_effective_value_pinned_history_and_invalidates_old_approvals() {
    let mut store = ObservationSlotStore::new();
    let pinned = candidate(boundary("s1", 1, 1, 1, 'a'), 1, true);
    assert_eq!(
        store.admit(&pinned, NOW),
        Ok(SlotAdmission::Inserted { approval_fence: 1 })
    );
    let newer = candidate(boundary("s2", 2, 1, 2, 'b'), 2, false);
    assert_eq!(
        store.admit(&newer, NOW),
        Ok(SlotAdmission::Superseded { approval_fence: 2 })
    );
    let image = store.capture().unwrap_or_else(|_| unreachable!());
    let restored = image.restore().unwrap_or_else(|_| unreachable!());
    let effective = restored.effective_selections();
    assert_eq!(effective.len(), 1);
    assert_eq!(effective[0].boundary.state_id, "s2");
    let restored_slot = restored.slot(&key()).unwrap_or_else(|| unreachable!());
    assert_eq!(restored_slot.history().len(), 1);
    assert!(restored_slot.history()[0].pinned);
    assert_eq!(restored_slot.history()[0].boundary.state_id, "s1");
    assert_eq!(
        restored_slot.history()[0].boundary.observation_sha256,
        digest('a')
    );
    assert!(!restored_slot.approval_current(1));
    assert!(restored_slot.approval_current(2));
}

#[test]
fn a_different_identity_gets_its_own_slot_and_never_overwrites_another() {
    let mut store = ObservationSlotStore::new();
    let a = candidate(boundary("a-1", 1, 1, 1, 'a'), 1, false);
    assert_eq!(
        store.admit(&a, NOW),
        Ok(SlotAdmission::Inserted { approval_fence: 1 })
    );
    let mut b_boundary = boundary("b-1", 1, 1, 1, 'b');
    b_boundary.agent_id = "agent-2".to_owned();
    let mut b = candidate(b_boundary, 1, false);
    b.key = ObservationSlotKey::new("run-1", "episode-1", "agent-2", "host-observe", "adapter-1");
    assert_eq!(
        store.admit(&b, NOW),
        Ok(SlotAdmission::Inserted { approval_fence: 1 })
    );
    assert_eq!(store.len(), 2);
    let a_slot = store.slot(&key()).unwrap_or_else(|| unreachable!());
    assert_eq!(
        a_slot
            .read(NOW)
            .unwrap_or_else(|_| unreachable!())
            .boundary
            .state_id,
        "a-1"
    );
    assert_eq!(
        a_slot
            .read(NOW)
            .unwrap_or_else(|_| unreachable!())
            .boundary
            .agent_id,
        "agent-1"
    );
    assert_eq!(a_slot.approval_fence(), 1);
}
