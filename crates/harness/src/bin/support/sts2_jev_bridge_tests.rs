// SPDX-License-Identifier: MIT

//! Offline fixtures for the System One bridge.
//!
//! Every exchange here is a deterministic fake supplied through the transport parameter, so nothing
//! in this suite opens a socket, needs a credential, or depends on a provider being reachable.

use super::*;

/// A bridge request for a small combat turn.
fn request() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "model_execution_id": "model-execution-7",
        "objective": "survive the turn",
        "hard_constraints": ["never end the turn with unspent lethal"],
        "legal_action_ids": ["play:card-17", "play:card-18", "combat.end-turn"],
        "observation": {
            "state_id": "combat-1",
            "generation": 3,
            "player": {"hp": 30, "max_hp": 80, "energy": 3, "gold": 0, "hand": []},
            "state": {"state": "combat", "turn_index": 2},
        },
    }))
    .unwrap_or_default()
}

/// A provider response naming one of the presented options.
fn response(choice: &str, confidence: f64) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "model": "jev-1.13.0",
        "answers": {
            "action": {
                "type": "choice",
                "choice": choice,
                "probabilities": {
                    "play:card-17": 0.62,
                    "play:card-18": 0.21,
                    "combat.end-turn": 0.17,
                },
                "confidence": confidence,
            },
        },
        "usage": {"input_tokens": 900, "output_tokens": 0},
    }))
    .unwrap_or_default()
}

/// Decides against a fake transport, capturing the request body it was given.
fn decide_with(
    request: &[u8],
    reply: Result<Vec<u8>, &'static str>,
) -> (Result<Value, String>, Vec<u8>) {
    let mut seen = Vec::new();
    let outcome = decide(request, "jev-latest", &mut |body| {
        seen = body.to_vec();
        reply
            .clone()
            .map_err(|message| -> Box<dyn std::error::Error> { message.into() })
    })
    .map_err(|error| error.to_string());
    (outcome, seen)
}

#[test]
fn describe_reports_configuration_without_opening_a_connection() {
    let options = options::Options::parse(
        vec!["--describe", "--model", "jev-1.13.0"]
            .into_iter()
            .map(str::to_owned),
    )
    .expect("options");
    let described = describe(&options);
    assert_eq!(described["kind"], json!("systemone"));
    assert_eq!(described["provider"], json!("typesafe"));
    assert_eq!(described["model"], json!("jev-1.13.0"));
    assert_eq!(
        described["endpoint"],
        json!("https://api.typesafe.ai/v1/systemone")
    );
    assert_eq!(described["transport"], json!(null));
}

#[test]
fn a_confident_answer_becomes_one_action_decision() {
    let (decision, sent) = decide_with(&request(), Ok(response("play:card-17", 0.81)));
    let decision = decision.unwrap_or_default();
    assert_eq!(decision["decision"], json!("action"));
    assert_eq!(decision["action_id"], json!("play:card-17"));
    assert_eq!(decision["confidence"], json!(81));

    // The request the transport received carries the catalog as the option set and nothing else.
    let sent: Value = serde_json::from_slice(&sent).unwrap_or_default();
    assert_eq!(sent["model"], json!("jev-latest"));
    let keys: Vec<String> = sent["questions"]["action"]["criteria"]
        .as_object()
        .map(|map| map.keys().cloned().collect())
        .unwrap_or_default();
    assert_eq!(
        keys,
        vec![
            "combat.end-turn".to_owned(),
            "play:card-17".to_owned(),
            "play:card-18".to_owned(),
        ]
    );
    let instructions = sent["questions"]["action"]["instructions"]
        .as_str()
        .unwrap_or_default();
    assert!(instructions.contains("Objective: survive the turn"));
    assert!(instructions.contains("Constraint: never end the turn with unspent lethal"));
}

#[test]
fn an_unconfident_answer_asks_to_re_observe() {
    let (decision, _) = decide_with(&request(), Ok(response("play:card-17", 0.20)));
    let decision = decision.unwrap_or_default();
    assert_eq!(decision["decision"], json!("reobserve"));
    assert_eq!(decision.get("action_id"), None);
}

#[test]
fn an_answer_outside_the_catalog_is_refused() {
    let (decision, _) = decide_with(&request(), Ok(response("play:card-99", 0.99)));
    assert!(decision.is_err());
}

#[test]
fn a_transport_failure_or_malformed_reply_is_refused() {
    for reply in [
        Err("transport reported failure"),
        Ok(b"not json".to_vec()),
        Ok(serde_json::to_vec(&json!({"answers": {}})).unwrap_or_default()),
        Ok(vec![b'x'; LIMIT + 1]),
    ] {
        let (decision, _) = decide_with(&request(), reply);
        assert!(decision.is_err());
    }
}

#[test]
fn a_missing_or_oversized_catalog_is_refused_before_any_exchange() {
    let no_catalog =
        serde_json::to_vec(&json!({"observation": {"state_id": "s"}})).unwrap_or_default();
    let mut called = false;
    let outcome = decide(&no_catalog, "jev-latest", &mut |_| {
        called = true;
        Ok(Vec::new())
    });
    assert!(outcome.is_err());
    assert!(!called, "the transport must not run for an invalid catalog");

    let (oversized, _) = decide_with(&vec![b'x'; LIMIT + 1], Ok(Vec::new()));
    assert!(oversized.is_err());
}

/// Writes an executable shell fixture and returns its path.
#[cfg(unix)]
fn shell_fixture(name: &str, script: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = std::env::temp_dir().join(name);
    std::fs::write(&path, script).expect("write fixture");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod fixture");
    path
}

#[cfg(unix)]
#[test]
fn the_transport_runs_the_named_executable_and_returns_its_output() {
    let body = String::from_utf8(response("combat.end-turn", 0.9)).unwrap_or_default();
    let script = shell_fixture(
        "sts2-jev-transport-reply.sh",
        &format!("#!/bin/sh\ncat > /dev/null\nprintf '%s' '{body}'\n"),
    );
    let received = exchange(
        script.to_str().unwrap_or_default(),
        b"{}",
        Duration::from_secs(10),
    )
    .expect("exchange");
    let received: Value = serde_json::from_slice(&received).unwrap_or_default();
    assert_eq!(
        received["answers"]["action"]["choice"],
        json!("combat.end-turn")
    );
    let _ = std::fs::remove_file(&script);
}

#[cfg(unix)]
#[test]
fn a_transport_that_never_exits_is_killed_at_the_deadline() {
    let script = shell_fixture("sts2-jev-transport-hang.sh", "#!/bin/sh\nsleep 30\n");
    let started = Instant::now();
    let outcome = exchange(
        script.to_str().unwrap_or_default(),
        b"{}",
        Duration::from_millis(200),
    );
    assert!(outcome.is_err());
    assert!(started.elapsed() < Duration::from_secs(5));
    let _ = std::fs::remove_file(&script);
}

#[cfg(unix)]
#[test]
fn a_transport_that_exits_nonzero_is_refused() {
    let script = shell_fixture(
        "sts2-jev-transport-fail.sh",
        "#!/bin/sh\ncat > /dev/null\nexit 3\n",
    );
    let outcome = exchange(
        script.to_str().unwrap_or_default(),
        b"{}",
        Duration::from_secs(10),
    );
    assert!(outcome.is_err());
    let _ = std::fs::remove_file(&script);
}
