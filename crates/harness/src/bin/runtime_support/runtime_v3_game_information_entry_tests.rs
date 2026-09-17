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
        verification::verify_entry_result(verification::EntryOutcome {
            output: &output,
            gateway_requests: gateway_result,
            live_requests,
            agent_log,
            mcp_log,
            archive_path,
            scope: &scope,
            replay,
            scripted: matches!(mode, EntryMode::Scripted),
        });
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
#[path = "runtime_v3_game_information_entry_tests_verification.rs"]
mod verification;

use gateway::serve_gateway;
use support::{
    explicit_revalidation_approval_and_adoption, finish_child, free_loopback_address,
    read_json_lines, start_runtime_child, wait_for_management, write_owner_config, write_private,
};
