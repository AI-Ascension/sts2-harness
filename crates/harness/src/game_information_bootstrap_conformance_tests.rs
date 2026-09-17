// SPDX-License-Identifier: MIT
//! Consumes the shared `game-information-live-observation-bootstrap-v1`
//! conformance vectors at the harness boundary. Fixture bytes stay identical to
//! the protocol artifact; a fixture that still carries the protocol's
//! pre-release schema digest is first refused as-is and then re-pinned so the
//! semantic reason the case names is exercised.
use super::*;
use crate::game_information_binding::game_information_bootstrap::{
    BootstrapError, PROFILE, SCHEMA_DIGEST, select_snapshot, validate_request,
};
use std::path::{Path, PathBuf};

const ARTIFACT_PREFIX: &str = "artifacts/game-information-live-observation-bootstrap-v1/";
const CASE_PATH: &str = "conformance/cases/game-information-live-observation-bootstrap-v1.json";
const FIXTURE_DIR: &str =
    "conformance/fixtures/game-information-live-observation-bootstrap-v1/invalid";
const BOOTSTRAP_TOOL: &str = "sts2.game_information.live_observation_bootstrap";
/// Untrusted producer text carried by the not-observable golden.
const UNTRUSTED_REASON: &str = "native snapshot identity is unavailable";

const EXPECTED_VALID_VECTORS: &[&str] = &[
    "BOOTSTRAP-VALID-REQUEST",
    "BOOTSTRAP-VALID-DISTINCT-DUPLICATE-DEFINITIONS",
    "BOOTSTRAP-VALID-NATIVE-UNAVAILABLE",
];

const EXPECTED_INVALID_VECTORS: &[&str] = &[
    "BOOTSTRAP-INVALID-FOREIGN-INSTANCE",
    "BOOTSTRAP-INVALID-DUPLICATE-VISIBLE-ENTITY",
    "BOOTSTRAP-INVALID-MISSING-NATIVE-REF",
    "BOOTSTRAP-INVALID-SELECTOR-INSTANCE-MISMATCH",
    "BOOTSTRAP-INVALID-STALE-GENERATION",
    "BOOTSTRAP-INVALID-CROSS-RUN",
    "BOOTSTRAP-INVALID-FOREIGN-MANIFEST",
];

fn artifact_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../protocol-artifact/game-information-live-observation-bootstrap-v1")
}

fn read_bytes(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))
}

fn read_json(path: &Path) -> Result<Value, String> {
    serde_json::from_slice(&read_bytes(path)?)
        .map_err(|error| format!("parse {}: {error}", path.display()))
}

/// Resolves a protocol-repository-relative vector path inside the pinned copy.
fn vector_path(vector: &Value, key: &str) -> Result<PathBuf, String> {
    let relative = vector[key]
        .as_str()
        .ok_or_else(|| format!("vector {key} path is missing"))?;
    let inner = relative.strip_prefix(ARTIFACT_PREFIX).unwrap_or(relative);
    if inner.starts_with('/') || inner.split('/').any(|segment| segment == "..") {
        return Err(format!(
            "vector path {relative} escapes the pinned artifact"
        ));
    }
    Ok(artifact_root().join(inner))
}

