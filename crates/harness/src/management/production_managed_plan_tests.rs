// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn served_managed_render_does_not_reuse_a_plan_without_sending_new_prepared_bytes() {
    let (source, config) = render_fixture();
    let mut next_source = source.clone();
    next_source.boundary.generation = 2;
    next_source.identity.boundary = next_source.boundary.clone();
    next_source.source_id = "strategy-v2".to_owned();
    next_source.source_version = 2;
    next_source.active_revision_id = "revision-2".to_owned();
    next_source.identity.source_id = next_source.source_id.clone();
    next_source.identity.source_version = next_source.source_version;
    next_source.identity.active_revision_id = next_source.active_revision_id.clone();
    next_source.document.draft.base_revision_id = next_source.active_revision_id.clone();
    let item = next_source
        .document
        .items
        .get_mut("strategy-1:1")
        .expect("first source item");
    item.bytes = b"trusted retained strategy v2".to_vec();
    item.reference.sha256 = crate::sha256_hex(&item.bytes);
    next_source.document.draft.selected_items[0] = item.reference.clone();
    next_source.source_digest =
        crate::context_control::context_source_digest(&next_source.document).expect("source hash");
    next_source.identity.source_digest = next_source.source_digest.clone();
    let responses = Arc::new(Mutex::new(VecDeque::from([
        br#"{"decision":"plan","action_ids":["combat.play-card","combat.end-turn"],"rationale":"bounded managed plan"}"#.to_vec(),
        br#"{"decision":"action","action_id":"combat.end-turn","rationale":"fresh managed request"}"#.to_vec(),
    ])));
    let exchanges = Arc::new(AtomicUsize::new(0));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let transport = PreparedRecordingTransport {
        exchanges: Arc::clone(&exchanges),
        requests: Arc::clone(&requests),
        on_exchange: None,
        responses: Some(responses),
    };
    let provider = ExoDecisionSource::new(ExoSession::new(ExoProvider::new(transport, config)));
    let mut provider = provider;
    let first_input = managed_plan_input(1);
    let first_prepared = provider
        .prepare_managed_context(&first_input, &source)
        .expect("first managed input");
    let first = provider
        .decide_prepared_for(
            &first_input,
            "decision.live.v1",
            "context.live.v1",
            &first_prepared,
        )
        .expect("first managed plan");
    assert!(matches!(
        first,
        Decision::Action { ref action_id, .. } if action_id == "combat.play-card"
    ));
    provider.action_completed(true);

    let second_input = managed_plan_input(2);
    let second_prepared = provider
        .prepare_managed_context(&second_input, &next_source)
        .expect("second managed input");
    let second_request: serde_json::Value =
        serde_json::from_slice(second_prepared.provider_bytes()).expect("second prepared request");
    assert_eq!(
        second_request["management_context"]["selected_items"][0]["content"],
        "trusted retained strategy v2"
    );
    let second = provider
        .decide_prepared_for(
            &second_input,
            "decision.live.v1",
            "context.live.v1",
            &second_prepared,
        )
        .expect("second managed decision");

    assert!(matches!(
        second,
        Decision::Action { ref action_id, .. } if action_id == "combat.end-turn"
    ));
    assert_eq!(exchanges.load(Ordering::SeqCst), 2);
    assert_eq!(
        requests.lock().expect("recorded requests").as_slice(),
        &[
            first_prepared.provider_bytes().to_vec(),
            second_prepared.provider_bytes().to_vec(),
        ]
    );
}
