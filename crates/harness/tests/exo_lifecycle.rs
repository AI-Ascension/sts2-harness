// SPDX-License-Identifier: MIT

#![cfg(unix)]
#![allow(clippy::expect_used, clippy::panic)]

use sts2_harness as harness_api;

#[path = "support/exo_lifecycle.rs"]
mod fixture;
#[path = "support/exo_lifecycle_recovery.rs"]
mod recovery;
use fixture::{Effect, Fixture};
use sts2_harness::exo_lifecycle::*;
use sts2_harness::provider_session::ProviderSessionMetadataStore;
use sts2_harness::{ProviderReservationState, sha256_hex};

#[test]
fn lock_child_process_helper() {
    let Some(directory) = std::env::var_os("STS2_TEST_OWNER_DIRECTORY") else {
        return;
    };
    let mut fixture = Fixture::new();
    fixture.config.directory = directory.into();
    if std::env::var_os("STS2_TEST_OWNER_CREATE").is_some() {
        let _owner = fixture.owner();
        return;
    }
    #[cfg(target_os = "linux")]
    for descriptor in std::fs::read_dir("/proc/self/fd").expect("descriptor list") {
        if let Ok(target) = std::fs::read_link(descriptor.expect("descriptor").path()) {
            assert_ne!(
                target,
                fixture.config.directory.join("owner.lock"),
                "lock leaked through exec"
            );
        }
    }
    if std::env::var_os("STS2_TEST_OWNER_EXPECT_BUSY").is_some() {
        assert!(matches!(fixture.reopen(), Err(LifecycleError::Busy)));
    } else {
        assert_eq!(
            fixture
                .reopen()
                .expect("claim after process release")
                .claim_epoch(),
            2
        );
    }
}

#[test]
fn os_process_contention_and_exec_descriptor_isolation() {
    let mut fixture = Fixture::new();
    let owner = fixture.owner();
    let before = std::fs::read(fixture.config.directory.join("journal.enc")).expect("journal");
    let status = std::process::Command::new(std::env::current_exe().expect("test executable"))
        .args(["--exact", "lock_child_process_helper"])
        .env("STS2_TEST_OWNER_DIRECTORY", &fixture.config.directory)
        .env("STS2_TEST_OWNER_EXPECT_BUSY", "1")
        .status()
        .expect("contending process");
    assert!(status.success());
    assert_eq!(
        std::fs::read(fixture.config.directory.join("journal.enc")).expect("unchanged"),
        before
    );
    drop(owner);
    let status = std::process::Command::new(std::env::current_exe().expect("test executable"))
        .args(["--exact", "lock_child_process_helper"])
        .env("STS2_TEST_OWNER_DIRECTORY", &fixture.config.directory)
        .env_remove("STS2_TEST_OWNER_EXPECT_BUSY")
        .status()
        .expect("new process claim");
    assert!(status.success());
}

#[test]
fn owner_process_exit_releases_lock_for_next_process_claim() {
    let fixture = Fixture::new();
    let status = std::process::Command::new(std::env::current_exe().expect("test executable"))
        .args(["--exact", "lock_child_process_helper"])
        .env("STS2_TEST_OWNER_DIRECTORY", &fixture.config.directory)
        .env("STS2_TEST_OWNER_CREATE", "1")
        .env_remove("STS2_TEST_OWNER_EXPECT_BUSY")
        .status()
        .expect("owner process");
    assert!(status.success());
    let status = std::process::Command::new(std::env::current_exe().expect("test executable"))
        .args(["--exact", "lock_child_process_helper"])
        .env("STS2_TEST_OWNER_DIRECTORY", &fixture.config.directory)
        .env_remove("STS2_TEST_OWNER_CREATE")
        .env_remove("STS2_TEST_OWNER_EXPECT_BUSY")
        .status()
        .expect("replacement process");
    assert!(status.success());
}

