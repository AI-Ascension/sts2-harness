// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use sha2::{Digest, Sha256};
use sts2_harness::context_control::{
    ContextBoundary, ContextControlStore, ContextDraft, ContextSourceDocument, ControlAuthority,
    DurableContextOwnerControlReceipt, DurableContextOwnerPublication,
    DurableContextOwnerPublicationWrite, DurableContextSourceSnapshot, DurableControlStoreError,
    StoreMode, context_source_digest,
};
use sts2_harness::management::{
    CONTEXT_OWNER_BINDING_SCHEMA_VERSION, CONTEXT_OWNER_PUBLICATION_LOOKUP_SCHEMA_VERSION,
    CONTEXT_OWNER_PUBLICATION_REQUEST_SCHEMA_VERSION, CONTEXT_OWNER_PUBLICATION_SCHEMA_VERSION,
    CONTEXT_OWNER_RECEIPT_SCHEMA_VERSION, ContextBindingContinuity, ContextBindingGrants,
    ContextBindingState, ContextControlCommand, ContextControlCommandKind, ContextControlReceipt,
    ContextOwnerBinding, ContextOwnerDraftPublicationLookupRequest,
    ContextOwnerDraftPublicationReceipt, ContextOwnerDraftPublicationRequest,
};

include!("context_control_publication_store/support.rs");
include!("context_control_publication_store/migration.rs");

#[test]
fn wrong_store_key_authenticates_before_a_missing_publication_lookup() {
    let (directory, mut store) = publication_store_fixture("wrong-key");
    publication_owner_state(&mut store);
    let request = publication_request("request.saved", 1, &publication_boundary());
    let saved = write_publication(&mut store, PUBLICATION_ACTOR, &request, 0)
        .expect("save one publication");
    assert_eq!(
        store
            .recover_draft_publication(
                PUBLICATION_OWNER,
                PUBLICATION_ACTOR,
                &publication_lookup(&request)
            )
            .expect("right key recovers saved receipt")
            .expect("receipt exists"),
        saved
    );
    drop(store);

    let missing = publication_request("request.missing", 1, &publication_boundary());
    let wrong_key_store = ContextControlStore::open(
        directory.join("control.sqlite3"),
        [0x6c; 32],
        PUBLICATION_RUN,
    )
    .expect("open without claiming the rightful writer");
    assert_eq!(
        wrong_key_store.recover_draft_publication(
            PUBLICATION_OWNER,
            PUBLICATION_ACTOR,
            &publication_lookup(&missing),
        ),
        Err(DurableControlStoreError::AuthenticationFailed),
        "wrong key cannot make a missing request look like an empty index"
    );
    drop(wrong_key_store);

    let rightful = ContextControlStore::open(
        directory.join("control.sqlite3"),
        PUBLICATION_KEY,
        PUBLICATION_RUN,
    )
    .expect("rightful key remains usable");
    assert_eq!(
        rightful
            .recover_draft_publication(
                PUBLICATION_OWNER,
                PUBLICATION_ACTOR,
                &publication_lookup(&missing),
            )
            .expect("authenticated missing request"),
        None
    );
    drop(rightful);
    cleanup_publication_store(directory);
}

#[test]
fn publication_failpoint_rolls_back_source_receipt_and_owner_cas_then_allows_retry() {
    let (directory, mut store) = publication_store_fixture("rollback");
    let original_state = publication_owner_state(&mut store);
    let request = publication_request("request.rollback", 1, &publication_boundary());
    let source_id = store
        .draft_publication_source_id(PUBLICATION_OWNER, PUBLICATION_ACTOR, &request)
        .expect("derive deterministic run-local source");
    store.set_failpoint(Some(
        sts2_harness::context_control::DurableStoreFailpoint::BeforeCommit,
    ));
    assert_eq!(
        write_publication(&mut store, PUBLICATION_ACTOR, &request, 0),
        Err(DurableControlStoreError::Failpoint)
    );
    let state = store
        .load_owner_context_state(PUBLICATION_OWNER)
        .expect("read state after rollback")
        .expect("owner state remains");
    assert_eq!(state.record_version, 1);
    assert_eq!(state.bytes, original_state);
    assert_eq!(
        store
            .load_context_source(&source_id, 1, &publication_source(&request.draft_id).digest)
            .expect("check source rollback"),
        None
    );
    assert_eq!(
        store
            .recover_draft_publication(
                PUBLICATION_OWNER,
                PUBLICATION_ACTOR,
                &publication_lookup(&request)
            )
            .expect("check receipt rollback"),
        None
    );
    assert!(
        store
            .list_draft_publications(PUBLICATION_OWNER, PUBLICATION_ACTOR)
            .expect("list after rollback")
            .is_empty()
    );

    let saved = write_publication(&mut store, PUBLICATION_ACTOR, &request, 0)
        .expect("retry commits complete write");
    assert_eq!(
        store
            .load_owner_context_state(PUBLICATION_OWNER)
            .expect("read committed owner state")
            .expect("owner state")
            .record_version,
        2
    );
    assert_eq!(
        store
            .recover_draft_publication(
                PUBLICATION_OWNER,
                PUBLICATION_ACTOR,
                &publication_lookup(&request)
            )
            .expect("recover committed retry")
            .expect("saved publication"),
        saved
    );
    drop(store);
    cleanup_publication_store(directory);
}

