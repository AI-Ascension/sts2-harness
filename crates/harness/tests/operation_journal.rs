// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use sts2_harness::{JournalDecision, JournalError, JournalKey, JournalOutcome, OperationJournal};

fn path(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("test clock is after the unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "sts2-harness-journal-{name}-{}-{nonce}.ndjson",
        std::process::id()
    ))
}

fn key(principal: &str, idempotency_key: &str) -> JournalKey {
    JournalKey {
        principal: principal.to_owned(),
        instance: "runtime:one".to_owned(),
        incarnation: "incarnation:one".to_owned(),
        operation: "capture".to_owned(),
        idempotency_key: idempotency_key.to_owned(),
    }
}

fn digest(seed: char) -> String {
    format!("sha256:{}", seed.to_string().repeat(64))
}

#[test]
fn identical_retries_replay_and_conflicting_retries_are_rejected() {
    let mut journal = OperationJournal::open(path("retry")).expect("journal opens");
    assert!(journal.is_empty());
    assert_eq!(
        journal
            .begin(key("alice", "k1"), &digest('a'))
            .expect("begin"),
        JournalDecision::Started(1)
    );
    let replay = journal
        .begin(key("alice", "k1"), &digest('a'))
        .expect("replay allowed");
    match replay {
        JournalDecision::Existing(entry) => {
            assert_eq!(entry.outcome, JournalOutcome::Pending);
            assert_eq!(entry.request_digest, digest('a'));
        }
        JournalDecision::Started(_) => panic!("identical retry must not restart"),
    }
    assert_eq!(
        journal
            .begin(key("alice", "k1"), &digest('b'))
            .expect_err("conflicting body"),
        JournalError::Conflict
    );
    journal
        .complete(&key("alice", "k1"), JournalOutcome::Accepted)
        .expect("complete");
    assert_eq!(
        journal.entry(&key("alice", "k1")).expect("entry").outcome,
        JournalOutcome::Accepted
    );
    assert_eq!(journal.len(), 1);
}

#[test]
fn keys_are_scoped_by_principal_instance_and_operation() {
    let mut journal = OperationJournal::open(path("scope")).expect("journal opens");
    journal
        .begin(key("alice", "k1"), &digest('a'))
        .expect("begin");
    let mut other_principal = key("bob", "k1");
    journal
        .begin(other_principal.clone(), &digest('a'))
        .expect("separate principal");
    other_principal.operation = "restore".to_owned();
    journal
        .begin(other_principal.clone(), &digest('a'))
        .expect("separate operation");
    other_principal.instance = "runtime:two".to_owned();
    journal
        .begin(other_principal.clone(), &digest('a'))
        .expect("separate instance");
    other_principal.incarnation = "incarnation:two".to_owned();
    journal
        .begin(other_principal, &digest('a'))
        .expect("separate incarnation");
    assert_eq!(journal.len(), 5);
}

#[test]
fn records_survive_reopen_and_a_torn_tail_is_discarded() {
    let path = path("recover");
    {
        let mut journal = OperationJournal::open(&path).expect("journal opens");
        journal
            .begin(key("alice", "k1"), &digest('a'))
            .expect("begin");
        journal
            .complete(&key("alice", "k1"), JournalOutcome::Unknown)
            .expect("uncertain outcome recorded");
    }
    let mut appended = OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("log reopens");
    appended
        .write_all(b"{\"key\":{\"princip")
        .expect("torn tail write");
    appended.flush().expect("flush");

    let journal = OperationJournal::open(&path).expect("replay ignores the torn tail");
    let entry = journal.entry(&key("alice", "k1")).expect("durable entry");
    assert_eq!(entry.outcome, JournalOutcome::Unknown);
    assert_eq!(entry.sequence, 1);
    assert_eq!(journal.len(), 1);
}

#[test]
fn invalid_inputs_and_unknown_completions_are_rejected() {
    let mut journal = OperationJournal::open(path("invalid")).expect("journal opens");
    assert_eq!(
        journal
            .begin(key("", "k1"), &digest('a'))
            .expect_err("empty principal"),
        JournalError::InvalidKey
    );
    assert_eq!(
        journal
            .begin(key("alice", "k1"), "not-a-digest")
            .expect_err("bad digest"),
        JournalError::InvalidDigest
    );
    assert_eq!(
        journal
            .complete(&key("alice", "missing"), JournalOutcome::Accepted)
            .expect_err("unknown attempt"),
        JournalError::Missing
    );
    let entry = journal
        .begin(key("alice", "k1"), &digest('a'))
        .expect("begin");
    assert_eq!(entry, JournalDecision::Started(1));
}