#[test]
fn exclusive_owner_releases_only_on_drop_and_restart_never_resends() {
    let mut fixture = Fixture::new();
    let owner = fixture.owner();
    assert!(matches!(fixture.reopen(), Err(LifecycleError::Busy)));
    drop(owner);
    let mut reopened = fixture.reopen().expect("new owner");
    let mut effect = Effect::default();
    assert!(
        reopened
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
}

#[test]
fn sent_boundary_precedes_one_handoff_and_duplicate_returns_stored_result() {
    let mut fixture = Fixture::new();
    let mut owner = fixture.owner();
    let mut effect = Effect::default();
    let StartOutcome::Started(mut handle) = owner
        .start(
            fixture.manifest.clone(),
            &fixture.input,
            &mut fixture.store,
            &fixture.fingerprint,
            &mut effect,
        )
        .expect("start")
    else {
        panic!("new send")
    };
    assert_eq!(owner.entries()[0].phase, LifecyclePhase::Sent);
    assert!(owner.entries()[0].possible_write);
    let decision = owner
        .poll(&mut handle, &mut fixture.store)
        .expect("complete")
        .expect("decision");
    assert!(!decision.action_id.is_empty());
    assert_eq!(owner.entries()[0].phase, LifecyclePhase::Completed);
    assert!(owner.poll(&mut handle, &mut fixture.store).is_err());
    assert!(matches!(
        owner.start(
            fixture.manifest.clone(),
            &fixture.input,
            &mut fixture.store,
            &fixture.fingerprint,
            &mut effect
        ),
        Ok(StartOutcome::Stored(_))
    ));
    assert_eq!(effect.calls, 1);
    let reservation = fixture
        .store
        .provider_reservation("reservation-1")
        .expect("reservation");
    assert_eq!(reservation.state, ProviderReservationState::Completed);
    assert_eq!(reservation.actual_units, Some(3));
    let journal = std::fs::read(fixture.config.directory.join("journal.enc")).expect("encrypted");
    assert!(
        !journal
            .windows(fixture.input.len())
            .any(|bytes| bytes == fixture.input)
    );
}

#[test]
fn ambiguous_handoff_is_unknown_and_duplicate_cannot_retry() {
    let mut fixture = Fixture::new();
    let mut owner = fixture.owner();
    let mut effect = Effect {
        ambiguous: true,
        ..Effect::default()
    };
    assert!(matches!(
        owner.start(
            fixture.manifest.clone(),
            &fixture.input,
            &mut fixture.store,
            &fixture.fingerprint,
            &mut effect
        ),
        Err(LifecycleError::Unknown)
    ));
    assert_eq!(owner.entries()[0].phase, LifecyclePhase::Unknown);
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
    assert_eq!(effect.calls, 1);
    assert_eq!(
        fixture
            .store
            .provider_reservation("reservation-1")
            .expect("reservation")
            .state,
        ProviderReservationState::Unknown
    );
    drop(owner);
    let mut reopened = fixture.reopen().expect("held restart");
    assert!(
        reopened
            .start(
                fixture.manifest.clone(),
                &fixture.input,
                &mut fixture.store,
                &fixture.fingerprint,
                &mut effect
            )
            .is_err()
    );
    assert_eq!(effect.calls, 1);
}

#[test]
fn revocation_can_linearize_after_handoff_and_fences_late_completion() {
    let mut fixture = Fixture::new();
    let mut owner = fixture.owner();
    let mut effect = Effect::default();
    let StartOutcome::Started(mut handle) = owner
        .start(
            fixture.manifest.clone(),
            &fixture.input,
            &mut fixture.store,
            &fixture.fingerprint,
            &mut effect,
        )
        .expect("start")
    else {
        panic!("handle")
    };
    // Acquiring this mutex proves try_start did not retain the short authority guard.
    *fixture
        .authority
        .revoked
        .try_lock()
        .expect("guard released") = true;
    assert!(matches!(
        owner.poll(&mut handle, &mut fixture.store),
        Err(LifecycleError::Fenced)
    ));
    assert_eq!(owner.entries()[0].phase, LifecyclePhase::Fenced);
    assert!(
        !fixture
            .store
            .decision(&fixture.manifest.execution_id)
            .expect("decision")
            .completed
    );
    assert_eq!(effect.calls, 1);
}

#[test]
fn unknown_zero_and_excess_usage_never_invent_accounting() {
    for units in [None, Some(0), Some(11)] {
        let mut fixture = Fixture::new();
        let mut owner = fixture.owner();
        let mut effect = Effect {
            units,
            ..Effect::default()
        };
        let StartOutcome::Started(mut handle) = owner
            .start(
                fixture.manifest.clone(),
                &fixture.input,
                &mut fixture.store,
                &fixture.fingerprint,
                &mut effect,
            )
            .expect("start")
        else {
            panic!("handle")
        };
        assert!(matches!(
            owner.poll(&mut handle, &mut fixture.store),
            Err(LifecycleError::Unknown)
        ));
        let reservation = fixture
            .store
            .provider_reservation("reservation-1")
            .expect("reservation");
        assert_eq!(reservation.state, ProviderReservationState::Unknown);
        assert_eq!(reservation.actual_units, None);
    }
}

#[test]
fn changed_input_or_authority_or_prepared_identity_is_denied_before_effect() {
    for variant in 0..4 {
        let mut fixture = Fixture::new();
        let mut owner = fixture.owner();
        let mut manifest = fixture.manifest.clone();
        match variant {
            0 => manifest.input_digest = sha256_hex("other"),
            1 => manifest.authority.session_epoch += 1,
            2 => manifest.prepared_id = "different-prepared".into(),
            _ => manifest.authority.catalog_digest = sha256_hex("other"),
        }
        let mut effect = Effect::default();
        assert!(
            owner
                .start(
                    manifest,
                    &fixture.input,
                    &mut fixture.store,
                    &fixture.fingerprint,
                    &mut effect
                )
                .is_err()
        );
        assert_eq!(effect.calls, 0);
        assert!(owner.entries().is_empty());
    }
}

#[test]
fn replaced_lock_inode_poison_is_sticky() {
    use std::os::unix::fs::PermissionsExt;
    let mut fixture = Fixture::new();
    let mut owner = fixture.owner();
    let lock = fixture.config.directory.join("owner.lock");
    std::fs::rename(&lock, fixture.config.directory.join("displaced.lock"))
        .expect("simulate replacement");
    std::fs::write(&lock, b"").expect("replacement");
    std::fs::set_permissions(&lock, std::fs::Permissions::from_mode(0o600)).expect("private");
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
    assert!(matches!(
        owner.start(
            fixture.manifest.clone(),
            &fixture.input,
            &mut fixture.store,
            &fixture.fingerprint,
            &mut effect
        ),
        Err(LifecycleError::Poisoned)
    ));
    assert_eq!(effect.calls, 0);
}

#[test]
fn v1_import_requires_quiesced_cutover_preserves_old_bytes_and_never_resends() {
    let mut fixture = Fixture::new();
    let path = fixture.root.join("legacy.enc");
    let legacy =
        ProviderSessionMetadataStore::encrypted(&path, [7; 32], fixture.config.scope.clone())
            .expect("legacy");
    legacy
        .save(fixture.broker.as_ref().expect("broker"))
        .expect("save");
    let original = std::fs::read(&path).expect("legacy bytes");
    fixture.config.legacy_path = Some(path.clone());
    let import = |fixture: &Fixture| {
        LifecycleOwner::import_legacy(
            fixture.config.clone(),
            [7; 32],
            "migrated-owner".into(),
            &fixture.policy,
            &fixture.capabilities,
            sha256_hex(&original),
            "authenticated-cutover-fixture".into(),
            fixture.authority.clone(),
        )
    };
    assert!(matches!(
        import(&fixture),
        Err(LifecycleError::LegacyOwnerNotQuiesced)
    ));
    assert!(!fixture.config.directory.exists());
    *fixture
        .authority
        .legacy_quiesced
        .lock()
        .expect("legacy authority") = true;
    let mut owner = import(&fixture).expect("migration");
    assert_eq!(std::fs::read(&path).expect("preserved"), original);
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
    assert!(import(&fixture).is_err());
    let v2_before = std::fs::read(fixture.config.directory.join("journal.enc")).expect("v2");
    legacy
        .save(fixture.broker.as_ref().expect("old broker"))
        .expect("old v1 writer");
    assert_ne!(std::fs::read(&path).expect("fresh v1 nonce"), original);
    assert_eq!(
        std::fs::read(fixture.config.directory.join("journal.enc")).expect("isolated v2"),
        v2_before
    );
}