#[test]
fn configured_and_dynamic_sources_share_capacity_and_exact_replay_survives_full_capacity() {
    let (directory, mut store) = publication_store_fixture("capacity-15");
    publication_owner_state(&mut store);
    add_static_source(&mut store);
    let first = publication_request("request.capacity.first", 1, &publication_boundary());
    let saved = write_publication(&mut store, PUBLICATION_ACTOR, &first, 15)
        .expect("15 configured plus one dynamic");
    assert_eq!(
        write_publication(&mut store, PUBLICATION_ACTOR, &first, 15)
            .expect("exact retry at capacity"),
        saved
    );
    let second = publication_request("request.capacity.second", 2, &publication_boundary());
    assert_eq!(
        write_publication(&mut store, OTHER_PUBLICATION_ACTOR, &second, 15),
        Err(DurableControlStoreError::PublicationCapacity)
    );
    assert_eq!(
        store
            .load_owner_context_state(PUBLICATION_OWNER)
            .expect("state after cap refusal")
            .expect("state")
            .record_version,
        2
    );
    assert_eq!(
        store
            .list_draft_publications(PUBLICATION_OWNER, PUBLICATION_ACTOR)
            .expect("only one dynamic source")
            .len(),
        1
    );
    assert!(
        store
            .list_draft_publications(PUBLICATION_OWNER, OTHER_PUBLICATION_ACTOR)
            .expect("second actor has no capacity-refused publication")
            .is_empty()
    );
    drop(store);
    cleanup_publication_store(directory);

    let (directory, mut store) = publication_store_fixture("capacity-16");
    publication_owner_state(&mut store);
    add_static_source(&mut store);
    let first = publication_request("request.capacity.full", 1, &publication_boundary());
    assert_eq!(
        write_publication(&mut store, PUBLICATION_ACTOR, &first, 16),
        Err(DurableControlStoreError::PublicationCapacity),
        "16 configured sources leave no dynamic slot"
    );
    assert!(
        store
            .list_draft_publications(PUBLICATION_OWNER, PUBLICATION_ACTOR)
            .expect("no dynamic rows at full configured capacity")
            .is_empty()
    );
    drop(store);
    cleanup_publication_store(directory);
}

#[test]
fn independent_store_handles_racing_for_the_last_dynamic_slot_commit_at_most_one() {
    let (directory, store) = publication_store_fixture("capacity-race");
    let mut store = store;
    publication_owner_state(&mut store);
    add_static_source(&mut store);
    drop(store);
    let path = directory.join("control.sqlite3");
    let first_store = ContextControlStore::open(&path, PUBLICATION_KEY, PUBLICATION_RUN)
        .expect("open first unclaimed writer");
    let second_store = ContextControlStore::open(&path, PUBLICATION_KEY, PUBLICATION_RUN)
        .expect("open second unclaimed writer");
    let first = publication_request("request.race.first", 1, &publication_boundary());
    let second = publication_request("request.race.second", 1, &publication_boundary());
    let barrier = Arc::new(Barrier::new(3));
    let first_barrier = Arc::clone(&barrier);
    let first_handle = std::thread::spawn(move || {
        let mut store = first_store;
        first_barrier.wait();
        write_publication(&mut store, PUBLICATION_ACTOR, &first, 15)
    });
    let second_barrier = Arc::clone(&barrier);
    let second_handle = std::thread::spawn(move || {
        let mut store = second_store;
        second_barrier.wait();
        write_publication(&mut store, PUBLICATION_ACTOR, &second, 15)
    });
    barrier.wait();
    let first_result = first_handle.join().expect("first writer did not panic");
    let second_result = second_handle.join().expect("second writer did not panic");
    assert_ne!(first_result.is_ok(), second_result.is_ok());
    for error in [first_result.err(), second_result.err()]
        .into_iter()
        .flatten()
    {
        assert!(matches!(
            error,
            DurableControlStoreError::Fenced | DurableControlStoreError::PublicationCapacity
        ));
    }

    let reader =
        ContextControlStore::open(&path, PUBLICATION_KEY, PUBLICATION_RUN).expect("open verifier");
    assert_eq!(
        reader
            .list_draft_publications(PUBLICATION_OWNER, PUBLICATION_ACTOR)
            .expect("one final-slot winner")
            .len(),
        1
    );
    assert_eq!(
        reader
            .load_owner_context_state(PUBLICATION_OWNER)
            .expect("state after race")
            .expect("owner state")
            .record_version,
        2
    );
    drop(reader);

    let mut capacity_writer = ContextControlStore::open(&path, PUBLICATION_KEY, PUBLICATION_RUN)
        .expect("open a fresh writer after the race has settled");
    let at_capacity =
        publication_request("request.race.capacity-check", 2, &publication_boundary());
    assert_eq!(
        write_publication(
            &mut capacity_writer,
            OTHER_PUBLICATION_ACTOR,
            &at_capacity,
            15
        ),
        Err(DurableControlStoreError::PublicationCapacity),
        "with the owner-state version refreshed, the full configured-plus-dynamic bound is decisive"
    );
    assert_eq!(
        capacity_writer
            .load_owner_context_state(PUBLICATION_OWNER)
            .expect("read state after capacity refusal")
            .expect("owner state")
            .record_version,
        2,
        "capacity refusal does not consume another owner-state version"
    );
    assert_eq!(
        capacity_writer
            .list_draft_publications(PUBLICATION_OWNER, PUBLICATION_ACTOR)
            .expect("list after capacity refusal")
            .len(),
        1,
        "capacity refusal leaves the winner as the only dynamic publication"
    );
    assert!(
        capacity_writer
            .list_draft_publications(PUBLICATION_OWNER, OTHER_PUBLICATION_ACTOR)
            .expect("the second actor has no capacity-refused publication")
            .is_empty()
    );
    drop(capacity_writer);
    cleanup_publication_store(directory);
}
