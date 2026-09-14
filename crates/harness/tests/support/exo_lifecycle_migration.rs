// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use super::fixture::{Effect, Fixture};
use sts2_harness::exo_lifecycle::*;
use sts2_harness::provider_session::ProviderSessionMetadataStore;
use sts2_harness::sha256_hex;

fn legacy(fixture: &mut Fixture) -> (ProviderSessionMetadataStore, Vec<u8>) {
    let path = fixture.root.join("legacy.enc");
    let legacy =
        ProviderSessionMetadataStore::encrypted(&path, [7; 32], fixture.config.scope.clone())
            .expect("legacy");
    legacy
        .save(fixture.broker.as_ref().expect("broker"))
        .expect("save");
    let original = std::fs::read(&path).expect("legacy bytes");
    fixture.config.legacy_path = Some(path);
    (legacy, original)
}

fn import(fixture: &Fixture, original: &[u8]) -> Result<LifecycleOwner, LifecycleError> {
    LifecycleOwner::import_legacy(
        fixture.config.clone(),
        [7; 32],
        "migrated-owner".into(),
        &fixture.policy,
        &fixture.capabilities,
        sha256_hex(original),
        "authenticated-cutover-fixture".into(),
        fixture.authority.clone(),
    )
}

#[test]
fn v1_import_requires_quiesced_cutover_preserves_old_bytes_and_never_resends() {
    let mut fixture = Fixture::new();
    let (legacy, original) = legacy(&mut fixture);
    assert!(matches!(
        import(&fixture, &original),
        Err(LifecycleError::LegacyOwnerNotQuiesced)
    ));
    assert!(!fixture.config.directory.exists());
    *fixture
        .authority
        .legacy_quiesced
        .lock()
        .expect("legacy authority") = true;
    let mut owner = import(&fixture, &original).expect("migration");
    assert_eq!(
        std::fs::read(legacy.path().expect("legacy path")).expect("preserved"),
        original
    );
    let mut effect = Effect::default();
    assert!(
        owner
            .start(
                fixture.manifest.clone(),
                &fixture.input,
                &mut fixture.store,
                &fixture.fingerprint,
                &mut effect
            )
            .is_err()
    );
    assert_eq!(effect.calls, 0);
    assert!(import(&fixture, &original).is_err());
    let v2_before = std::fs::read(fixture.config.directory.join("journal.enc")).expect("v2");
    legacy
        .save(fixture.broker.as_ref().expect("old broker"))
        .expect("old v1 writer");
    assert_ne!(
        std::fs::read(legacy.path().expect("legacy path")).expect("fresh v1 nonce"),
        original
    );
    assert_eq!(
        std::fs::read(fixture.config.directory.join("journal.enc")).expect("isolated v2"),
        v2_before
    );
}

#[test]
fn changed_legacy_source_is_denied_before_destination_creation() {
    let mut fixture = Fixture::new();
    let (legacy, original) = legacy(&mut fixture);
    *fixture
        .authority
        .legacy_quiesced
        .lock()
        .expect("legacy authority") = true;
    legacy
        .save(fixture.broker.as_ref().expect("old broker"))
        .expect("new v1 envelope");
    assert!(matches!(
        import(&fixture, &original),
        Err(LifecycleError::Stale)
    ));
    assert!(!fixture.config.directory.exists());
}
