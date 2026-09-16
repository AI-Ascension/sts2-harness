// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use super::*;

fn scope() -> SessionScope {
    SessionScope::new("project", "run.policy-owner", "episode", "agent").expect("scope")
}

fn path(label: &str) -> std::path::PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "sts2-provider-policy-owner-{label}-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&directory).expect("directory");
    directory.join("owner.bin")
}

fn store(path: &std::path::Path, scope: SessionScope) -> ProviderSessionMetadataStore {
    ProviderSessionMetadataStore::encrypted(path, [11; 32], scope).expect("store")
}

fn valid_policy(scope: SessionScope) -> ProviderSessionPolicy {
    let mut policy = ProviderSessionPolicy::disabled(scope);
    policy.mode = super::super::ProviderSessionMode::FixtureOnly;
    policy.credential_realm_ref = "fixture-realm".to_owned();
    policy.profile_sha256 = crate::sha256_hex("fixture-profile");
    policy
}

#[test]
fn exact_import_and_migration_source_bytes_are_retained() {
    let path = path("bytes");
    let scope = scope();
    let owner = ProviderSessionPolicyOwner::open(
        store(&path, scope.clone()),
        scope.clone(),
        NativeCapabilities::fixture(),
    )
    .expect("owner");

    let mut source = valid_policy(scope.clone());
    source.version = 2;
    source.epoch = 2;
    source.max_completed_turns = super::super::MAX_COMPLETED_TURNS + 1;
    let mut source_bytes = serde_json::to_vec_pretty(&source).expect("source");
    source_bytes.push(b'\n');
    let source_sha256 = owner.import(source_bytes.clone()).expect("source import");

    let mut target = source.clone();
    target.version = 3;
    target.epoch = 3;
    target.max_completed_turns = super::super::MAX_COMPLETED_TURNS;
    let target_bytes = serde_json::to_vec(&target).expect("target");
    owner
        .propose(
            "proposal-bytes",
            &source_sha256,
            target_bytes.clone(),
            owner.metadata().expect("metadata").revision,
        )
        .expect("proposal");
    drop(owner);

    let storage = store(&path, scope.clone());
    let journal: Journal = serde_json::from_slice(&storage.load_owner_journal().expect("journal"))
        .expect("decode journal");
    let imported = journal
        .policies
        .iter()
        .find(|record| record.sha256 == source_sha256)
        .expect("source record");
    assert_eq!(imported.bytes, source_bytes);
    assert_eq!(
        journal.proposals[0].migration.original_policy_bytes,
        source_bytes
    );
    let target_record = journal
        .policies
        .iter()
        .find(|record| record.sha256 == crate::sha256_hex(&target_bytes))
        .expect("target record");
    assert_eq!(target_record.bytes, target_bytes);
    std::fs::remove_dir_all(path.parent().expect("parent")).expect("cleanup");
}

#[test]
fn opening_rejects_policy_digest_and_scope_tampering() {
    for tamper_scope in [false, true] {
        let path = path(if tamper_scope { "scope" } else { "digest" });
        let scope = scope();
        let owner = ProviderSessionPolicyOwner::open(
            store(&path, scope.clone()),
            scope.clone(),
            NativeCapabilities::fixture(),
        )
        .expect("owner");
        owner
            .import(serde_json::to_vec(&valid_policy(scope.clone())).expect("policy"))
            .expect("import");
        drop(owner);

        let storage = store(&path, scope.clone());
        let mut journal: Journal =
            serde_json::from_slice(&storage.load_owner_journal().expect("journal"))
                .expect("decode journal");
        if tamper_scope {
            let mut policy: ProviderSessionPolicy =
                serde_json::from_slice(&journal.policies[0].bytes).expect("policy");
            policy.scope.run_id = "foreign-run".to_owned();
            journal.policies[0].bytes = serde_json::to_vec(&policy).expect("encode policy");
            journal.policies[0].sha256 = crate::sha256_hex(&journal.policies[0].bytes);
        } else {
            journal.policies[0].sha256 = "0".repeat(64);
        }
        storage
            .save_owner_journal(&serde_json::to_vec(&journal).expect("encode journal"))
            .expect("save tampered fixture");
        drop(storage);

        let reopened = ProviderSessionPolicyOwner::open(
            store(&path, scope.clone()),
            scope,
            NativeCapabilities::fixture(),
        );
        assert!(reopened.is_err(), "tampered owner state must fail closed");
        std::fs::remove_dir_all(path.parent().expect("parent")).expect("cleanup");
    }
}

#[test]
fn opening_rejects_proposal_digest_tampering() {
    let path = path("proposal");
    let scope = scope();
    let owner = ProviderSessionPolicyOwner::open(
        store(&path, scope.clone()),
        scope.clone(),
        NativeCapabilities::fixture(),
    )
    .expect("owner");
    let mut source = valid_policy(scope.clone());
    source.version = 2;
    source.epoch = 2;
    source.max_completed_turns = super::super::MAX_COMPLETED_TURNS + 1;
    let source_sha256 = owner
        .import(serde_json::to_vec(&source).expect("source"))
        .expect("source import");
    let mut target = source.clone();
    target.version = 3;
    target.epoch = 3;
    target.max_completed_turns = super::super::MAX_COMPLETED_TURNS;
    owner
        .propose(
            "proposal-tamper",
            &source_sha256,
            serde_json::to_vec(&target).expect("target"),
            owner.metadata().expect("metadata").revision,
        )
        .expect("proposal");
    drop(owner);

    let storage = store(&path, scope.clone());
    let mut journal: Journal =
        serde_json::from_slice(&storage.load_owner_journal().expect("journal"))
            .expect("decode journal");
    journal.proposals[0].digest = "0".repeat(64);
    storage
        .save_owner_journal(&serde_json::to_vec(&journal).expect("encode journal"))
        .expect("save tampered fixture");
    drop(storage);

    assert!(
        ProviderSessionPolicyOwner::open(
            store(&path, scope.clone()),
            scope,
            NativeCapabilities::fixture(),
        )
        .is_err(),
        "tampered proposal state must fail closed"
    );
    std::fs::remove_dir_all(path.parent().expect("parent")).expect("cleanup");
}
