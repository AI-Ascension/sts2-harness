// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use super::*;
use crate::provider_session::*;
use std::os::unix::fs::PermissionsExt;

#[path = "tests_bounds.rs"]
mod bounds;

struct Fixture {
    root: std::path::PathBuf,
    config: JournalConfig,
    snapshot: JournalSnapshot,
}
impl Fixture {
    fn new() -> Self {
        let mut random = [0; 16];
        getrandom::fill(&mut random).expect("test randomness");
        let root =
            std::env::temp_dir().join(format!("sts2-owner-journal-{}", crate::sha256_hex(random)));
        std::fs::create_dir(&root).expect("root");
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).expect("private");
        let scope = SessionScope::new("project", "run", "episode", "agent").expect("scope");
        let mut policy = ProviderSessionPolicy::disabled(scope.clone());
        policy.mode = ProviderSessionMode::FixtureOnly;
        policy.credential_realm_ref = "fixture-realm".into();
        policy.profile_sha256 = crate::sha256_hex("codex-app-server-fixture-v1");
        let broker = ProviderSessionBroker::new(
            scope.clone(),
            policy,
            NativeCapabilities::fixture(),
            "owner",
        )
        .expect("broker");
        let config = JournalConfig {
            directory: root.join("journal"),
            legacy_path: None,
            store_id: "store".into(),
            scope,
            owner_binding_digest: crate::sha256_hex("owner"),
        };
        let snapshot = JournalSnapshot::new(&config, broker.snapshot());
        Self {
            root,
            config,
            snapshot,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn every_commit_failure_poisons_and_reopen_observes_only_whole_snapshot() {
    use io::CommitStage::*;
    for stage in [
        BeforeWrite,
        AfterPartialWrite,
        AfterWrite,
        AfterFileSync,
        AfterRename,
        AfterDirectorySync,
    ] {
        let fixture = Fixture::new();
        let mut journal = OwnerJournal::create(fixture.config.clone(), [7; 32], &fixture.snapshot)
            .expect("create");
        let mut next = fixture.snapshot.clone();
        next.revision += 1;
        io::FAILURE.set(Some(stage));
        let failed = journal.commit(&next);
        io::FAILURE.set(None);
        assert_eq!(failed, Err(LifecycleError::Io));
        assert_eq!(journal.commit(&next), Err(LifecycleError::Poisoned));
        drop(journal);
        let (_reopened, snapshot) =
            OwnerJournal::open(fixture.config.clone(), [7; 32]).expect("whole snapshot");
        assert_eq!(
            snapshot.revision,
            if matches!(stage, AfterRename | AfterDirectorySync) {
                2
            } else {
                1
            }
        );
        assert!(
            std::fs::read_dir(&fixture.config.directory)
                .expect("files")
                .all(|entry| !entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".tmp"))
        );
    }
}

#[test]
fn authenticated_scope_key_store_and_owner_binding_are_required() {
    let fixture = Fixture::new();
    let encoded = io::encode(&fixture.config, &[7; 32], &fixture.snapshot).expect("encode");
    assert!(io::decode(&fixture.config, &[8; 32], &encoded).is_err());
    for variant in 0..3 {
        let mut config = fixture.config.clone();
        match variant {
            0 => config.store_id = "other".into(),
            1 => {
                config.scope = SessionScope::new("other", "run", "episode", "agent").expect("scope")
            }
            _ => config.owner_binding_digest = crate::sha256_hex("other"),
        }
        assert!(io::decode(&config, &[7; 32], &encoded).is_err());
    }
    let mut tampered = encoded;
    tampered[50] ^= 1;
    assert!(io::decode(&fixture.config, &[7; 32], &tampered).is_err());
}

#[test]
fn externally_advanced_revision_fences_the_current_writer() {
    let fixture = Fixture::new();
    let mut journal =
        OwnerJournal::create(fixture.config.clone(), [7; 32], &fixture.snapshot).expect("create");
    let mut advanced = fixture.snapshot.clone();
    advanced.revision = 2;
    let bytes = io::encode(&fixture.config, &[7; 32], &advanced).expect("encode");
    io::write(&journal.lease, &bytes).expect("simulate external advance");
    assert_eq!(journal.commit(&advanced), Err(LifecycleError::Stale));
    assert_eq!(journal.commit(&advanced), Err(LifecycleError::Poisoned));
}
