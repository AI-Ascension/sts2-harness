// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use sts2_harness::{CoopCoordinator, CoopError, CoopPeerRole, CoopSyncStatus};

#[test]
fn disagreement_disconnect_and_ally_target_suspend_local_mutation() {
    let mut coordinator = CoopCoordinator::new(7).expect("generation is valid");
    coordinator
        .register("local", CoopPeerRole::Local, 7)
        .expect("local registers");
    coordinator
        .register("ally", CoopPeerRole::Ally, 7)
        .expect("ally registers");
    assert_eq!(coordinator.status(), CoopSyncStatus::Synchronized);
    assert!(
        coordinator
            .authorize_local_action("local", 7, Some("ally"))
            .is_ok()
    );

    coordinator
        .report_generation("ally", 8)
        .expect("disagreement is recorded");
    assert_eq!(coordinator.status(), CoopSyncStatus::Disagreement);
    assert_eq!(
        coordinator.authorize_local_action("local", 7, None),
        Err(CoopError::MutationSuspended)
    );

    coordinator.reconnect("ally", 7).expect("peer catches up");
    coordinator
        .disconnect("ally")
        .expect("disconnect is recorded");
    assert_eq!(coordinator.status(), CoopSyncStatus::Disconnected);
    assert_eq!(
        coordinator.authorize_local_action("local", 7, None),
        Err(CoopError::MutationSuspended)
    );
}

#[test]
fn only_local_peer_and_known_ally_can_be_targeted() {
    let mut coordinator = CoopCoordinator::new(3).expect("generation is valid");
    coordinator
        .register("local", CoopPeerRole::Local, 3)
        .expect("local registers");
    coordinator
        .register("ally", CoopPeerRole::Ally, 3)
        .expect("ally registers");
    assert_eq!(
        coordinator.authorize_local_action("ally", 3, None),
        Err(CoopError::NotLocalPeer)
    );
    assert_eq!(
        coordinator.authorize_local_action("local", 3, Some("unknown")),
        Err(CoopError::InvalidAllyTarget)
    );
}
