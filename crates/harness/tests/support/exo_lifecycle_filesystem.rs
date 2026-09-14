// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use super::fixture::{Effect, Fixture};
use std::os::unix::fs::{PermissionsExt, symlink};
use sts2_harness::exo_lifecycle::*;

#[test]
fn missing_symlink_or_nonprivate_lock_is_never_recreated() {
    for variant in 0..3 {
        let mut fixture = Fixture::new();
        let mut owner = fixture.owner();
        let lock = fixture.config.directory.join("owner.lock");
        let original = fixture.config.directory.join("original.lock");
        std::fs::rename(&lock, &original).expect("displace lock");
        match variant {
            0 => (),
            1 => symlink(&original, &lock).expect("symlink"),
            _ => {
                std::fs::write(&lock, b"").expect("replacement");
                std::fs::set_permissions(&lock, std::fs::Permissions::from_mode(0o644))
                    .expect("nonprivate");
            }
        }
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
        assert!(fixture.reopen().is_err());
        if variant == 0 {
            assert!(!lock.exists());
        }
        assert!(original.exists());
        assert_eq!(effect.calls, 0);
    }
}

#[test]
fn replacing_only_journal_inode_does_not_release_the_separate_owner_lock() {
    let mut fixture = Fixture::new();
    let mut owner = fixture.owner();
    let journal = fixture.config.directory.join("journal.enc");
    let replacement = fixture.config.directory.join("replacement.enc");
    std::fs::write(&replacement, std::fs::read(&journal).expect("journal")).expect("replacement");
    std::fs::set_permissions(&replacement, std::fs::Permissions::from_mode(0o600))
        .expect("private");
    std::fs::rename(&replacement, &journal).expect("atomic replacement");
    assert!(matches!(fixture.reopen(), Err(LifecycleError::Busy)));
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
            .is_ok()
    );
    assert_eq!(effect.calls, 1);
    assert_eq!(
        std::fs::metadata(&journal)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn wrong_key_and_symlinked_directory_leave_existing_journal_unchanged() {
    let mut fixture = Fixture::new();
    let owner = fixture.owner();
    drop(owner);
    let journal = fixture.config.directory.join("journal.enc");
    let before = std::fs::read(&journal).expect("journal");
    assert!(
        LifecycleOwner::open(
            fixture.config.clone(),
            [8; 32],
            "new-owner".into(),
            &fixture.policy,
            &fixture.capabilities,
            fixture.authority.clone()
        )
        .is_err()
    );
    assert_eq!(std::fs::read(&journal).expect("unchanged"), before);
    let alias = fixture.root.join("alias");
    symlink(&fixture.config.directory, &alias).expect("directory alias");
    fixture.config.directory = alias;
    assert!(fixture.reopen().is_err());
    assert_eq!(std::fs::read(&journal).expect("unchanged"), before);
}
