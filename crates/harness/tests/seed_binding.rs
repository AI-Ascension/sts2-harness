// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use sts2_harness::seed_binding::{
    MAX_SEED_BYTES, RecordingTransport, SeedBindingError, SeedMode, SeedRecord, SeedSource,
    SeedStore, SentStart, SetupRequest, canonicalize_seed, dispatch_start, resolve_seed,
};

#[derive(Default)]
struct MemoryStore {
    records: Vec<SeedRecord>,
    persist_failures: usize,
}

impl SeedStore for MemoryStore {
    fn load(&self, operation_id: &str) -> Result<Option<SeedRecord>, SeedBindingError> {
        Ok(self
            .records
            .iter()
            .find(|record| record.operation_id == operation_id)
            .cloned())
    }

    fn persist(&mut self, record: &SeedRecord) -> Result<(), SeedBindingError> {
        if self.persist_failures > 0 {
            self.persist_failures -= 1;
            return Err(SeedBindingError::PersistenceFailed);
        }
        self.records.push(record.clone());
        Ok(())
    }
}

#[derive(Default)]
struct CountingSource {
    draws: usize,
}

impl SeedSource for CountingSource {
    fn draw(&mut self) -> Result<String, SeedBindingError> {
        self.draws += 1;
        Ok(format!("generated-{}", self.draws))
    }
}

fn request(mode: SeedMode) -> SetupRequest {
    SetupRequest {
        operation_id: String::from("op-1"),
        instance_id: String::from("instance.1"),
        observed_instance_id: String::from("instance.1"),
        baseline_digest: "a".repeat(64),
        observed_baseline_digest: "a".repeat(64),
        lease_id: String::from("lease.1"),
        observed_lease_id: String::from("lease.1"),
        setup: String::from("standard.ironclad.asc0"),
        supported_setups: vec![
            String::from("standard.ironclad.asc0"),
            String::from("daily.ironclad"),
        ],
        mode,
    }
}

#[test]
fn explicit_seed_is_canonical_and_draws_nothing() {
    let mut store = MemoryStore::default();
    let mut source = CountingSource::default();
    let resolved = resolve_seed(
        &request(SeedMode::Explicit(String::from("seed.alpha"))),
        &mut store,
        &mut source,
    )
    .unwrap();
    assert!(!resolved.reused);
    assert_eq!(
        resolved.record.requested_seed.as_deref(),
        Some("seed.alpha")
    );
    assert_eq!(resolved.record.effective_seed, "seed.alpha");
    assert_eq!(source.draws, 0);
    assert_eq!(store.records.len(), 1);
}

#[test]
fn generate_once_draws_exactly_once_and_a_duplicate_reuses_it() {
    let mut store = MemoryStore::default();
    let mut source = CountingSource::default();
    let first = resolve_seed(&request(SeedMode::GenerateOnce), &mut store, &mut source).unwrap();
    let second = resolve_seed(&request(SeedMode::GenerateOnce), &mut store, &mut source).unwrap();
    assert_eq!(source.draws, 1);
    assert!(!first.reused);
    assert!(second.reused);
    assert_eq!(first.record, second.record);
    assert_eq!(second.record.requested_seed, None);
    assert_eq!(store.records.len(), 1);
}

#[test]
fn generate_once_reuses_the_persisted_seed_across_restart() {
    let mut store = MemoryStore::default();
    let mut first_source = CountingSource::default();
    let first = resolve_seed(
        &request(SeedMode::GenerateOnce),
        &mut store,
        &mut first_source,
    )
    .unwrap();
    // A restarted process resolves the same operation through the same store.
    let mut restarted_source = CountingSource::default();
    let restarted = resolve_seed(
        &request(SeedMode::GenerateOnce),
        &mut store,
        &mut restarted_source,
    )
    .unwrap();
    assert_eq!(first_source.draws, 1);
    assert_eq!(restarted_source.draws, 0);
    assert!(restarted.reused);
    assert_eq!(restarted.record.effective_seed, first.record.effective_seed);
}

#[test]
fn a_lost_response_retry_does_not_redraw() {
    let mut store = MemoryStore::default();
    let mut source = CountingSource::default();
    let started = resolve_seed(&request(SeedMode::GenerateOnce), &mut store, &mut source).unwrap();
    // The start reply is lost; the caller resolves again before retrying.
    let retried = resolve_seed(&request(SeedMode::GenerateOnce), &mut store, &mut source).unwrap();
    assert_eq!(source.draws, 1);
    assert_eq!(retried.record, started.record);
    let mut transport = RecordingTransport::default();
    let sent = dispatch_start(&retried.record, &mut transport).unwrap();
    assert_eq!(sent.effective_seed, started.record.effective_seed);
}

#[test]
fn a_persist_failure_fails_closed_before_any_start() {
    let mut store = MemoryStore {
        persist_failures: 1,
        ..MemoryStore::default()
    };
    let mut source = CountingSource::default();
    match resolve_seed(&request(SeedMode::GenerateOnce), &mut store, &mut source) {
        Err(SeedBindingError::PersistenceFailed) => {}
        other => panic!("expected a persistence failure, got {other:?}"),
    }
    assert!(store.records.is_empty());
    assert_eq!(source.draws, 1);
    // A later attempt is a new logical attempt: the run never mutated on failure.
    let recovered =
        resolve_seed(&request(SeedMode::GenerateOnce), &mut store, &mut source).unwrap();
    assert_eq!(source.draws, 2);
    assert_eq!(store.records.len(), 1);
    assert_eq!(recovered.record.effective_seed, "generated-2");
}

