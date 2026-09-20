// SPDX-License-Identifier: MIT

//! MIT synthetic fixtures. No credentials, provider sockets, host processes or game data.

use super::*;
use serde_json::json;

fn input(execution: &str) -> Value {
    json!({
        "model_execution_id": execution, "objective": "synthetic objective",
        "hard_constraints": [], "legal_action_ids": ["action-00", "action-01"],
        "observation": {
            "state_id": "synthetic-state", "generation": 3,
            "player": {"hp": 30, "max_hp": 80, "energy": 3, "gold": 0, "hand": []},
            "state": {"state": "combat", "turn_index": 2},
        },
    })
}

fn options(tactical: bool) -> options::Options {
    let mut args = vec!["--model", "jev-1.13.0", "--gate", "20"];
    if tactical {
        args.push("--tactical");
    }
    options::Options::parse(args.into_iter().map(str::to_owned)).expect("synthetic options")
}

#[test]
fn capture_identity_separates_execution_and_comparable_input() {
    let first = projection::Identity::new(&input("execution-a"), &options(false), &"a".repeat(64))
        .expect("first")
        .pending();
    let second = projection::Identity::new(&input("execution-b"), &options(true), &"a".repeat(64))
        .expect("second")
        .pending();
    assert_eq!(first["input_digest"], second["input_digest"]);
    assert_ne!(
        first["model_execution_id_digest"],
        second["model_execution_id_digest"]
    );
    assert_eq!(first["provider_attempts"], Value::Null);
    assert!(!first.to_string().contains("synthetic objective"));
    assert!(!first.to_string().contains("execution-a"));
}

#[test]
fn capture_identity_changes_when_material_input_changes() {
    let source = input("execution-a");
    let identity = projection::Identity::new(&source, &options(false), &"a".repeat(64))
        .expect("identity")
        .pending();
    for key in ["objective", "hard_constraints", "observation"] {
        let mut changed = source.clone();
        changed[key] = json!("changed");
        let other = projection::Identity::new(&changed, &options(false), &"a".repeat(64))
            .expect("changed identity")
            .pending();
        assert_ne!(identity["input_digest"], other["input_digest"]);
    }
}

#[test]
fn capture_refuses_missing_execution_and_duplicate_catalog() {
    let mut missing = input("execution-a");
    missing
        .as_object_mut()
        .expect("object")
        .remove("model_execution_id");
    assert!(projection::Identity::new(&missing, &options(false), &"a".repeat(64)).is_err());
    let mut duplicate = input("execution-a");
    duplicate["legal_action_ids"] = json!(["action-00", "action-00"]);
    assert!(projection::Identity::new(&duplicate, &options(false), &"a".repeat(64)).is_err());
}

#[test]
fn capture_failure_never_carries_provider_error_text() {
    let identity =
        projection::Identity::new(&input("execution-a"), &options(false), &"a".repeat(64))
            .expect("identity");
    let failed = identity.failed(1, 15);
    assert_eq!(failed["status"], "failed");
    assert_eq!(failed["provider_attempts"], 1);
    assert!(failed.get("provider").is_none());
    assert!(failed.get("decision").is_none());
}

