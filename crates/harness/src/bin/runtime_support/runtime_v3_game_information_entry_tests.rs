// SPDX-License-Identifier: MIT

use super::*;
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::AtomicU64;
use sts2_harness::context_memory::policy_owner::{
    MemoryPolicyAuthority, MemoryPolicyOwner, PolicyAccess, PolicyCommand, PolicyStoreConsent,
    TrustedPolicyState,
};
use sts2_harness::context_memory::{MAX_OPTIONAL_BYTES, MEMORY_POLICY_SCHEMA_MAX_OPTIONAL_BYTES};
use sts2_harness::management::{AuthContext, StaticAuthenticator};

const CHILD_TEST: &str = "runtime_support::runtime_v3::game_information_owner::owner_management_tests::entry_tests::runtime_entry_child";
const OWNER_TOKEN: &str = "runtime-entry-owner-token";
const PROJECT: &str = "project";
const RUN: &str = "run";
const EPISODE: &str = "episode";
const AGENT: &str = "agent";
const STATE_ID: &str = "combat-1";
const STATE_GENERATION: u64 = 41;

#[test]
fn runtime_entry_child() {
    if std::env::var("STS2_LOOKUP_ENTRY_CHILD").as_deref() != Ok("true") {
        return;
    }
    let config = crate::runtime_support::RuntimeConfig::from_environment()
        .expect("the isolated child receives a complete runtime configuration");
    crate::runtime_support::run(config)
        .expect("the actual runtime entry completes the adopted game-information episode");
}