#[test]
fn mismatched_setup_identity_rejects_before_any_draw() {
    assert_rejected_before_draw(
        |request| request.observed_instance_id = String::from("instance.2"),
        SeedBindingError::InstanceMismatch,
    );
    assert_rejected_before_draw(
        |request| request.observed_baseline_digest = "b".repeat(64),
        SeedBindingError::BaselineMismatch,
    );
    assert_rejected_before_draw(
        |request| request.observed_lease_id = String::from("lease.2"),
        SeedBindingError::LeaseMismatch,
    );
    assert_rejected_before_draw(
        |request| request.setup = String::from("daily.ironclad.unsupported"),
        SeedBindingError::UnsupportedSetup,
    );
}

fn assert_rejected_before_draw(mutate: fn(&mut SetupRequest), expected: SeedBindingError) {
    let mut store = MemoryStore::default();
    let mut source = CountingSource::default();
    let mut request = request(SeedMode::GenerateOnce);
    mutate(&mut request);
    assert_eq!(
        resolve_seed(&request, &mut store, &mut source).unwrap_err(),
        expected
    );
    assert_eq!(source.draws, 0);
    assert!(store.records.is_empty());
}

#[test]
fn a_conflicting_explicit_seed_is_refused_against_the_persisted_record() {
    let mut store = MemoryStore::default();
    let mut source = CountingSource::default();
    resolve_seed(
        &request(SeedMode::Explicit(String::from("seed.alpha"))),
        &mut store,
        &mut source,
    )
    .unwrap();
    let conflict = resolve_seed(
        &request(SeedMode::Explicit(String::from("seed.beta"))),
        &mut store,
        &mut source,
    );
    assert_eq!(
        conflict.unwrap_err(),
        SeedBindingError::ConfigurationConflict
    );
    assert_eq!(source.draws, 0);
    assert_eq!(store.records.len(), 1);
    assert_eq!(store.records[0].effective_seed, "seed.alpha");
}

#[test]
fn generate_once_conflicts_with_an_explicit_persisted_record() {
    let mut store = MemoryStore::default();
    let mut source = CountingSource::default();
    resolve_seed(
        &request(SeedMode::Explicit(String::from("seed.alpha"))),
        &mut store,
        &mut source,
    )
    .unwrap();
    let conflict = resolve_seed(&request(SeedMode::GenerateOnce), &mut store, &mut source);
    assert_eq!(
        conflict.unwrap_err(),
        SeedBindingError::ConfigurationConflict
    );
    assert_eq!(source.draws, 0);
}

#[test]
fn explicit_conflicts_with_a_generated_persisted_record() {
    let mut store = MemoryStore::default();
    let mut source = CountingSource::default();
    resolve_seed(&request(SeedMode::GenerateOnce), &mut store, &mut source).unwrap();
    let conflict = resolve_seed(
        &request(SeedMode::Explicit(String::from("seed.alpha"))),
        &mut store,
        &mut source,
    );
    assert_eq!(
        conflict.unwrap_err(),
        SeedBindingError::ConfigurationConflict
    );
    assert_eq!(source.draws, 1);
}

#[test]
fn canonical_seed_enforces_utf8_byte_bounds() {
    let two_byte = "\u{e9}".repeat(32);
    assert_eq!(two_byte.len(), MAX_SEED_BYTES);
    assert_eq!(canonicalize_seed(&two_byte).unwrap(), two_byte);
    let mut two_byte_over = two_byte.clone();
    two_byte_over.push('a');
    assert_eq!(two_byte_over.len(), MAX_SEED_BYTES + 1);
    assert_eq!(
        canonicalize_seed(&two_byte_over).unwrap_err(),
        SeedBindingError::SeedTooLarge
    );

    let four_byte = "\u{1f600}".repeat(16);
    assert_eq!(four_byte.len(), MAX_SEED_BYTES);
    assert_eq!(canonicalize_seed(&four_byte).unwrap(), four_byte);
    let mut four_byte_over = four_byte.clone();
    four_byte_over.push('a');
    assert_eq!(
        canonicalize_seed(&four_byte_over).unwrap_err(),
        SeedBindingError::SeedTooLarge
    );
}

#[test]
fn canonical_seed_rejects_empty_control_and_padding() {
    assert_eq!(
        canonicalize_seed("").unwrap_err(),
        SeedBindingError::EmptySeed
    );
    assert_eq!(
        canonicalize_seed("bad\nseed").unwrap_err(),
        SeedBindingError::SeedControlCharacter
    );
    assert_eq!(
        canonicalize_seed(" padded ").unwrap_err(),
        SeedBindingError::SeedNotCanonical
    );
}

#[test]
fn the_recording_transport_sends_the_persisted_seed_and_operation_unchanged() {
    let mut store = MemoryStore::default();
    let mut source = CountingSource::default();
    let resolved = resolve_seed(&request(SeedMode::GenerateOnce), &mut store, &mut source).unwrap();
    let mut transport = RecordingTransport::default();
    let sent: SentStart = dispatch_start(&resolved.record, &mut transport).unwrap();
    assert_eq!(
        sent,
        SentStart {
            operation_id: resolved.record.operation_id.clone(),
            effective_seed: resolved.record.effective_seed.clone(),
            binding_digest: resolved.record.binding_digest.clone(),
        }
    );
    assert_eq!(transport.starts(), std::slice::from_ref(&sent));
    assert_eq!(
        transport.starts()[0].effective_seed,
        resolved.record.effective_seed
    );
}
