// SPDX-License-Identifier: MIT

use super::live_peers::LoggedRequest;
use super::{AGENT, EPISODE, PROJECT, RUN};
use serde_json::json;
use std::path::Path;
use std::process::Output;
use sts2_harness::context_memory::MemoryScope;

pub(super) struct EntryOutcome<'a> {
    pub(super) output: &'a Output,
    pub(super) gateway_requests: Option<Result<Vec<String>, String>>,
    pub(super) live_requests: Option<Vec<LoggedRequest>>,
    pub(super) agent_log: &'a Path,
    pub(super) mcp_log: &'a Path,
    pub(super) archive_path: &'a Path,
    pub(super) scope: &'a MemoryScope,
    pub(super) replay: bool,
    pub(super) scripted: bool,
}

pub(super) fn verify_entry_result(outcome: EntryOutcome<'_>) {
    let EntryOutcome {
        output,
        gateway_requests,
        live_requests,
        agent_log,
        mcp_log,
        archive_path,
        scope,
        replay,
        scripted,
    } = outcome;
    assert!(
        output.status.success(),
        "runtime entry failed:\n{}\n{}\nMCP log:\n{}\nAgent log:\n{}\nGateway result: {:?}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&std::fs::read(mcp_log).unwrap_or_default()),
        String::from_utf8_lossy(&std::fs::read(agent_log).unwrap_or_default()),
        gateway_requests
    );
    if let Some(requests) = gateway_requests {
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
                    && request.correlation.as_deref() == Some("game-information-binding-discovery")
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

    let agent_events = super::support::read_json_lines(agent_log);
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
    let mcp_events = super::support::read_json_lines(mcp_log);
    if scripted {
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
    super::assertions::assert_archive_transcript(
        replay,
        scripted,
        &mcp_events,
        archive_path,
        scope,
    );
}
