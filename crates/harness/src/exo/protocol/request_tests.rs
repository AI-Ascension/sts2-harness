// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use serde_json::{Value, json};

#[cfg(unix)]
use super::{EXO_MAX_MAP_REQUEST_BYTES, EXO_MAX_STANDARD_REQUEST_BYTES};
use super::{ExoDecisionRequest, ExoError};
use crate::episode::map::{MapDecisionContext, RUNTIME_MAP_PROFILE, RUNTIME_MAP_SCHEMA_DIGEST};
#[cfg(unix)]
use crate::exo::Decision;
use crate::exo::SanitizedObservation;
use crate::exo::{ExoConfig, ExoProvider, ExoSession, ExoTransport, ExoTransportError};
use crate::identity::ModelExecutionId;
#[cfg(unix)]
use crate::{ExoProcessConfig, ExoProcessTransport};

const REQUEST_BOUND: usize = 128 * 1024;
const REVISION: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn snapshot(edge_count: usize) -> Value {
    let mut nodes = vec![json!({
        "id":"start", "row":0, "column":0, "category":"start", "visited":true
    })];
    let mut node_ids = vec![String::from("start")];
    for index in 1..256 {
        let id = format!("z{index:03}{}", "a".repeat(123));
        node_ids.push(id.clone());
        nodes.push(json!({
            "id":id, "row":index, "column":0, "category":"monster", "visited":false
        }));
    }
    let mut edges = Vec::new();
    'outer: for from in 0..node_ids.len() {
        for to in (from + 1)..node_ids.len() {
            edges.push(json!({"from":node_ids[from],"to":node_ids[to]}));
            if edges.len() == edge_count {
                break 'outer;
            }
        }
    }
    json!({
        "state_id":"state-1", "generation":1, "schema_version":"visible-map-v1",
        "projection_version":"runtime-map-v1", "game_build":"build", "mod_version":"mod",
        "map_instance_id":"map-1", "act_id":1, "scope_id":"scope-1", "availability":"available",
        "completeness":"complete", "freshness":"current", "reason":null,
        "nodes":nodes, "edges":edges, "position":{"kind":"current","node_id":"start"},
        "history":["start"], "terminal_node_ids":[node_ids[255]],
        "bindings":[{"graph_node_id":node_ids[1],"host_action_id":"move-1",
            "action":{"kind":"select_map_node","node_id":node_ids[1]}}]
    })
}

fn map_context(edge_count: usize) -> MapDecisionContext {
    let snapshot = snapshot(edge_count);
    let digest = crate::sha256_hex(serde_json::to_vec(&snapshot).unwrap());
    let wrapper = json!({
        "profile":RUNTIME_MAP_PROFILE,
        "schema_digest":RUNTIME_MAP_SCHEMA_DIGEST,
        "snapshot_digest":digest,
        "snapshot":snapshot
    });
    MapDecisionContext::from_exo_value(&wrapper, "state-1", 1, &[String::from("move-1")])
        .expect("test map fixture must satisfy the map contract")
}

fn observation() -> SanitizedObservation {
    SanitizedObservation::new(json!({
        "state_id":"state-1", "generation":1, "visible_seed":null,
        "player":{"hp":50,"max_hp":50,"energy":3,"gold":99,
            "hand":[],"deck":[],"discard":[],"exhaust":[]},
        "state":{"state":"map","node_id":"start","options":["z001"]},
        "legal_actions":[{"action_id":"move-1",
            "action":{"kind":"select_map_node","node_id":"z001"}}]
    }))
    .expect("test observation must satisfy the fair-play contract")
}

#[test]
fn representative_complete_map_fits_the_128k_provider_bound() {
    let request = ExoDecisionRequest::new_with_map(
        ModelExecutionId::new(1).unwrap(),
        REVISION,
        "state-1",
        1,
        observation(),
        vec![String::from("move-1")],
        "choose a legal map node",
        Vec::new(),
        8 * 1024,
        map_context(1),
    )
    .unwrap();
    let encoded = request.encode(REQUEST_BOUND).unwrap();
    assert!(encoded.len() <= REQUEST_BOUND);
}

#[test]
fn map_request_bound_matches_snapshot_and_wire_overhead() {
    let context = map_context(1);
    let ordinary = ExoDecisionRequest::new(
        ModelExecutionId::new(1).unwrap(),
        REVISION,
        "state-1",
        1,
        observation(),
        vec![String::from("move-1")],
        "choose a legal map node",
        Vec::new(),
        8 * 1024,
    )
    .unwrap();
    let mapped = ExoDecisionRequest::new_with_map(
        ModelExecutionId::new(1).unwrap(),
        REVISION,
        "state-1",
        1,
        observation(),
        vec![String::from("move-1")],
        "choose a legal map node",
        Vec::new(),
        8 * 1024,
        context.clone(),
    )
    .unwrap();
    let ordinary_bytes = serde_json::to_vec(&ordinary).unwrap();
    let mapped_bytes = serde_json::to_vec(&mapped).unwrap();
    let wire = context.to_wire();
    let snapshot_bytes = serde_json::to_vec(&wire["snapshot"]).unwrap();
    assert_eq!(
        mapped_bytes.len() - ordinary_bytes.len(),
        snapshot_bytes.len() + crate::EXO_MAP_REQUEST_OVERHEAD_BYTES
    );
    assert_eq!(
        super::EXO_MAX_MAP_REQUEST_BYTES,
        super::EXO_MAX_STANDARD_REQUEST_BYTES + 256 * 1024 + crate::EXO_MAP_REQUEST_OVERHEAD_BYTES
    );
}

