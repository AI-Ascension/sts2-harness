// SPDX-License-Identifier: MIT

use super::*;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use sts2_harness::context_memory::policy_owner::{
    MemoryPolicyAuthority, MemoryPolicyOwner, PolicyAccess, PolicyCommand, PolicyStoreConsent,
    SavedPolicyRef, TrustedPolicyState,
};
use sts2_harness::context_memory::{DurableMemoryStore, MAX_OPTIONAL_BYTES, MemoryScope};
use sts2_harness::management::{AuthContext, StaticAuthenticator};

pub(super) struct EntryFixture {
    _temporary: fixture::Fixture,
    pub(super) root: PathBuf,
    pub(super) scope: MemoryScope,
    pub(super) policy_path: PathBuf,
    pub(super) source_policy: SavedPolicyRef,
    pub(super) source_raw: Vec<u8>,
    pub(super) corpus_path: PathBuf,
    pub(super) python: PathBuf,
    pub(super) python_sha: String,
    pub(super) mcp_script: PathBuf,
    pub(super) mcp_log: PathBuf,
    pub(super) agent_log: PathBuf,
    pub(super) agent_script: PathBuf,
    pub(super) config_path: PathBuf,
    pub(super) archive_path: PathBuf,
}

impl EntryFixture {
    pub(super) fn new() -> Self {
        let temporary = fixture::Fixture::new();
        let root = temporary.directory.clone();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
            .expect("private runtime test directory");

        let scope = fixture::scope();
        let mut trusted: TrustedPolicyState = fixture::state();
        let grant = trusted
            .grants
            .get_mut("policy-grant")
            .expect("fixture selector grant");
        grant.subject = String::from("profile:lookup-owner");
        grant.expires_at = 4_102_444_800;
        let authenticator = Arc::new(
            StaticAuthenticator::single(
                OWNER_TOKEN,
                AuthContext::new("profile:lookup-owner", ["workflow:*".to_owned()])
                    .expect("profile owner context"),
            )
            .expect("synthetic seed owner authenticator"),
        );
        let clock = Arc::new(fixture::Clock(AtomicU64::new(100)));
        let authority = Arc::new(
            MemoryPolicyAuthority::new(trusted.clone(), authenticator, clock)
                .expect("trusted policy authority"),
        );
        let policy_path = root.join("entry-policy.sqlite");
        let seeded_owner = MemoryPolicyOwner::open(
            &policy_path,
            [7; 32],
            authority.clone(),
            PolicyStoreConsent::SyntheticOnly,
        )
        .expect("seed explicit profile-owned policy store");
        let access = || PolicyAccess {
            bearer: Some(OWNER_TOKEN),
            grant_id: "policy-grant",
        };
        let imported = fixture::bytes(&fixture::policy(1, MEMORY_POLICY_SCHEMA_MAX_OPTIONAL_BYTES));
        let imported_ref = fixture::reference(&imported);
        seeded_owner
            .execute(
                access(),
                PolicyCommand::Import {
                    key: String::from("entry-import"),
                    raw: imported.clone(),
                },
            )
            .expect("import trusted source policy");
        seeded_owner
            .execute(
                access(),
                PolicyCommand::ProposeMigration {
                    key: String::from("entry-seed-proposal"),
                    review_id: String::from("entry-seed-review"),
                    source: imported_ref,
                    target_raw: fixture::bytes(&fixture::policy(2, MAX_OPTIONAL_BYTES)),
                    expected_active_version: None,
                },
            )
            .expect("propose bounded saved policy");
        let seed_review = seeded_owner
            .inspect_review(access(), "entry-seed-review")
            .expect("seed policy review");
        seeded_owner
            .execute(
                access(),
                PolicyCommand::Approve {
                    key: String::from("entry-seed-approval"),
                    review_id: seed_review.review_id.clone(),
                    review_sha256: seed_review.review_sha256.clone(),
                },
            )
            .expect("approve exact seed review");
        seeded_owner
            .execute(
                access(),
                PolicyCommand::Adopt {
                    key: String::from("entry-seed-adoption"),
                    review_id: seed_review.review_id.clone(),
                    review_sha256: seed_review.review_sha256.clone(),
                },
            )
            .expect("adopt source policy before runtime startup");
        let source = seeded_owner
            .inspect_policy(access(), &seed_review.target)
            .expect("exact adopted saved-policy bytes");
        std::fs::set_permissions(&policy_path, std::fs::Permissions::from_mode(0o600))
            .expect("private policy database");

        let corpus_path = root.join("entry-corpus.sqlite");
        let mut corpus_store = DurableMemoryStore::open_private(
            corpus_path.to_str().expect("UTF-8 corpus path"),
            scope.clone(),
            [9; 32],
        )
        .expect("private corpus database");
        for entry in trusted.corpus.entries() {
            corpus_store
                .publish(entry.clone())
                .expect("seed exact trusted corpus");
        }
        drop(corpus_store);

        let python = std::fs::canonicalize("/usr/bin/python3").expect("Python interpreter");
        let python_bytes = std::fs::read(&python).expect("read bounded test interpreter");
        assert!(python_bytes.len() <= 128 * 1024 * 1024);
        let python_sha = sts2_harness::sha256_hex(&python_bytes);
        let mcp_log = root.join("entry-mcp.jsonl");
        let agent_log = root.join("entry-agent.jsonl");
        let mcp_script = root.join("entry-mcp.py");
        let agent_script = root.join("entry-agent.py");
        write_private(&mcp_script, &mcp_server_script(&mcp_log));
        std::fs::set_permissions(&mcp_script, std::fs::Permissions::from_mode(0o700))
            .expect("make synthetic MCP executable");
        write_private(&agent_script, &agent_script_source(&agent_log));
        let config_path = root.join("lookup-owner-config.json");
        let archive_path = root.join("entry-archive.sqlite");

        Self {
            _temporary: temporary,
            root,
            scope,
            policy_path,
            source_policy: seed_review.target,
            source_raw: source.raw_bytes().to_vec(),
            corpus_path,
            python,
            python_sha,
            mcp_script,
            mcp_log,
            agent_log,
            agent_script,
            config_path,
            archive_path,
        }
    }
}

use super::peer::{agent_script_source, mcp_server_script};
use sts2_harness::context_memory::MEMORY_POLICY_SCHEMA_MAX_OPTIONAL_BYTES;