#[test]
fn corrupt_middle_records_are_reported() {
    let path = path("corrupt");
    {
        let mut journal = OperationJournal::open(&path).expect("journal opens");
        journal
            .begin(key("alice", "k1"), &digest('a'))
            .expect("begin");
    }
    let mut appended = OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("log reopens");
    appended.write_all(b"not-json\n").expect("corrupt write");
    appended
        .write_all(format!("{}\n", "x".repeat(8)).as_bytes())
        .expect("second line");
    appended.flush().expect("flush");
    assert_eq!(
        OperationJournal::open(&path).expect_err("corrupt middle record"),
        JournalError::Corrupt
    );
}

#[test]
fn torn_tail_recovery_remains_writable_across_restarts() {
    let path = path("tail-append");
    {
        let mut journal = OperationJournal::open(&path).unwrap();
        journal.begin(key("alice", "a"), &digest('a')).unwrap();
    }
    OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"{\"key\":")
        .unwrap();
    {
        let mut journal = OperationJournal::open(&path).unwrap();
        assert_eq!(
            journal.begin(key("alice", "b"), &digest('b')).unwrap(),
            JournalDecision::Started(2)
        );
        journal
            .complete(&key("alice", "a"), JournalOutcome::Accepted)
            .unwrap();
    }
    let journal = OperationJournal::open(&path).unwrap();
    assert_eq!(
        journal.entry(&key("alice", "a")).unwrap().outcome,
        JournalOutcome::Accepted
    );
    assert_eq!(
        journal.entry(&key("alice", "b")).unwrap().outcome,
        JournalOutcome::Pending
    );
}

#[test]
fn ownership_is_exclusive_and_released_on_drop() {
    let path = path("ownership");
    let owner = OperationJournal::open(&path).unwrap();
    assert_eq!(
        OperationJournal::open(&path).unwrap_err(),
        JournalError::Locked
    );
    drop(owner);
    assert!(OperationJournal::open(&path).is_ok());
}

#[test]
fn child_process_cannot_open_owned_journal() {
    const CHILD_PATH: &str = "STS2_JOURNAL_REVIEW_CHILD_PATH";
    if let Ok(path) = std::env::var(CHILD_PATH) {
        assert_eq!(
            OperationJournal::open(path).unwrap_err(),
            JournalError::Locked
        );
        return;
    }
    let path = path("process-owner");
    let _owner = OperationJournal::open(&path).unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "child_process_cannot_open_owned_journal"])
        .env(CHILD_PATH, &path)
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn terminal_outcomes_are_immutable_and_unknown_can_be_reconciled() {
    let path = path("terminal");
    {
        let mut journal = OperationJournal::open(&path).unwrap();
        let key = key("alice", "a");
        journal.begin(key.clone(), &digest('a')).unwrap();
        journal.complete(&key, JournalOutcome::Unknown).unwrap();
        journal.complete(&key, JournalOutcome::Accepted).unwrap();
        journal.complete(&key, JournalOutcome::Accepted).unwrap();
        for outcome in [
            JournalOutcome::Pending,
            JournalOutcome::Unknown,
            JournalOutcome::Rejected,
        ] {
            assert_eq!(
                journal.complete(&key, outcome),
                Err(JournalError::InvalidTransition)
            );
        }
    }
    let journal = OperationJournal::open(&path).unwrap();
    assert_eq!(
        journal.entry(&key("alice", "a")).unwrap().outcome,
        JournalOutcome::Accepted
    );
}