#[cfg(unix)]
#[test]
fn complete_map_over_standard_bound_reaches_fake_provider() -> Result<(), Box<dyn std::error::Error>>
{
    let bridge = ExoProcessConfig::new(
        "/bin/sh",
        vec![
            String::from("-c"),
            String::from(
                r#"request=$(cat) || exit 1; bytes=$(printf '%s' "$request" | wc -c); test "$bytes" -gt 131072 || exit 2; case "$request" in *'"schema":"sts2.exo-decision-map-v1"'*'"map_context":{"profile":"runtime-map-v1"'*) printf '%s' '{"decision":"action","action_id":"move-1","rationale":"large map round trip"}' ;; *) exit 2 ;; esac"#,
            ),
        ],
        None,
        Vec::new(),
    )?;
    let config = ExoConfig::new(REVISION, EXO_MAX_MAP_REQUEST_BYTES, 8 * 1024, 1_000)?;
    let provider = ExoProvider::new(ExoProcessTransport::new(bridge), config);
    let mut session = ExoSession::new(provider);
    let decision = session.decide_with_map(
        ModelExecutionId::new(1).ok_or("execution identity")?,
        "state-1",
        1,
        observation(),
        vec![String::from("move-1")],
        "choose a legal map node",
        Vec::new(),
        map_context(600),
    )?;
    assert_eq!(
        decision,
        Decision::Action {
            action_id: String::from("move-1"),
            rationale: String::from("large map round trip"),
            confidence: None,
        }
    );
    assert_eq!(EXO_MAX_STANDARD_REQUEST_BYTES, REQUEST_BOUND);
    Ok(())
}

#[derive(Default)]
struct CountingTransport {
    exchanges: usize,
}

impl ExoTransport for CountingTransport {
    fn exchange(
        &mut self,
        _request: &[u8],
        _max_response_bytes: usize,
        _timeout_millis: u32,
    ) -> Result<Vec<u8>, ExoTransportError> {
        self.exchanges += 1;
        Err(ExoTransportError::Unavailable)
    }

    fn close(&mut self) -> Result<(), ExoTransportError> {
        Ok(())
    }
}

#[test]
fn oversized_map_is_rejected_before_provider_transport() {
    let config = ExoConfig::new(REVISION, REQUEST_BOUND, 8 * 1024, 1_000).unwrap();
    let transport = CountingTransport::default();
    let provider = ExoProvider::new(transport, config);
    let mut session = ExoSession::new(provider);
    let result = session.decide_with_map(
        ModelExecutionId::new(1).unwrap(),
        "state-1",
        1,
        observation(),
        vec![String::from("move-1")],
        "choose a legal map node",
        Vec::new(),
        map_context(600),
    );
    assert_eq!(result, Err(ExoError::RequestTooLarge));
    assert_eq!(session.into_transport().exchanges, 0);
}

#[cfg(unix)]
#[test]
fn map_request_round_trips_as_serialized_bridge_input() -> Result<(), Box<dyn std::error::Error>> {
    let bridge = ExoProcessConfig::new(
        "/bin/sh",
        vec![
            String::from("-c"),
            String::from(
                r#"request=$(cat) || exit 1; case "$request" in *'"schema":"sts2.exo-decision-map-v1"'*'"map_context":{"profile":"runtime-map-v1"'*) printf '%s' '{"decision":"action","action_id":"move-1","rationale":"map round trip"}' ;; *) exit 2 ;; esac"#,
            ),
        ],
        None,
        Vec::new(),
    )?;
    let config = ExoConfig::new(REVISION, REQUEST_BOUND, 8 * 1024, 1_000)?;
    let provider = ExoProvider::new(ExoProcessTransport::new(bridge), config);
    let mut session = ExoSession::new(provider);
    let decision = session.decide_with_map(
        ModelExecutionId::new(1).ok_or("execution identity")?,
        "state-1",
        1,
        observation(),
        vec![String::from("move-1")],
        "choose a legal map node",
        Vec::new(),
        map_context(1),
    )?;
    assert_eq!(
        decision,
        Decision::Action {
            action_id: String::from("move-1"),
            rationale: String::from("map round trip"),
            confidence: None,
        }
    );
    Ok(())
}
