// SPDX-License-Identifier: MIT

use super::*;
use std::net::TcpListener;

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
    run_runtime_entry(EntryMode::Scripted);
}

#[test]
#[ignore = "operator-only acceptance; requires exact Gateway, MCP, and harness runtime binaries"]
fn runtime_entry_adopts_queries_and_replays_through_real_gateway_and_mcp_processes() {
    run_runtime_entry(EntryMode::RealPeers {
        mismatch_manifest: false,
    });
}

#[test]
#[ignore = "operator-only negative acceptance; requires exact Gateway, MCP, and harness runtime binaries"]
fn runtime_entry_refuses_foreign_manifest_before_query_or_agent_delivery() {
    run_runtime_entry(EntryMode::RealPeers {
        mismatch_manifest: true,
    });
}

#[derive(Clone, Copy)]
enum EntryMode {
    Scripted,
    RealPeers { mismatch_manifest: bool },
}

fn run_runtime_entry(mode: EntryMode) {
    let fixture = fixture_setup::EntryFixture::new();
    let root = &fixture.root;
    let scope = fixture.scope.clone();
    let policy_path = &fixture.policy_path;
    let source_raw = &fixture.source_raw;
    let corpus_path = &fixture.corpus_path;
    let python = &fixture.python;
    let python_sha = &fixture.python_sha;
    let mcp_script = &fixture.mcp_script;
    let mcp_log = &fixture.mcp_log;
    let agent_log = &fixture.agent_log;
    let agent_script = &fixture.agent_script;

    let gateway = if matches!(mode, EntryMode::Scripted) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("synthetic gateway listener");
        listener
            .set_nonblocking(true)
            .expect("nonblocking gateway listener");
        Some(listener)
    } else {
        None
    };
    let gateway_binary = match mode {
        EntryMode::Scripted => None,
        EntryMode::RealPeers { .. } => Some(
            live_peers::pinned_binary("STS2_GATEWAY_BINARY")
                .expect("exact Gateway binary for real-peer acceptance"),
        ),
    };
    let mcp_binary = match mode {
        EntryMode::Scripted => mcp_script.clone(),
        EntryMode::RealPeers { .. } => {
            live_peers::pinned_binary("STS2_MCP_BINARY").expect("exact MCP binary")
        }
    };
    let runtime_binary = match mode {
        EntryMode::Scripted => None,
        EntryMode::RealPeers { .. } => Some(
            live_peers::pinned_binary("STS2_HARNESS_RUNTIME_BINARY")
                .expect("shipped harness runtime binary"),
        ),
    };
    let gateway_address = gateway
        .as_ref()
        .map(|listener| listener.local_addr().expect("gateway address"));
    let management_address = free_loopback_address();
    let config_path = &fixture.config_path;
    let archive_path = &fixture.archive_path;

    let replay_modes: &[bool] = if matches!(
        mode,
        EntryMode::RealPeers {
            mismatch_manifest: true
        }
    ) {
        &[false]
    } else {
        &[false, true]
    };
    for replay in replay_modes.iter().copied() {
        if replay {
            let _ = std::fs::remove_file(mcp_log);
            let _ = std::fs::remove_file(agent_log);
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
            config_path,
            corpus_path,
            policy_path,
            archive_path,
            management_address,
            replay,
        );
        let mut live = match (mode, gateway_binary.as_deref()) {
            (EntryMode::RealPeers { mismatch_manifest }, Some(binary)) => Some(
                live_peers::LivePeers::start(binary, mismatch_manifest)
                    .expect("start actual Gateway and synthetic producer"),
            ),
            _ => None,
        };
        let gateway_address = live
            .as_ref()
            .map(live_peers::LivePeers::address)
            .or(gateway_address)
            .expect("gateway address");
        let mut child = start_runtime_child(
            gateway_address,
            config_path,
            &config_sha,
            &mcp_binary,
            runtime_binary.as_deref(),
            if live.is_some() {
                "gateway-token"
            } else {
                "synthetic-gateway-token"
            },
            agent_script,
            python,
            python_sha,
            agent_log,
            &execution_path,
            &branch_path,
            replay,
        );
        let client = wait_for_management(management_address);

        // The real runtime entry is blocked in its preflight. It must not allocate
        // a game lease or spawn the MCP/agent before the operator adopts the current policy.
        if let Some(listener) = gateway.as_ref() {
            match listener.accept() {
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => panic!("unexpected gateway preflight probe error: {error}"),
                Ok((_stream, _)) => {
                    let _ = child.kill();
                    panic!("runtime performed a gateway effect before explicit policy adoption");
                }
            }
        } else if let Some(live) = live.as_ref() {
            live.assert_no_downstream_request();
        }
        explicit_revalidation_approval_and_adoption(
            &client,
            &fixture.source_policy,
            source_raw,
            if replay {
                "entry-replay"
            } else {
                "entry-first"
            },
        );

        let gateway_worker = gateway.as_ref().map(|listener| {
            let copy = listener.try_clone().expect("clone gateway listener");
            std::thread::spawn(move || serve_gateway(copy))
        });
        let output = finish_child(child);
        let gateway_result = gateway_worker.map(|worker| worker.join().expect("gateway worker"));
        let live_requests = live.take().map(|peers| {
            peers
                .finish()
                .expect("actual Gateway and synthetic producer complete cleanly")
        });
        if matches!(
            mode,
            EntryMode::RealPeers {
                mismatch_manifest: true
            }
        ) {
            assert!(
                !output.status.success(),
                "foreign producer manifest must prevent runtime admission"
            );
            assert!(
                read_json_lines(agent_log).is_empty(),
                "foreign producer manifest must be refused before agent delivery"
            );
            let requests = live_requests.expect("actual producer request log");
            assert!(
                requests
                    .iter()
                    .any(|request| request.path.ends_with("/game-information/lookup-binding")),
                "actual MCP bootstrap must reach Gateway lookup-binding validation"
            );
            assert!(
                !requests
                    .iter()
                    .any(|request| request.path.ends_with("/game-information/detail")),
                "foreign producer manifest must be refused before a content query"
            );
            continue;
        }
        assert!(
            output.status.success(),
            "runtime entry failed:\n{}\n{}\nMCP log:\n{}\nAgent log:\n{}\nGateway result: {:?}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&std::fs::read(mcp_log).unwrap_or_default()),
            String::from_utf8_lossy(&std::fs::read(agent_log).unwrap_or_default()),
            gateway_result
        );
        if let Some(requests) = gateway_result {
            let requests = requests.expect("gateway allocation, binding and release flow");
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
                requests
                    .iter()
                    .filter(|request| {
                        request.contains("/game-information/live-observation-bootstrap")
                    })
                    .count(),
                usize::from(!replay),
                "bootstrap must cross the MCP-owned gateway route only on the live run"
            );
            assert_eq!(
                requests.last().map(String::as_str),
                Some("POST /v1/instances/instance-1/release")
            );
        } else {
            let requests = live_requests.expect("actual Gateway producer request log");
            if replay {
                eprintln!(
                    "real-peer replay downstream requests: {:?}",
                    requests
                        .iter()
                        .map(|request| request.path.as_str())
                        .collect::<Vec<_>>()
                );
                assert!(
                    !requests.iter().any(|request| {
                        matches!(
                            request.path.as_str(),
                            "/api/v1/game-information/query"
                                | "/api/v1/game-information/list"
                                | "/api/v1/game-information/detail"
                        )
                    }),
                    "replay must deliver the archived transcript without another content query"
                );
            } else {
                assert!(
                    requests
                        .iter()
                        .any(|request| request.path == "/api/v1/game-information/capabilities"),
                    "actual MCP capabilities call must cross the actual Gateway"
                );
                assert!(
                    requests
                        .iter()
                        .any(|request| request.path == "/api/v1/game-information/detail"),
                    "actual MCP live content query must cross the actual Gateway"
                );
                assert!(
                    requests.iter().any(|request| {
                        request.path == "/api/v1/game-information/detail"
                            && request.body["query"]["binding"]["mode"] == "live"
                            && request.body["query"]["parent_observation"]["state_generation"] == 0
                    }),
                    "actual MCP live detail query must carry the bootstrapped snapshot"
                );
                assert!(
                    requests.iter().any(|request| {
                        request.path == "/api/v1/game-information/live-observation-bootstrap"
                    }),
                    "actual MCP bootstrap must cross the actual Gateway and game-mod route"
                );
            }
            assert!(
                requests.iter().any(|request| {
                    request.path == "/api/v1/game-information/lookup-binding"
                        && request.correlation.as_deref()
                            == Some("game-information-binding-discovery")
                        && request.body["operation"] == "discovery"
                        && request.body["project_id"] == PROJECT
                        && request.body["run_id"] == RUN
                        && request.body["episode_id"] == EPISODE
                        && request.body["agent_id"] == AGENT
                        && request.body["authority_epoch"] == 1
                }),
                "actual MCP startup discovery must carry the adopted owner identity"
            );
        }

        let agent_events = read_json_lines(agent_log);
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
        let mcp_events = read_json_lines(mcp_log);
        if matches!(mode, EntryMode::Scripted) {
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
        }
        assertions::assert_archive_transcript(
            replay,
            matches!(mode, EntryMode::Scripted),
            &mcp_events,
            archive_path,
            &scope,
        );
    }
}

#[path = "runtime_v3_game_information_entry_assertions.rs"]
mod assertions;
#[path = "runtime_v3_game_information_entry_fixture_setup.rs"]
mod fixture_setup;
#[path = "runtime_v3_game_information_entry_gateway.rs"]
mod gateway;
#[path = "runtime_v3_game_information_entry_live_peers.rs"]
mod live_peers;
#[path = "runtime_v3_game_information_entry_peer.rs"]
mod peer;
#[path = "runtime_v3_game_information_entry_support.rs"]
mod support;

use gateway::serve_gateway;
use support::{
    explicit_revalidation_approval_and_adoption, finish_child, free_loopback_address,
    read_json_lines, start_runtime_child, wait_for_management, write_owner_config, write_private,
};
