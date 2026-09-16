// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn committed_revision_receipt_recovers_its_resulting_revision() {
    let (owner, actor, request, runtime_binding, digest, selected) = setup();
    owner
        .record_observation(
            &actor,
            &request,
            &digest,
            &runtime_binding,
            &observation(1),
            &selected,
        )
        .expect("observation");
    let actions = EpisodeLegalActionSet::new(
        "combat-1",
        1,
        vec![EpisodeLegalAction::new("combat.end-turn", ActionKind::EndTurn).expect("action")],
    )
    .expect("actions");
    owner
        .record_legal_actions(&actor, &request, &digest, &runtime_binding, &actions)
        .expect("catalog");
    let catalog = owner.catalog(&actor).expect("catalog");
    let descriptor = &catalog.descriptors[0];
    let binding_request = ContextBindingRequest {
        workflow_run_id: runtime_binding.run_id.clone(),
        definition_digest: digest.clone(),
        instance_id: request.instance_id.clone(),
        graph_id: "graph-1".into(),
        node_id: "node-1".into(),
        node_execution_id: "execution-1".into(),
        node_kind: "decide".into(),
        context_ref: descriptor.context_ref.clone(),
        binding_id: descriptor.binding_id.clone(),
        binding_version: descriptor.version,
        binding_digest: descriptor.digest.clone(),
    };
    let initial_binding = owner.bind(&actor, &binding_request).expect("binding");
    let pause = ContextControlCommand::Pause {
        idempotency_key: "commit-receipt-pause".into(),
        expected_control_version: initial_binding.boundary.control_version,
    };
    owner
        .control(&actor, &initial_binding, &pause)
        .expect("pause");
    let paused_binding = owner
        .bind(&actor, &binding_request)
        .expect("paused binding");
    let manifest = "e".repeat(64);
    let commit = ContextControlCommand::Commit {
        idempotency_key: "commit-receipt-commit".into(),
        expected_control_version: paused_binding.boundary.control_version,
        expected_revision_id: paused_binding.approved_revision_id.clone(),
        expected_boundary: paused_binding.boundary.clone(),
        preview_manifest_digest: manifest.clone(),
        approved_manifest_digest: manifest,
    };
    let receipt = owner
        .control(&actor, &paused_binding, &commit)
        .expect("commit accepted and durably recorded");
    assert_eq!(receipt.effect, "revision_committed");
    assert_eq!(receipt.revision_id.as_deref(), Some("revision-2"));
    receipt
        .validate_for(&paused_binding, &commit)
        .expect("exact commit receipt");

    let snapshot = admitted_snapshot(&request, &digest, &binding_request);
    let restarted = Owner {
        configuration: owner.configuration.clone(),
        key: owner.key,
        current: Mutex::new(BTreeMap::new()),
    };
    let recovered = restarted
        .recover_control_receipt(&actor, &snapshot, &commit)
        .expect("historical lookup")
        .expect("recorded commit receipt");
    assert_eq!(recovered.receipt, receipt);
    assert_eq!(recovered.binding, paused_binding);
    assert!(restarted.current.lock().expect("current lock").is_empty());
}
