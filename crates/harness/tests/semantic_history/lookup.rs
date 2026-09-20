// SPDX-License-Identifier: MIT

//! Lookup: the harness-owned agent tool port, its bounds, its disclosure and its refusals.

use super::*;
use sts2_harness::semantic_history::{
    RetainedHistoryLookup, SEMANTIC_LOOKUP_TOOL, SEMANTIC_MAX_LOOKUP_REQUEST_BYTES,
    SemanticLookupAuthority, SemanticLookupDetail, SemanticLookupPort, SemanticLookupRequest,
    SemanticPruneRequest, SemanticRetentionPolicy,
};

fn lookup_fence(branch: &str) -> SemanticHistoryFence {
    SemanticHistoryFence {
        run_id: "run-1".to_owned(),
        branch_id: branch.to_owned(),
        episode: 1,
        epoch: 4,
    }
}

fn request(branch: &str, limit: usize) -> SemanticLookupRequest {
    SemanticLookupRequest {
        operation_id: "lookup-1".to_owned(),
        fence: lookup_fence(branch),
        binding: binding(),
        query: SemanticEventListQuery {
            limit,
            ..SemanticEventListQuery::default()
        },
    }
}

/// A history with an observed card play, a damage caused by it, and a disclosed gap.
fn populated() -> SemanticHistoryStore {
    let mut b = batch(
        "b1",
        1,
        vec![
            card_played("e1", 1),
            damage("e2", 2, Some("e1")),
            gap("g3", 3, SemanticCoverageStatus::Dropped),
        ],
    );
    b.window.intervals = vec![dropped(3, 3)];
    let mut store = SemanticHistoryStore::in_memory(unique("populated"));
    store
        .append(&SemanticHistoryAppend {
            operation_id: "op-1".to_owned(),
            binding: binding(),
            batch: b,
        })
        .expect("history lands");
    store
}

fn granted(store: &SemanticHistoryStore) -> RetainedHistoryLookup<'_> {
    RetainedHistoryLookup::new(store, SemanticLookupAuthority::Granted)
}

