// SPDX-License-Identifier: MIT

//! Shared construction helpers for the semantic history suites.

#![allow(dead_code)]

use sts2_harness::semantic_history::{
    SemanticHistoryAppend, SemanticHistoryBinding, SemanticHistoryCaptureWindow,
    SemanticHistoryCausalParent, SemanticHistoryCoverage, SemanticHistoryCoverageInterval,
    SemanticHistoryCoverageStatus, SemanticHistoryEventInput, SemanticHistoryKind,
    SemanticHistoryNamespace, SemanticHistoryOrigin, SemanticHistoryStore, SemanticHistorySubject,
    SemanticHistoryValue,
};

pub const ROOT: &str = "branch_root";

pub fn binding() -> SemanticHistoryBinding {
    SemanticHistoryBinding {
        project_id: "project_alpha".to_owned(),
        run_id: "run_0001".to_owned(),
        agent_id: "agent_0001".to_owned(),
        game_profile: "sts2_dev".to_owned(),
        content_manifest_id: "manifest_0001".to_owned(),
        locale: "en-US".to_owned(),
        authority_epoch: 1,
    }
}

/// The same owner scope as [`binding`], under a different run.
pub fn binding_other_run() -> SemanticHistoryBinding {
    SemanticHistoryBinding {
        run_id: "run_0002".to_owned(),
        ..binding()
    }
}

/// The same owner scope as [`binding`], advanced past its authority epoch.
pub fn binding_other_epoch() -> SemanticHistoryBinding {
    SemanticHistoryBinding {
        authority_epoch: 2,
        ..binding()
    }
}

/// The same owner scope as [`binding`], with a field that is not an opaque identity.
pub fn binding_non_opaque() -> SemanticHistoryBinding {
    SemanticHistoryBinding {
        run_id: "/etc/passwd".to_owned(),
        ..binding()
    }
}

pub fn window(start: u64) -> SemanticHistoryCaptureWindow {
    SemanticHistoryCaptureWindow::complete(start)
}

pub fn store() -> SemanticHistoryStore {
    SemanticHistoryStore::open(binding(), ROOT, window(1)).expect("store opens")
}

pub fn subject(identity: &str) -> SemanticHistorySubject {
    SemanticHistorySubject {
        namespace: SemanticHistoryNamespace::LiveInstance,
        identity: identity.to_owned(),
    }
}

pub fn quantity(amount: i64, unit: &str) -> SemanticHistoryValue {
    SemanticHistoryValue::Quantity {
        amount,
        unit: unit.to_owned(),
    }
}

pub fn event(
    event_id: &str,
    kind: SemanticHistoryKind,
    sequence: u64,
    value: Option<SemanticHistoryValue>,
) -> SemanticHistoryEventInput {
    SemanticHistoryEventInput {
        event_id: event_id.to_owned(),
        kind,
        sequence,
        episode_id: "episode_0001".to_owned(),
        authority_epoch: 1,
        origin: SemanticHistoryOrigin::Native,
        subject: Some(subject("instance_hero")),
        value,
        coverage: SemanticHistoryCoverage::captured(),
    }
}

/// A stated causal parent.
pub fn stated_parent(parent: &str) -> SemanticHistoryCausalParent {
    SemanticHistoryCausalParent::Stated {
        event_id: parent.to_owned(),
    }
}

/// A window that declares one dropped span.
pub fn window_with_gap(start: u64, from: u64, to: u64) -> SemanticHistoryCaptureWindow {
    SemanticHistoryCaptureWindow {
        capture_start: start,
        history_before_capture: Some(4),
        intervals: vec![SemanticHistoryCoverageInterval {
            from_sequence: from,
            to_sequence: to,
            status: SemanticHistoryCoverageStatus::Dropped,
            label: "capture_dropped".to_owned(),
        }],
    }
}

/// A gap event whose coverage matches a declared dropped span.
pub fn gap_event(event_id: &str, sequence: u64) -> SemanticHistoryEventInput {
    SemanticHistoryEventInput {
        coverage: SemanticHistoryCoverage::gap(
            SemanticHistoryCoverageStatus::Dropped,
            "capture_dropped",
        ),
        ..event(event_id, SemanticHistoryKind::CardPlayed, sequence, None)
    }
}

/// Appends one event with no value and no stated parent.
pub fn append_plain(
    store: &mut SemanticHistoryStore,
    branch: &str,
    event_id: &str,
    kind: SemanticHistoryKind,
    sequence: u64,
) -> SemanticHistoryAppend {
    store
        .append(
            branch,
            event(event_id, kind, sequence, None),
            SemanticHistoryCausalParent::NotStated,
        )
        .expect("a plain event is admitted")
}

/// Appends one event that states a causal parent.
pub fn append_linked(
    store: &mut SemanticHistoryStore,
    branch: &str,
    event_id: &str,
    kind: SemanticHistoryKind,
    sequence: u64,
    parent: &str,
) -> SemanticHistoryAppend {
    store
        .append(
            branch,
            event(event_id, kind, sequence, None),
            stated_parent(parent),
        )
        .expect("a linked event is admitted")
}

/// Appends one quantity-changing event with a stated amount.
pub fn append_quantity(
    store: &mut SemanticHistoryStore,
    branch: &str,
    event_id: &str,
    kind: SemanticHistoryKind,
    sequence: u64,
    amount: i64,
) -> SemanticHistoryAppend {
    store
        .append(
            branch,
            event(event_id, kind, sequence, Some(quantity(amount, "hp"))),
            SemanticHistoryCausalParent::NotStated,
        )
        .expect("a stated quantity is admitted")
}

/// A store on the root branch holding `count` plain events, with sequences `1..=count`.
pub fn store_with_events(count: u64) -> SemanticHistoryStore {
    let mut store = store();
    for sequence in 1..=count {
        append_plain(
            &mut store,
            ROOT,
            &format!("event_{sequence}"),
            SemanticHistoryKind::CardPlayed,
            sequence,
        );
    }
    store
}
