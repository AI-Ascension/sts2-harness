// SPDX-License-Identifier: MIT

//! AC3 (durable half) — persisted lifetime counters and identities survive a process restart, and a
//! crash on either side of the durable write cannot hand a consumed slot back (issue #111).
//!
//! Synthetic fixtures only; no provider, host, game, or wall clock is contacted. Each case opens a
//! real SQLite file in a fresh temporary directory and reopens it as a new handle, which is the
//! closest in-process stand-in for a restart.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

#[path = "support/context_lifetime.rs"]
mod fixture;

use std::fs;
use std::path::PathBuf;

use fixture::{INSIDE, invocation, ledger_with, next_n_scope, scope_id};
use sts2_harness::context_control::{
    ContextBoundary, ContextControlStore, ControlAuthority, DurableControlStoreError,
    DurableLifetimeState, DurableStoreFailpoint, StoreMode,
};

/// Must match the fixture owner's run identity, because the durable image refuses a scope whose
/// owner disagrees with the run it is stored under.
const RUN_ID: &str = "run-1";
const KEY: [u8; 32] = [0x6d; 32];

fn boundary() -> ContextBoundary {
    ContextBoundary {
        run_id: RUN_ID.into(),
        episode_id: "episode-1".into(),
        agent_id: "agent-1".into(),
        state_id: "state-1".into(),
        generation: 1,
        observation_sha256: "a".repeat(64),
        catalog_sha256: "b".repeat(64),
        adapter_revision: "adapter-1".into(),
        model_revision: "model-1".into(),
        configuration_sha256: "c".repeat(64),
        output_schema_sha256: "d".repeat(64),
        controller_epoch: 1,
        gate_epoch: 1,
        control_version: 1,
    }
}

fn store_path() -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("context-lifetime-durable-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&directory).expect("create fixture directory");
    directory.join("control.sqlite3")
}

fn open_store(path: &PathBuf) -> ContextControlStore {
    let authority = ControlAuthority::new(boundary(), "revision-1");
    ContextControlStore::create(path, KEY, RUN_ID, &authority, StoreMode::Enabled)
        .expect("create control store")
}

/// AC3: the consumed counters and the admitted identities survive a restart and reload unchanged.
#[test]
fn counters_and_identities_survive_a_restart() {
    let path = store_path();
    let mut ledger = ledger_with(next_n_scope(3));
    ledger
        .admit(scope_id(), &invocation("invocation-1"), INSIDE, None)
        .expect("admission succeeds");
    ledger
        .admit(scope_id(), &invocation("invocation-2"), INSIDE, None)
        .expect("admission succeeds");
    let before = ledger
        .preview(scope_id(), INSIDE)
        .expect("preview succeeds");
    assert_eq!(before.consumed, 2);
    assert_eq!(before.remaining, 1);

    let mut store = open_store(&path);
    store
        .persist_lifetime(&ledger)
        .expect("lifetime state persists");
    drop(store);

    // A fresh handle over the same file is the restart.
    let reopened = ContextControlStore::open(&path, KEY, RUN_ID).expect("reopen control store");
    let state = reopened
        .load_lifetime()
        .expect("lifetime state loads")
        .expect("lifetime state was persisted");
    assert_eq!(state.run_id, RUN_ID);
    assert_eq!(state.manifest_count, 2);
    assert_eq!(state.scope_count, 1);

    let restored = state.restore().expect("the ledger restores");
    let after = restored
        .preview(scope_id(), INSIDE)
        .expect("preview succeeds");
    assert_eq!(after.consumed, before.consumed, "the count is preserved");
    assert_eq!(after.remaining, before.remaining, "the window is preserved");
    assert_eq!(after.scope_digest, before.scope_digest);
    assert_eq!(restored.manifests().len(), 2);
    assert_eq!(
        restored.manifests()[0].invocation_id,
        "invocation-1",
        "identities are preserved in admission order"
    );
    assert_eq!(restored.manifests()[1].invocation_id, "invocation-2");
}