#[test]
fn replay_rejects_invalid_records_and_oversized_lines() {
    let valid = serde_json::json!({"key": key("alice", "a"), "request_digest": digest('a'), "outcome": "pending", "sequence": 1});
    let mut invalid = Vec::new();
    for sequence in [0, 2, u64::MAX] {
        let mut entry = valid.clone();
        entry["sequence"] = sequence.into();
        invalid.push(entry);
    }
    let mut entry = valid.clone();
    entry["key"]["principal"] = "".into();
    invalid.push(entry);
    let mut entry = valid.clone();
    entry["request_digest"] = "bad".into();
    invalid.push(entry);
    let mut entry = valid.clone();
    entry["outcome"] = "accepted".into();
    invalid.push(entry);
    for entry in invalid {
        let path = path("invalid-record");
        std::fs::write(&path, format!("{entry}\n")).unwrap();
        assert_eq!(
            OperationJournal::open(path).unwrap_err(),
            JournalError::Corrupt
        );
    }
    let path = path("oversized");
    std::fs::write(&path, vec![b'x'; 16 * 1024 + 1]).unwrap();
    assert_eq!(
        OperationJournal::open(path).unwrap_err(),
        JournalError::Capacity
    );
}

#[test]
fn replay_rejects_completed_corrupt_tail_and_terminal_overwrite() {
    let path = path("committed-corruption");
    std::fs::write(&path, b"not-json\n").unwrap();
    assert_eq!(
        OperationJournal::open(&path).unwrap_err(),
        JournalError::Corrupt
    );
    let pending = serde_json::json!({"key": key("alice", "a"), "request_digest": digest('a'), "outcome": "pending", "sequence": 1});
    let mut accepted = pending.clone();
    accepted["outcome"] = "accepted".into();
    let mut rejected = pending.clone();
    rejected["outcome"] = "rejected".into();
    std::fs::write(&path, format!("{pending}\n{accepted}\n{rejected}\n")).unwrap();
    assert_eq!(
        OperationJournal::open(path).unwrap_err(),
        JournalError::Corrupt
    );
}

#[test]
fn process_exit_preserves_authorization_without_rerunning_operation() {
    const CHILD_PATH: &str = "STS2_JOURNAL_CRASH_CHILD_PATH";
    if let Ok(path) = std::env::var(CHILD_PATH) {
        let mut journal = OperationJournal::open(path).unwrap();
        journal.begin(key("alice", "crash"), &digest('a')).unwrap();
        // Exit without dropping the live journal, after persistence but before an acknowledgement.
        std::process::exit(0);
    }
    let path = path("exit");
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "process_exit_preserves_authorization_without_rerunning_operation",
        ])
        .env(CHILD_PATH, &path)
        .status()
        .unwrap();
    assert!(status.success());
    let mut journal = OperationJournal::open(&path).unwrap();
    assert!(matches!(
        journal.begin(key("alice", "crash"), &digest('a')).unwrap(),
        JournalDecision::Existing(_)
    ));
}

#[test]
fn completion_crash_boundaries_recover_and_allow_reconciliation() {
    let record = serde_json::json!({"key": key("alice", "a"), "request_digest": digest('a'), "outcome": "accepted", "sequence": 1});
    let line = format!("{record}\n");
    for cut in [1, line.len() / 2, line.len() - 1, line.len()] {
        let path = path("completion-boundary");
        {
            let mut journal = OperationJournal::open(&path).unwrap();
            journal.begin(key("alice", "a"), &digest('a')).unwrap();
        }
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(&line.as_bytes()[..cut])
            .unwrap();
        {
            let mut journal = OperationJournal::open(&path).unwrap();
            let expected = if cut == line.len() {
                JournalOutcome::Accepted
            } else {
                JournalOutcome::Pending
            };
            assert_eq!(journal.entry(&key("alice", "a")).unwrap().outcome, expected);
            journal
                .complete(&key("alice", "a"), JournalOutcome::Accepted)
                .unwrap();
        }
        let journal = OperationJournal::open(&path).unwrap();
        assert_eq!(
            journal.entry(&key("alice", "a")).unwrap().outcome,
            JournalOutcome::Accepted
        );
    }
}

#[test]
fn replay_enforces_entry_capacity() {
    let path = path("replay-capacity");
    let mut records = String::new();
    for index in 1..=sts2_harness::MAX_JOURNAL_ENTRIES + 1 {
        let record = serde_json::json!({"key": key("alice", &index.to_string()), "request_digest": digest('a'), "outcome": "pending", "sequence": index});
        records.push_str(&format!("{record}\n"));
    }
    std::fs::write(&path, records).unwrap();
    assert_eq!(
        OperationJournal::open(path).unwrap_err(),
        JournalError::Capacity
    );
}