fn sorted_ids(vectors: &[Value]) -> Result<Vec<&str>, String> {
    let mut ids = vectors
        .iter()
        .map(|vector| {
            vector["id"]
                .as_str()
                .ok_or_else(|| String::from("vector id is invalid"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    ids.sort_unstable();
    Ok(ids)
}

fn file_names(directory: &Path) -> Result<Vec<String>, String> {
    let mut names = std::fs::read_dir(directory)
        .map_err(|error| format!("list {}: {error}", directory.display()))?
        .map(|entry| {
            entry
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .map_err(|error| format!("list {}: {error}", directory.display()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    names.sort_unstable();
    Ok(names)
}

/// Maps the shared case vocabulary onto the harness boundary error. Both
/// `invalid_binding` and `stale_snapshot` fail the scope fence: a visible entity
/// outside the attested scope, selector or parent generation never becomes a
/// binding. `invalid_request` names a malformed native reference.
fn expected_error(code: &str) -> Result<BootstrapError, String> {
    Ok(match code {
        "invalid_binding" | "stale_snapshot" => BootstrapError::Scope,
        "invalid_request" => BootstrapError::Invalid,
        _ => return Err(format!("unexpected shared error code {code}")),
    })
}

fn mcp_context() -> LookupMcpContext {
    LookupMcpContext {
        instance_id: "instance-1".to_owned(),
        mcp_session_id: "mcp-1".to_owned(),
        lease_id: "lease-1".to_owned(),
        lease_epoch: 7,
    }
}

/// The owner replaces the golden's placeholder correlation with its RPC id.
fn owner_request(request: &Value, id: u64) -> Value {
    let mut owned = request.clone();
    owned["correlation_id"] = json!(id.to_string());
    owned
}

/// Wraps producer bytes as the MCP tool result the owner port receives.
fn tool_result(id: u64, is_error: bool, text: &[u8]) -> Result<Value, LookupError> {
    let text = std::str::from_utf8(text).map_err(|_| LookupError::Invalid)?;
    Ok(json!({"jsonrpc":"2.0","id":id,"result":{"isError":is_error,
        "content":[{"type":"text","text":text}]}}))
}

#[test]
fn all_shared_live_bootstrap_vectors_are_consumed_at_the_boundary() -> Result<(), String> {
    let root = artifact_root();
    let case = read_json(&root.join(CASE_PATH))?;
    assert_eq!(
        case["case_id"],
        "CT-GAME-INFORMATION-LIVE-OBSERVATION-BOOTSTRAP-V1-001"
    );
    assert_eq!(case["profile"], PROFILE);
    assert!(
        case["consumers"]
            .as_array()
            .is_some_and(|consumers| consumers.iter().any(|name| name == "sts2-harness")),
        "the shared case names the harness as a consumer"
    );
    let valid = case["valid_vectors"]
        .as_array()
        .ok_or_else(|| String::from("valid vector index is missing"))?;
    let invalid = case["invalid_vectors"]
        .as_array()
        .ok_or_else(|| String::from("invalid vector index is missing"))?;
    let mut expected_valid = EXPECTED_VALID_VECTORS.to_vec();
    let mut expected_invalid = EXPECTED_INVALID_VECTORS.to_vec();
    expected_valid.sort_unstable();
    expected_invalid.sort_unstable();
    assert_eq!(sorted_ids(valid)?, expected_valid);
    assert_eq!(sorted_ids(invalid)?, expected_invalid);

    let mut named_fixtures = invalid
        .iter()
        .map(|vector| {
            vector_path(vector, "fixture").and_then(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .ok_or_else(|| String::from("fixture path has no file name"))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    named_fixtures.sort_unstable();
    assert_eq!(
        file_names(&root.join(FIXTURE_DIR))?,
        named_fixtures,
        "every invalid fixture on disk is named by the shared case"
    );
    let mut named_goldens = valid
        .iter()
        .flat_map(|vector| ["golden", "query"].map(|key| (vector, key)))
        .filter(|(vector, key)| vector[*key].is_string())
        .map(|(vector, key)| {
            vector_path(vector, key).and_then(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .ok_or_else(|| String::from("golden path has no file name"))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    named_goldens.sort_unstable();
    assert_eq!(
        file_names(&root.join("golden"))?,
        named_goldens,
        "every golden on disk is named by the shared case"
    );

    let request_vector = valid
        .iter()
        .find(|vector| vector["id"] == "BOOTSTRAP-VALID-REQUEST")
        .ok_or_else(|| String::from("shared request vector is missing"))?;
    let request = read_json(&vector_path(request_vector, "golden")?)?;
    for vector in valid {
        let id = vector["id"]
            .as_str()
            .ok_or_else(|| String::from("valid vector id is invalid"))?;
        let golden_path = vector_path(vector, "golden")?;
        let golden = read_json(&golden_path)?;
        match id {
            "BOOTSTRAP-VALID-REQUEST" => {
                validate_request(&golden).map_err(|error| format!("{id}: {error}"))?;
                assert_eq!(golden["kind"], "bootstrap_request", "{id}");
            }
            "BOOTSTRAP-VALID-DISTINCT-DUPLICATE-DEFINITIONS" => {
                let query = read_json(&vector_path(vector, "query")?)?;
                assert_eq!(
                    select_snapshot(&request, &golden),
                    Err(BootstrapError::Ambiguous),
                    "{id}: a wildcard over two occurrences is refused"
                );
                let occurrence = query["query"]["binding"]["instance_ref"].clone();
                let mut selected_request = request.clone();
                selected_request["selector"]["instance_ref"] = occurrence.clone();
                let mut selected_response = golden.clone();
                selected_response["selector"]["instance_ref"] = occurrence.clone();
                let snapshot = select_snapshot(&selected_request, &selected_response)
                    .map_err(|error| format!("{id}: {error}"))?;
                assert_eq!(snapshot["instance_ref"], occurrence, "{id}");
                assert_eq!(
                    snapshot, query["query"]["binding"]["snapshot_ref"],
                    "{id}: the selected snapshot is the unchanged query-v1 binding"
                );
                assert_eq!(
                    query["query"]["parent_observation"]["state_generation"],
                    golden["parent_observation"]["state_generation"],
                    "{id}"
                );
            }
            "BOOTSTRAP-VALID-NATIVE-UNAVAILABLE" => {
                let bytes = read_bytes(&golden_path)?;
                assert_eq!(golden["error"]["code"], "not_observable", "{id}");
                let result = call_live_observation_bootstrap_mcp(
                    &mcp_context(),
                    &owner_request(&request, 1),
                    |rpc_id, _| tool_result(rpc_id, true, &bytes),
                );
                assert_eq!(result, Err(LookupError::MissingCapability), "{id}");
            }
            _ => return Err(format!("{id} has no boundary consumer")),
        }
    }

    for vector in invalid {
        let id = vector["id"]
            .as_str()
            .ok_or_else(|| String::from("invalid vector id is invalid"))?;
        let expected = expected_error(
            vector["expected_error"]
                .as_str()
                .ok_or_else(|| String::from("invalid expected error is missing"))?,
        )?;
        assert!(vector["schema_valid"].is_boolean(), "{id} schema flag");
        let fixture = read_json(&vector_path(vector, "fixture")?)?;
        assert_eq!(
            fixture["correlation_id"], request["correlation_id"],
            "{id} pairs with the shared request"
        );
        let mut pinned = fixture.clone();
        if fixture["schema_digest"] != SCHEMA_DIGEST {
            assert_eq!(
                select_snapshot(&request, &fixture),
                Err(BootstrapError::Invalid),
                "{id}: an unpinned schema digest never reaches semantic checks"
            );
            pinned["schema_digest"] = json!(SCHEMA_DIGEST);
        }
        assert_eq!(select_snapshot(&request, &pinned), Err(expected), "{id}");
    }
    Ok(())
}

#[test]
fn not_observable_bootstrap_error_is_missing_capability_and_never_reaches_the_agent() -> TestResult
{
    struct GoldenErrorMcp {
        golden: Vec<u8>,
        scope: Value,
        calls: usize,
    }
    impl LookupMcpPort for GoldenErrorMcp {
        fn information_correlation(&self) -> Result<String, LookupError> {
            Ok("1".to_owned())
        }
        fn call_information(
            &mut self,
            _tool: &str,
            _request: &Value,
        ) -> Result<Vec<u8>, LookupError> {
            Err(LookupError::MissingCapability)
        }
        fn call_live_observation_bootstrap(
            &mut self,
            request: &Value,
        ) -> Result<Vec<u8>, LookupError> {
            self.calls += 1;
            // The owner supplies the authenticated scope and RPC correlation.
            let mut body = request.clone();
            body["scope"] = self.scope.clone();
            let golden = self.golden.clone();
            call_live_observation_bootstrap_mcp(
                &mcp_context(),
                &owner_request(&body, 1),
                |id, arguments| {
                    assert_eq!(arguments["name"], BOOTSTRAP_TOOL);
                    tool_result(id, true, &golden)
                },
            )
        }
    }
    struct ObservingAgent {
        definition: Value,
        turns: usize,
        saw_typed_error: bool,
    }
    impl LookupAgentPort for ObservingAgent {
        fn next_turn(&mut self, input: LookupAgentInput<'_>) -> Result<LookupTurn, LookupError> {
            self.turns += 1;
            assert!(
                input.binding.snapshot.is_none(),
                "an error response never installs a snapshot"
            );
            match input.feedback {
                LookupFeedback::Start => {
                    let request =
                        crate::game_information_binding::game_information_bootstrap::request(
                            "pending",
                            Value::Null,
                            self.definition.clone(),
                            None,
                        );
                    Ok(LookupTurn::Bootstrap {
                        operation_id: "bootstrap".to_owned(),
                        request: serde_json::to_vec(&request).map_err(|_| LookupError::Invalid)?,
                    })
                }
                LookupFeedback::Error(LookupError::MissingCapability) => {
                    self.saw_typed_error = true;
                    Ok(LookupTurn::Decide {
                        action_id: "play:card-17".to_owned(),
                    })
                }
                LookupFeedback::Error(other) => Err(other.clone()),
                // Any producer payload delivered after an error is a boundary breach.
                LookupFeedback::Bootstrap { .. }
                | LookupFeedback::Data { .. }
                | LookupFeedback::Bytes { .. } => Err(LookupError::Divergence),
            }
        }
    }

    let golden_bytes = read_bytes(&artifact_root().join("golden/error-native-unavailable.json"))?;
    let golden: Value = serde_json::from_slice(&golden_bytes)?;
    assert_eq!(golden["kind"], "error_response");
    assert_eq!(golden["error"]["code"], "not_observable");
    assert_eq!(golden["error"]["reason"], UNTRUSTED_REASON);
    let (mut session, mut corpus) = setup(8192)?;
    let legal = crate::EpisodeLegalActionSet::new(
        "state-42",
        42,
        vec![crate::EpisodeLegalAction::new(
            "play:card-17",
            crate::ActionKind::PlayCard,
        )?],
    )?;
    let mut port = GoldenErrorMcp {
        golden: golden_bytes,
        scope: golden["scope"].clone(),
        calls: 0,
    };
    let mut agent = ObservingAgent {
        definition: golden["selector"]["definition_ref"].clone(),
        turns: 0,
        saw_typed_error: false,
    };
    assert_eq!(
        run_lookup_tool_loop(&mut session, &mut corpus, &mut port, &mut agent, &legal, 3)?,
        "play:card-17"
    );
    assert_eq!(
        port.calls, 1,
        "one bootstrap call, no retry with a different build"
    );
    assert_eq!(agent.turns, 2);
    assert!(agent.saw_typed_error, "the agent sees only the typed error");
    assert!(session.binding.snapshot.is_none());
    assert!(session.records.is_empty(), "no query record is produced");
    let records = session.bootstrap_records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].error, Some(LookupError::MissingCapability));
    assert!(records[0].response.is_none());
    let retained = serde_json::to_string(records)?;
    for untrusted in [
        UNTRUSTED_REASON,
        "not_observable",
        "corr-bootstrap-unavailable",
    ] {
        assert!(
            !retained.contains(untrusted),
            "retained transcript leaks {untrusted}"
        );
    }
    Ok(())
}