/// AC3: a restart cannot resurrect applicability. After reloading a fully consumed window, the next
/// logical invocation is still refused.
#[test]
fn a_restart_cannot_resurrect_a_consumed_window() {
    let path = store_path();
    let mut ledger = ledger_with(next_n_scope(2));
    ledger
        .admit(scope_id(), &invocation("invocation-1"), INSIDE, None)
        .expect("admission succeeds");
    ledger
        .admit(scope_id(), &invocation("invocation-2"), INSIDE, None)
        .expect("admission succeeds");
    let mut store = open_store(&path);
    store
        .persist_lifetime(&ledger)
        .expect("lifetime state persists");
    drop(store);

    let reopened = ContextControlStore::open(&path, KEY, RUN_ID).expect("reopen control store");
    let mut restored = reopened
        .load_lifetime()
        .expect("load succeeds")
        .expect("state exists")
        .restore()
        .expect("the ledger restores");

    let refused = restored.admit(scope_id(), &invocation("invocation-3"), INSIDE, None);
    assert!(
        matches!(
            refused,
            Err(sts2_harness::context_control::ContextLifetimeError::Exhausted { .. })
        ),
        "a reloaded exhausted window must not admit, got {refused:?}"
    );
    // The already-admitted invocations are still answered idempotently after the reload.
    let replay = restored
        .admit(scope_id(), &invocation("invocation-1"), INSIDE, None)
        .expect("an admitted invocation is answered from the reloaded history");
    assert_eq!(replay.ordinal, 1);
}

/// AC3: a held (possible-dispatch) invocation stays consumed and held across a restart, so a
/// dispatch that may have happened is never silently handed back.
#[test]
fn a_held_invocation_stays_consumed_across_a_restart() {
    let path = store_path();
    let mut ledger = ledger_with(next_n_scope(3));
    let _ = ledger.admit(
        scope_id(),
        &invocation("invocation-1"),
        INSIDE,
        Some(sts2_harness::context_control::LifetimeFailpoint::AfterDurableAdmission),
    );
    assert_eq!(
        ledger.preview(scope_id(), INSIDE).expect("preview").held,
        vec!["invocation-1".to_owned()]
    );
    let mut store = open_store(&path);
    store
        .persist_lifetime(&ledger)
        .expect("lifetime state persists");
    drop(store);

    let reopened = ContextControlStore::open(&path, KEY, RUN_ID).expect("reopen control store");
    let restored = reopened
        .load_lifetime()
        .expect("load succeeds")
        .expect("state exists")
        .restore()
        .expect("the ledger restores");
    let preview = restored
        .preview(scope_id(), INSIDE)
        .expect("preview succeeds");
    assert_eq!(preview.consumed, 1, "the slot is still consumed");
    assert_eq!(
        preview.held,
        vec!["invocation-1".to_owned()],
        "the invocation is still held for reconciliation"
    );
}

/// AC3: a settlement committed before the restart is still in force afterwards, and a released slot
/// is restored exactly once rather than re-consumed.
#[test]
fn a_settlement_survives_a_restart() {
    let path = store_path();
    let mut ledger = ledger_with(next_n_scope(3));
    ledger
        .admit(scope_id(), &invocation("invocation-1"), INSIDE, None)
        .expect("admission succeeds");
    ledger
        .reconcile(
            "invocation-1",
            sts2_harness::context_control::DispatchSettlement::Released,
        )
        .expect("reconciliation succeeds");
    let mut store = open_store(&path);
    store
        .persist_lifetime(&ledger)
        .expect("lifetime state persists");
    drop(store);

    let reopened = ContextControlStore::open(&path, KEY, RUN_ID).expect("reopen control store");
    let restored = reopened
        .load_lifetime()
        .expect("load succeeds")
        .expect("state exists")
        .restore()
        .expect("the ledger restores");
    let preview = restored
        .preview(scope_id(), INSIDE)
        .expect("preview succeeds");
    assert_eq!(preview.consumed, 0, "the released slot stays released");
    assert_eq!(preview.remaining, 3);
    assert!(preview.held.is_empty());
    assert_eq!(
        restored.manifests()[0].settlement,
        sts2_harness::context_control::DispatchSettlement::Released,
        "the settlement is not rewritten by the reload"
    );
}

