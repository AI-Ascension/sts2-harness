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
