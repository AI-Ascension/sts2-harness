// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use sts2_harness::context_memory::*;

fn scope() -> MemoryScope {
    MemoryScope::new(
        "project-fixture",
        "run-fixture",
        "episode-fixture",
        "agent-fixture",
    )
}

fn candidate(scope: MemoryScope) -> MapMemoryCandidate {
    MapMemoryCandidate {
        scope,
        node_id: "room-1".to_owned(),
        generation: 7,
        visible: true,
        actor_id: Some("actor-public".to_owned()),
        legal_id: Some("legal-current".to_owned()),
        private_actor: false,
        current_authority: true,
    }
}

#[test]
fn current_visible_candidate_requires_current_legal_authority() {
    let gate = MapMemoryGate::new(scope(), 7, ["legal-current".to_owned()]).expect("gate");
    let accepted = gate.accept(candidate(scope())).expect("accepted");
    assert_eq!(accepted.node_id, "room-1");
}

#[test]
fn private_actor_historical_and_future_candidates_are_denied() {
    let gate = MapMemoryGate::new(scope(), 7, ["legal-current".to_owned()]).expect("gate");

    let mut private = candidate(scope());
    private.private_actor = true;
    assert_eq!(gate.accept(private), Err(MemoryError::PermissionDenied));

    let mut historical = candidate(scope());
    historical.legal_id = Some("legal-old".to_owned());
    assert_eq!(gate.accept(historical), Err(MemoryError::PermissionDenied));

    let mut future = candidate(scope());
    future.generation = 8;
    assert_eq!(gate.accept(future), Err(MemoryError::InvalidEntry));

    let mut hidden = candidate(scope());
    hidden.visible = false;
    assert_eq!(gate.accept(hidden), Err(MemoryError::PermissionDenied));
}
