// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use sts2_harness::{DecisionFence, FenceError, MAX_OUTSTANDING_DECISIONS};

#[test]
fn a_decision_settles_exactly_once() {
    let mut fence = DecisionFence::new(1);
    let token = fence.issue(1).expect("token issues");
    assert_eq!(token.execution_epoch, 1);
    assert_eq!(fence.outstanding(), 1);
    fence.settle(token).expect("first settlement");
    assert_eq!(fence.outstanding(), 0);
    assert_eq!(
        fence.settle(token).expect_err("duplicate settlement"),
        FenceError::UnknownDecision
    );
}

#[test]
fn a_new_epoch_invalidates_every_outstanding_decision() {
    let mut fence = DecisionFence::new(1);
    let first = fence.issue(1).expect("first");
    let second = fence.issue(1).expect("second");
    let settled = fence.issue(1).expect("settled");
    fence.settle(settled).expect("settle before restore");

    assert_eq!(fence.advance_epoch(2).expect("epoch advances"), 2);
    assert_eq!(fence.outstanding(), 0);
    assert_eq!(fence.invalidated(), 2);
    assert_eq!(
        fence.settle(first).expect_err("stale decision"),
        FenceError::StaleDecision
    );
    assert_eq!(
        fence.settle(second).expect_err("stale decision"),
        FenceError::StaleDecision
    );
    assert_eq!(
        fence.issue(1).expect_err("stale epoch issue"),
        FenceError::StaleEpoch
    );
    let current = fence.issue(2).expect("current epoch issues");
    fence.settle(current).expect("current epoch settles");
}

#[test]
fn epochs_must_advance() {
    let mut fence = DecisionFence::new(3);
    assert_eq!(
        fence.advance_epoch(3).expect_err("equal epoch"),
        FenceError::NonAdvancingEpoch
    );
    assert_eq!(
        fence.advance_epoch(2).expect_err("lower epoch"),
        FenceError::NonAdvancingEpoch
    );
    assert_eq!(fence.current_epoch(), 3);
    assert_eq!(fence.invalidated(), 0);
}

#[test]
fn outstanding_decisions_are_bounded() {
    let mut fence = DecisionFence::new(1);
    for _ in 0..MAX_OUTSTANDING_DECISIONS {
        fence.issue(1).expect("within capacity");
    }
    assert_eq!(fence.outstanding(), MAX_OUTSTANDING_DECISIONS);
    assert_eq!(
        fence.issue(1).expect_err("capacity reached"),
        FenceError::Capacity
    );
}

#[test]
fn settling_an_unissued_token_is_refused() {
    let mut fence = DecisionFence::new(1);
    let foreign = sts2_harness::DecisionToken {
        execution_epoch: 1,
        sequence: 999,
    };
    assert_eq!(
        fence.settle(foreign).expect_err("unknown token"),
        FenceError::UnknownDecision
    );
}