#[test]
fn runtime_entry_adopts_delivers_and_replays_game_information_with_scripted_mcp_peer() {
    let temp = fixture::Fixture::new();
    let root = &temp.directory;
    std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700))
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

    let gateway = TcpListener::bind("127.0.0.1:0").expect("synthetic gateway listener");
    gateway
        .set_nonblocking(true)
        .expect("nonblocking gateway listener");
    let gateway_address = gateway.local_addr().expect("gateway address");
    let management_address = free_loopback_address();
    let config_path = root.join("lookup-owner-config.json");
    let archive_path = root.join("entry-archive.sqlite");

    for replay in [false, true] {
        if replay {
            let _ = std::fs::remove_file(&mcp_log);
            let _ = std::fs::remove_file(&agent_log);
        }
        let execution_path = root.join(if replay {
            "entry-execution-replay.sqlite"
        } else {
            "entry-execution.sqlite"
        });
        let branch_path = root.join(if replay {
            "entry-branches-replay.sqlite"
        } else {
            "entry-branches.sqlite"
        });
        let config_sha = write_owner_config(
            &config_path,
            &corpus_path,
            &policy_path,
            &archive_path,
            management_address,
            replay,
        );
        let mut child = start_runtime_child(
            gateway_address,
            &config_path,
            &config_sha,
            &mcp_script,
            &agent_script,
            &python,
            &python_sha,
            &agent_log,
            &execution_path,
            &branch_path,
            replay,
        );
        let client = wait_for_management(management_address);

        // The real runtime entry is blocked in its preflight. It must not allocate
        // a game lease or spawn the MCP/agent before the operator adopts the current policy.
        match gateway.accept() {
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => panic!("unexpected gateway preflight probe error: {error}"),
            Ok((_stream, _)) => {
                let _ = child.kill();
                panic!("runtime performed a gateway effect before explicit policy adoption");
            }
        }
        explicit_revalidation_approval_and_adoption(
            &client,
            &seed_review.target,
            source.raw_bytes(),
            if replay {
                "entry-replay"
            } else {
                "entry-first"
            },
        );

        let gateway_copy = gateway.try_clone().expect("clone gateway listener");
        let gateway_worker = std::thread::spawn(move || serve_gateway(gateway_copy));
        let output = finish_child(child);
        let gateway_result = gateway_worker.join().expect("synthetic gateway worker");
        assert!(
            output.status.success(),
            "runtime entry failed:\n{}\n{}\nMCP log:\n{}\nAgent log:\n{}\nGateway result: {:?}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&std::fs::read(&mcp_log).unwrap_or_default()),
            String::from_utf8_lossy(&std::fs::read(&agent_log).unwrap_or_default()),
            gateway_result
        );
        let requests = gateway_result.expect("gateway allocation, binding and release flow");
        assert_eq!(requests[0], "POST /v1/sessions/allocate");
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.contains("/game-information/lookup-binding"))
                .count(),
            3,
            "the actual runner must retain and reobserve the binding before its decision"
        );
        assert_eq!(
            requests.last().map(String::as_str),
            Some("POST /v1/instances/instance-1/release")
        );

        let agent_events = read_json_lines(&agent_log);
        assert!(
            agent_events.iter().any(|event| event["kind"] == "data")
                && agent_events.iter().any(|event| event["kind"] == "decision"),
            "agent must receive query data and emit a legal decision on the live run and replay"
        );
        assert_eq!(
            agent_events
                .iter()
                .filter(|event| event["kind"] == "data")
                .count(),
            1,
            "one query result must reach the agent for each run"
        );
        assert_eq!(
            agent_events
                .iter()
                .filter(|event| event["kind"] == "decision")
                .count(),
            1,
            "one decision must be emitted for each run"
        );
        assert!(
            agent_events
                .iter()
                .filter(|event| event["kind"] == "decision")
                .all(|event| event["action_id"] == "combat.end-turn"),
            "agent decision must be a member of the runtime-provided legal set"
        );
        assert!(
            agent_events
                .iter()
                .all(|event| event["owner_secrets_absent"] == true),
            "lookup agent must not inherit owner credentials or store keys"
        );
        let mcp_events = read_json_lines(&mcp_log);
        let startup = mcp_events
            .iter()
            .find(|event| event["kind"] == "startup")
            .expect("the MCP process records its explicit startup request");
        assert_eq!(
            startup["lookup_discovery_request"],
            json!({
                "operation":"discovery",
                "scope":{
                    "project_id":PROJECT,"run_id":RUN,"episode_id":EPISODE,"agent_id":AGENT
                },
                "authority_epoch":1,
                "correlation_id":"game-information-binding-discovery"
            }),
            "MCP bootstrap identity must come from the adopted owner, not STS2_AUTHORITY_EPOCH or the inherited observe-shaped value"
        );
        assert!(
            mcp_events
                .iter()
                .all(|event| event["owner_secrets_absent"] == true),
            "game MCP child must not inherit owner credentials or store keys"
        );
        if replay {
            assert!(
                !mcp_events.iter().any(|event| {
                    matches!(
                        event["tool"].as_str(),
                        Some("sts2.game_information_capabilities" | "sts2.game_information_list")
                    )
                }),
                "replay must deliver the archived transcript without another game-information query"
            );
        } else {
            assert!(
                mcp_events
                    .iter()
                    .any(|event| event["tool"] == "sts2.game_information_list"),
                "the live entry must send the admitted query through the actual MCP process"
            );
            let archived = DurableMemoryStore::open_private(
                archive_path.to_str().expect("UTF-8 archive path"),
                scope.clone(),
                [11; 32],
            )
            .expect("reopen encrypted lookup archive");
            let archived_corpus = archived.load_corpus().expect("read archived transcript");
            assert!(
                archived_corpus
                    .entries()
                    .any(|entry| entry.entry_id.starts_with("lookup-archive:")),
                "normal runtime shutdown must persist the game-information archive"
            );
        }
    }
}

#[path = "runtime_v3_game_information_entry_gateway.rs"]
mod gateway;
#[path = "runtime_v3_game_information_entry_peer.rs"]
mod peer;
#[path = "runtime_v3_game_information_entry_support.rs"]
mod support;

use gateway::serve_gateway;
use peer::{agent_script_source, mcp_server_script};
use support::{
    explicit_revalidation_approval_and_adoption, finish_child, free_loopback_address,
    read_json_lines, start_runtime_child, wait_for_management, write_owner_config, write_private,
};