#[test]
fn capture_projection_matches_the_shared_synthetic_golden() {
    let input = json!({"model_execution_id": "execution-baseline",
        "legal_action_ids": ["action-00", "action-01"], "observation": {},
        "objective": "synthetic", "hard_constraints": []});
    let body = json!({"model": "jev-1.13.0", "state": "synthetic",
        "questions": {"action": {"type": "choice", "instructions": "synthetic",
        "criteria": {"action-00": "first", "action-01": "second"}}}});
    let record = json!({"provider_call": true, "provider_request": body,
        "provider_response": {"model": "jev-1.13.0", "usage": {"input_tokens": 7}},
        "decision": {"decision": "action", "action_id": "action-00",
        "rationale": "PRIVATE_SENTINEL", "confidence": 90}});
    let identity =
        projection::Identity::new(&input, &options(false), &"a".repeat(64)).expect("identity");
    let actual = identity.complete(&record, 1, 12).expect("projection");
    let expected: Value =
        serde_json::from_str(include_str!("jev_capture_golden.json")).expect("golden");
    assert_eq!(actual, expected);
    assert!(!actual.to_string().contains("PRIVATE_SENTINEL"));
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    pub(super) struct Scratch(pub PathBuf);

    impl Scratch {
        pub(super) fn new() -> Self {
            use std::os::unix::fs::PermissionsExt as _;
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/jev-capture-tests")
                .join(format!(
                    "{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::SeqCst)
                ));
            std::fs::create_dir_all(&path).expect("scratch");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).expect("mode");
            Self(std::fs::canonicalize(path).expect("canonical path"))
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn captured_options(root: &Scratch, tactical: bool) -> options::Options {
        let mut options = options(tactical);
        options.audit_dir = Some(root.0.to_str().expect("path").to_owned());
        options
    }

    fn result(root: &Scratch) -> Value {
        serde_json::from_slice(
            &std::fs::read(root.0.join("attempt-0000.result.json")).expect("capture result"),
        )
        .expect("capture json")
    }

    #[test]
    fn capture_keeps_the_normal_decision_and_makes_exactly_one_exchange() {
        for tactical in [false, true] {
            let root = Scratch::new();
            let options = captured_options(&root, tactical);
            let bytes = serde_json::to_vec(&input("execution-a")).expect("input");
            let mut calls = 0;
            let decision = execute(&bytes, &options, &"a".repeat(64), &mut |body| {
                calls += 1;
                assert!(root.0.join("attempt-0000.pending.json").exists());
                let body: Value = serde_json::from_slice(body)?;
                Ok(serde_json::to_vec(
                    &super::super::super::tactical_fixture::reply(&body, "action-01"),
                )?)
            })
            .expect("capture");
            assert_eq!(calls, 1);
            assert_eq!(decision["decision"], "action");
            let captured = result(&root);
            assert_eq!(captured["provider_attempts"], 1);
            assert_eq!(
                captured["decision"]["selected_index"],
                if tactical { 1 } else { 0 }
            );
            assert!(
                decision.get("schema").is_none(),
                "stdout remains a decision, not a capture"
            );
            for forbidden in [
                "synthetic objective",
                "action-00",
                "execution-a",
                "rationale",
            ] {
                assert!(!captured.to_string().contains(forbidden));
            }
        }
    }

    #[test]
    fn provider_failure_is_captured_without_retry_or_payload() {
        let root = Scratch::new();
        let options = captured_options(&root, false);
        let mut calls = 0;
        let failed = execute(
            &serde_json::to_vec(&input("execution-a")).expect("input"),
            &options,
            &"a".repeat(64),
            &mut |_| {
                calls += 1;
                Err("PRIVATE_PROVIDER_ERROR".into())
            },
        );
        assert!(failed.is_err());
        assert_eq!(calls, 1);
        let captured = result(&root);
        assert_eq!(captured["status"], "failed");
        assert_eq!(captured["provider_attempts"], 1);
        assert!(!captured.to_string().contains("PRIVATE_PROVIDER_ERROR"));
    }

    #[test]
    fn invalid_capture_directory_prevents_provider_egress() {
        use std::os::unix::fs::PermissionsExt as _;
        let root = Scratch::new();
        std::fs::set_permissions(&root.0, std::fs::Permissions::from_mode(0o755)).expect("mode");
        let options = captured_options(&root, false);
        let mut calls = 0;
        assert!(
            execute(
                &serde_json::to_vec(&input("execution-a")).expect("input"),
                &options,
                &"a".repeat(64),
                &mut |_| {
                    calls += 1;
                    Ok(Vec::new())
                }
            )
            .is_err()
        );
        assert_eq!(calls, 0);
    }
}