/// AC3: a crash before the durable write leaves the previously committed counters intact.
#[test]
fn a_crash_before_the_lifetime_write_preserves_the_previous_counters() {
    let path = store_path();
    let mut ledger = ledger_with(next_n_scope(3));
    ledger
        .admit(scope_id(), &invocation("invocation-1"), INSIDE, None)
        .expect("admission succeeds");
    let mut store = open_store(&path);
    store
        .persist_lifetime(&ledger)
        .expect("first commit succeeds");

    // A second admission is committed to the ledger but stopped before it reaches the store.
    ledger
        .admit(scope_id(), &invocation("invocation-2"), INSIDE, None)
        .expect("admission succeeds");
    store.set_failpoint(Some(DurableStoreFailpoint::BeforeJournalWrite));
    assert_eq!(
        store.persist_lifetime(&ledger),
        Err(DurableControlStoreError::Failpoint)
    );
    drop(store);

    let reopened = ContextControlStore::open(&path, KEY, RUN_ID).expect("reopen control store");
    let restored = reopened
        .load_lifetime()
        .expect("load succeeds")
        .expect("state exists")
        .restore()
        .expect("the ledger restores");
    assert_eq!(
        restored
            .preview(scope_id(), INSIDE)
            .expect("preview")
            .consumed,
        1,
        "the failed write must not have advanced the committed count"
    );
}

/// AC3: a crash at the commit boundary likewise leaves the previously committed counters intact,
/// so the window never advances on a write that did not become durable.
#[test]
fn a_crash_before_commit_preserves_the_previous_counters() {
    let path = store_path();
    let mut ledger = ledger_with(next_n_scope(3));
    ledger
        .admit(scope_id(), &invocation("invocation-1"), INSIDE, None)
        .expect("admission succeeds");
    let mut store = open_store(&path);
    store
        .persist_lifetime(&ledger)
        .expect("first commit succeeds");

    ledger
        .admit(scope_id(), &invocation("invocation-2"), INSIDE, None)
        .expect("admission succeeds");
    store.set_failpoint(Some(DurableStoreFailpoint::BeforeCommit));
    assert_eq!(
        store.persist_lifetime(&ledger),
        Err(DurableControlStoreError::Failpoint)
    );
    drop(store);

    let reopened = ContextControlStore::open(&path, KEY, RUN_ID).expect("reopen control store");
    let restored = reopened
        .load_lifetime()
        .expect("load succeeds")
        .expect("state exists")
        .restore()
        .expect("the ledger restores");
    assert_eq!(
        restored
            .preview(scope_id(), INSIDE)
            .expect("preview")
            .consumed,
        1
    );
    assert_eq!(restored.manifests().len(), 1);
}

/// AC3: a run that never persisted lifetime state reports "no window" rather than an empty window,
/// so a caller cannot mistake absence for a fresh scope.
#[test]
fn a_run_without_persisted_state_reports_absence() {
    let path = store_path();
    let store = open_store(&path);
    assert_eq!(store.load_lifetime().expect("load succeeds"), None);
}

/// AC3: the persisted envelope is tamper-evident. Rewriting the stored bytes is refused rather than
/// silently trusted.
#[test]
fn a_tampered_lifetime_envelope_is_refused() {
    let path = store_path();
    let mut ledger = ledger_with(next_n_scope(2));
    ledger
        .admit(scope_id(), &invocation("invocation-1"), INSIDE, None)
        .expect("admission succeeds");
    let mut store = open_store(&path);
    store
        .persist_lifetime(&ledger)
        .expect("lifetime state persists");
    drop(store);

    // Corrupt the stored envelope bytes directly.
    let connection = rusqlite::Connection::open(&path).expect("open raw fixture database");
    let mut envelope: Vec<u8> = connection
        .query_row(
            "SELECT envelope FROM context_control_lifetime WHERE run_id = ?1",
            [RUN_ID],
            |row| row.get(0),
        )
        .expect("the row exists");
    let last = envelope.len() - 1;
    envelope[last] ^= 0xff;
    connection
        .execute(
            "UPDATE context_control_lifetime SET envelope = ?1 WHERE run_id = ?2",
            rusqlite::params![envelope, RUN_ID],
        )
        .expect("tamper with the stored envelope");
    drop(connection);

    let reopened = ContextControlStore::open(&path, KEY, RUN_ID).expect("reopen control store");
    assert!(
        reopened.load_lifetime().is_err(),
        "a tampered envelope must not be trusted"
    );
}

/// AC3: the durable image refuses a manifest that claims another run's ownership, so a reloaded
/// window cannot be widened by injecting a foreign identity.
#[test]
fn a_foreign_identity_is_refused_by_the_durable_image() {
    let ledger = ledger_with(next_n_scope(2));
    let mut state = DurableLifetimeState::capture(RUN_ID, &ledger);
    state.run_id = "run.other".to_owned();
    assert!(
        state.validate().is_err(),
        "an image whose run disagrees with its scope owner is refused"
    );
}