/// A path no other test shares: the store commits to disk even when opened in memory.
fn unique(name: &str) -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let mut root = std::env::temp_dir();
    root.push(format!(
        "semantic-history-lookup-{name}-{}-{}.json",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    root
}

#[test]
fn lookup_is_refused_until_the_harness_grants_the_port() {
    let store = populated();
    let mut port = RetainedHistoryLookup::new(&store, SemanticLookupAuthority::NotGranted);
    assert_eq!(port.authority(), SemanticLookupAuthority::NotGranted);
    let refused = port
        .lookup(&request("b1", 8), "corr-1")
        .expect_err("an ungranted port serves nothing");
    assert_eq!(refused.refusal, SemanticHistoryRefusal::LookupNotGranted);
    assert_eq!(port.served_operations(), 0);
}

#[test]
fn an_observed_record_returns_its_stated_detail_and_causal_parent() {
    let store = populated();
    let mut port = granted(&store);
    let page = port
        .lookup(&request("b1", 8), "corr-1")
        .expect("page served");
    assert_eq!(page.tool, SEMANTIC_LOOKUP_TOOL);
    assert!(!page.replayed);
    assert_eq!(page.matched, 3);
    let item = &page.items[1];
    assert_eq!(item.event_id, "e2");
    assert_eq!(item.kind, Some(SemanticEventKind::Damage));
    assert_eq!(item.origin, Some(SemanticEventOrigin::Native));
    assert_eq!(item.causal_parent.as_deref(), Some("e1"));
    match &item.detail {
        SemanticLookupDetail::Observed(record) => {
            assert_eq!(record.event.value.as_ref().expect("a quantity").amount, 6);
        }
        other => panic!("an observed record must carry its detail, got {other:?}"),
    }
}

#[test]
fn a_record_without_a_cause_states_none_rather_than_an_inferred_one() {
    let store = populated();
    let mut port = granted(&store);
    let page = port
        .lookup(&request("b1", 8), "corr-1")
        .expect("page served");
    assert_eq!(page.items[0].event_id, "e1");
    assert_eq!(page.items[0].causal_parent, None);
}

#[test]
fn a_disclosed_gap_is_returned_as_a_gap_not_as_an_event() {
    let store = populated();
    let mut port = granted(&store);
    let page = port
        .lookup(&request("b1", 8), "corr-1")
        .expect("page served");
    let item = &page.items[2];
    assert_eq!(item.event_id, "g3");
    assert_eq!(item.coverage, SemanticCoverageStatus::Dropped);
    assert_eq!(item.kind, None);
    assert_eq!(item.origin, None);
    assert_eq!(item.detail, SemanticLookupDetail::Gap);
}

#[test]
fn a_lookup_discloses_the_window_the_history_does_not_hold() {
    let store = populated();
    let mut port = granted(&store);
    let page = port
        .lookup(&request("b1", 8), "corr-1")
        .expect("page served");
    assert_eq!(page.coverage.capture_start_sequence, 1);
    assert!(!page.coverage.history_before_capture);
    assert_eq!(page.coverage.branch_id, "b1");
    assert_eq!(page.coverage.parent_branch_id, None);
    assert_eq!(page.coverage.intervals, vec![dropped(3, 3)]);
}

#[test]
fn a_page_over_its_result_bound_is_refused_rather_than_truncated() {
    let mut events = Vec::new();
    for n in 1..=64 {
        let mut event = card_played(&format!("e{n}"), n);
        event.label = Some("x".repeat(1024));
        events.push(event);
    }
    let mut b = batch("b1", 1, events);
    b.window.intervals = Vec::new();
    let mut store = SemanticHistoryStore::in_memory(unique("bytes"));
    store
        .append(&SemanticHistoryAppend {
            operation_id: "op-1".to_owned(),
            binding: binding(),
            batch: b,
        })
        .expect("history lands");
    let mut port = granted(&store);
    let refused = port
        .lookup(&request("b1", 64), "corr-1")
        .expect_err("a page over the result bound is refused, not shortened");
    assert_eq!(
        refused.refusal,
        SemanticHistoryRefusal::LookupPayloadTooLarge
    );
    let narrowed = port
        .lookup(&request("b1", 8), "corr-2")
        .expect("a narrowed page fits");
    assert_eq!(narrowed.items.len(), 8);
    assert!(
        narrowed.result_bytes <= sts2_harness::semantic_history::SEMANTIC_MAX_LOOKUP_RESULT_BYTES
    );
    assert_eq!(narrowed.matched, 64);
}

#[test]
fn a_re_delivered_lookup_is_recognised_and_a_reused_identity_is_refused() {
    let store = populated();
    let mut port = granted(&store);
    let first = port.lookup(&request("b1", 8), "corr-1").expect("first");
    assert!(!first.replayed);
    let second = port
        .lookup(&request("b1", 8), "corr-2")
        .expect("re-delivery");
    assert!(second.replayed);
    assert_eq!(port.served_operations(), 1);
    let mut conflict = request("b1", 8);
    conflict.query.limit = 2;
    let refused = port
        .lookup(&conflict, "corr-3")
        .expect_err("a reused identity with a different request is refused");
    assert_eq!(refused.refusal, SemanticHistoryRefusal::IdempotencyConflict);
}

#[test]
fn a_request_naming_another_run_branch_or_binding_is_refused() {
    let store = populated();
    let mut port = granted(&store);
    let mut other_branch = request("b1", 8);
    other_branch.fence.branch_id = "b2".to_owned();
    let refused = port
        .lookup(&other_branch, "corr-1")
        .expect_err("unknown branch");
    assert_eq!(refused.refusal, SemanticHistoryRefusal::UnknownBranch);

    let mut other_run = request("b1", 8);
    other_run.fence.run_id = "run-9".to_owned();
    let refused = port.lookup(&other_run, "corr-2").expect_err("stale fence");
    assert_eq!(refused.refusal, SemanticHistoryRefusal::StaleFence);

    let mut other_binding = request("b1", 8);
    other_binding.binding.manifest_digest = "manifest-digest-b".to_owned();
    let refused = port
        .lookup(&other_binding, "corr-3")
        .expect_err("another manifest");
    assert_eq!(refused.refusal, SemanticHistoryRefusal::BindingMismatch);
}

#[test]
fn a_stale_fence_is_refused_even_when_the_history_holds_no_records() {
    let mut store = SemanticHistoryStore::in_memory(unique("empty"));
    let mut empty = batch("b2", 1, Vec::new());
    empty.window.intervals = Vec::new();
    store
        .append(&SemanticHistoryAppend {
            operation_id: "op-0".to_owned(),
            binding: binding(),
            batch: empty,
        })
        .expect("a history with no records is still retained");

    let mut port = granted(&store);
    let mut stale = request("b2", 8);
    stale.fence.episode = 7;
    let refused = port
        .lookup(&stale, "corr-1")
        .expect_err("with no record to refuse this read, only the retained scope can");
    assert_eq!(refused.refusal, SemanticHistoryRefusal::StaleFence);

    let page = port.lookup(&request("b2", 8), "corr-2").expect("served");
    assert!(page.items.is_empty());
    assert_eq!(page.matched, 0);
    assert_eq!(page.coverage.capture_start_sequence, 1);
    assert_eq!(page.coverage.branch_id, "b2");
}

#[test]
fn a_path_shaped_operation_identity_is_refused_before_the_store_is_read() {
    let store = populated();
    let mut port = granted(&store);
    let mut traversal = request("b1", 8);
    traversal.operation_id = "../escape".to_owned();
    let refused = port
        .lookup(&traversal, "corr-1")
        .expect_err("a traversal identity is refused");
    assert_eq!(refused.refusal, SemanticHistoryRefusal::Identity);

    let mut unbounded = request("b1", 8);
    unbounded.operation_id = "o".repeat(257);
    let refused = port
        .lookup(&unbounded, "corr-2")
        .expect_err("an over-long identity is refused");
    assert_eq!(refused.refusal, SemanticHistoryRefusal::Identity);
}

#[test]
fn an_unstated_limit_is_refused_rather_than_guessed() {
    let store = populated();
    let mut port = granted(&store);
    let refused = port
        .lookup(&request("b1", 0), "corr-1")
        .expect_err("an unstated limit is refused");
    assert_eq!(refused.refusal, SemanticHistoryRefusal::PageTooLarge);
}

#[test]
fn a_request_that_is_not_the_served_shape_is_refused() {
    let oversized = vec![b' '; SEMANTIC_MAX_LOOKUP_REQUEST_BYTES + 1];
    let refused = SemanticLookupRequest::parse(&oversized).expect_err("over the byte bound");
    assert_eq!(
        refused.refusal,
        SemanticHistoryRefusal::LookupPayloadTooLarge
    );
    let refused = SemanticLookupRequest::parse(b"").expect_err("empty request");
    assert_eq!(
        refused.refusal,
        SemanticHistoryRefusal::LookupPayloadTooLarge
    );
    let refused = SemanticLookupRequest::parse(br#"{"fence":{}}"#).expect_err("missing fields");
    assert_eq!(refused.refusal, SemanticHistoryRefusal::LookupShape);
}

#[test]
fn an_unsupported_field_or_kind_name_is_refused_rather_than_ignored() {
    let body = serde_json::json!({
        "operation_id": "lookup-1",
        "fence": {"run_id": "run-1", "branch_id": "b1", "episode": 1, "epoch": 4},
        "binding": binding(),
        "query": {"limit": 8}
    });
    let mut with_extra = body.clone();
    with_extra["unexpected"] = serde_json::json!(true);
    let refused = SemanticLookupRequest::parse(&serde_json::to_vec(&with_extra).expect("json"))
        .expect_err("a field this shape does not declare is refused, not ignored");
    assert_eq!(refused.refusal, SemanticHistoryRefusal::LookupShape);

    let mut with_kind = body.clone();
    with_kind["query"] = serde_json::json!({"limit": 8, "kind": "teleportation"});
    let refused = SemanticLookupRequest::parse(&serde_json::to_vec(&with_kind).expect("json"))
        .expect_err("an unknown event kind is not a kind this vocabulary carries");
    assert_eq!(refused.refusal, SemanticHistoryRefusal::LookupShape);

    let mut without_limit = body;
    without_limit["query"] = serde_json::json!({});
    let parsed = SemanticLookupRequest::parse(&serde_json::to_vec(&without_limit).expect("json"))
        .expect("an unstated limit parses as zero");
    assert_eq!(parsed.query.limit, 0);
}

#[test]
fn a_lookup_reads_a_history_a_restart_wrote() {
    let name = unique("restart");
    let _ = std::fs::remove_file(&name);
    let mut store = SemanticHistoryStore::open(name.clone()).expect("opens empty");
    let mut b = batch(
        "b1",
        1,
        vec![card_played("e1", 1), damage("e2", 2, Some("e1"))],
    );
    b.window.intervals = Vec::new();
    store
        .append(&SemanticHistoryAppend {
            operation_id: "op-1".to_owned(),
            binding: binding(),
            batch: b,
        })
        .expect("history lands");
    let reopened = SemanticHistoryStore::open(name).expect("reopens what the restart wrote");
    assert_eq!(reopened.scope("b1"), Some(&scope("b1")));
    let mut port = granted(&reopened);
    let page = port
        .lookup(&request("b1", 8), "corr-1")
        .expect("served after restart");
    assert_eq!(page.matched, 2);
    assert_eq!(page.items[1].causal_parent.as_deref(), Some("e1"));
    let mut other = request("b1", 8);
    other.fence.branch_id = "b2".to_owned();
    let refused = port
        .lookup(&other, "corr-2")
        .expect_err("a branch never written");
    assert_eq!(refused.refusal, SemanticHistoryRefusal::UnknownBranch);
}

#[test]
fn a_pruned_span_is_served_as_retention_disclosed_never_as_empty_detail() {
    let mut store = populated();
    let prune = SemanticPruneRequest {
        branch_id: "b1".to_owned(),
        policy: SemanticRetentionPolicy {
            retain_observed: false,
            retain_latest: 1,
        },
    };
    let plan = store.prune_preview(&prune).expect("preview");
    store
        .prune("prune-1", &prune, &plan)
        .expect("prune applies");
    let mut port = granted(&store);
    let page = port
        .lookup(&request("b1", 8), "corr-1")
        .expect("page served");
    let pruned = page
        .items
        .iter()
        .find(|item| item.event_id == "e1")
        .expect("the pruned record is still identified");
    assert_eq!(pruned.sequence, 1);
    assert_eq!(pruned.kind, None);
    assert_eq!(pruned.detail, SemanticLookupDetail::RetentionDisclosed);
    assert_eq!(pruned.byte_len(), pruned.event_id.len());
    assert_ne!(pruned.detail, SemanticLookupDetail::Gap);
}

#[test]
fn a_lookup_leaves_the_store_unchanged() {
    let store = populated();
    let before = store.records("b1").expect("records").to_vec();
    let branch_count = store.branch_count();
    {
        let mut port = granted(&store);
        port.lookup(&request("b1", 8), "corr-1")
            .expect("page served");
    }
    let after = store.records("b1").expect("records").to_vec();
    assert_eq!(before, after);
    assert_eq!(store.branch_count(), branch_count);
}
