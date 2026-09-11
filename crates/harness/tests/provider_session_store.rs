// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use sts2_harness::provider_session::*;

static TEST_COUNTER: AtomicU64 = AtomicU64::new(1);

fn scope() -> SessionScope {
    SessionScope::new(
        "project-fixture",
        "run-fixture",
        "episode-fixture",
        "agent-fixture",
    )
    .expect("scope")
}

fn broker() -> ProviderSessionBroker {
    let scope = scope();
    let mut policy = ProviderSessionPolicy::disabled(scope.clone());
    policy.mode = ProviderSessionMode::FixtureOnly;
    policy.credential_realm_ref = "fixture-realm".to_owned();
    policy.profile_sha256 = sts2_harness::sha256_hex("codex-app-server-fixture-v1");
    ProviderSessionBroker::new(
        scope,
        policy,
        NativeCapabilities::fixture(),
        "owner-fixture",
    )
    .expect("broker")
}

fn private_test_directory() -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "ascension-provider-session-store-{}-{}",
        std::process::id(),
        TEST_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&directory).expect("test directory");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
            .expect("private permissions");
    }
    directory
}

#[test]
fn volatile_store_never_writes_and_cannot_restore() {
    let broker = broker();
    let store = ProviderSessionMetadataStore::volatile(scope()).expect("volatile store");
    assert_eq!(store.mode(), ProviderSessionMetadataMode::Volatile);
    store.save(&broker).expect("volatile save");
    assert_eq!(store.path(), None);
    assert!(matches!(
        store.load("replacement-owner", broker.policy(), broker.capabilities()),
        Err(ProviderSessionMetadataStoreError::Unsupported)
    ));
}

#[cfg(unix)]
#[test]
fn encrypted_store_round_trips_metadata_and_authenticates_bytes() {
    let directory = private_test_directory();
    let path = directory.join("metadata.bin");
    let mut broker = broker();
    let candidate = broker
        .create_candidate(
            "owner-fixture",
            "persisted-candidate",
            "branch-a",
            SessionPurpose::Executable,
            "2099-01-01T00:00:00Z",
        )
        .expect("candidate");
    let policy = broker.policy().clone();
    let capabilities = broker.capabilities().clone();
    let store = ProviderSessionMetadataStore::encrypted(&path, [7_u8; 32], scope())
        .expect("encrypted store");
    store.save(&broker).expect("save");
    let envelope = fs::read(&path).expect("envelope");
    assert!(!String::from_utf8_lossy(&envelope).contains("persisted-candidate"));
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        fs::metadata(&path).expect("metadata").permissions().mode() & 0o777,
        0o600
    );

    let restored = store
        .load("replacement-owner", &policy, &capabilities)
        .expect("load");
    assert_eq!(restored.owner_epoch(), 2);
    assert!(restored.operation(&candidate.operation_id).is_ok());

    let wrong_key = ProviderSessionMetadataStore::encrypted(&path, [8_u8; 32], scope())
        .expect("wrong-key store");
    assert!(matches!(
        wrong_key.load("replacement-owner", &policy, &capabilities),
        Err(ProviderSessionMetadataStoreError::Crypto)
    ));

    let mut tampered = envelope;
    let last = tampered.last_mut().expect("tagged envelope");
    *last ^= 1;
    fs::write(&path, tampered).expect("tamper");
    assert!(matches!(
        store.load("replacement-owner", &policy, &capabilities),
        Err(ProviderSessionMetadataStoreError::Crypto)
    ));
    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn encrypted_store_rejects_relative_or_unsafe_paths() {
    assert!(matches!(
        ProviderSessionMetadataStore::encrypted("relative/metadata.bin", [7_u8; 32], scope()),
        Err(ProviderSessionMetadataStoreError::InvalidPath)
    ));
    let directory = private_test_directory();
    let unsafe_path = directory.join("..").join("metadata.bin");
    assert!(matches!(
        ProviderSessionMetadataStore::encrypted(&unsafe_path, [7_u8; 32], scope()),
        Err(ProviderSessionMetadataStoreError::InvalidPath)
    ));
    fs::remove_dir_all(directory).expect("cleanup");
}
