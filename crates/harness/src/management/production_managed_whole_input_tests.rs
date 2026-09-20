// SPDX-License-Identifier: MIT

use super::*;

/// Selected limits that publish a separate output reserve, so the served decision admits the
/// assembled provider bytes against the combined whole-input bound.
fn reserved_limits(
    max_items: usize,
    max_context_bytes: usize,
    output_reserve_bytes: usize,
) -> ContextRenderLimits {
    ContextRenderLimits {
        max_items,
        max_context_bytes,
        output_reserve_bytes: Some(output_reserve_bytes),
        ..ContextRenderLimits::harness_maxima()
    }
}

/// The exact assembled provider bytes the served path will admit.
///
/// Measured independently of the session under test: the same source, boundary and execution
/// identity are rendered through the renderer's own entry point, so the assertions below compare
/// the served admission against bytes this test derived rather than against a value the session
/// reported about itself.
fn assembled_provider_bytes(source: &ContextRenderSource, config: &ExoConfig) -> usize {
    let input = input();
    let managed = crate::context_control::ManagedRenderInput {
        execution_id: input.execution_id.to_string(),
        state_id: input.observation.state_id().to_owned(),
        generation: input.observation.generation(),
        observation: input.observation.fair_play().as_value().clone(),
        legal_action_ids: input
            .legal_actions
            .actions()
            .iter()
            .map(|action| action.action_id().to_owned())
            .collect(),
        objective: input.objective.clone(),
        hard_constraints: input.hard_constraints.clone(),
        map_context: None,
    };
    crate::context_control::ContextRenderer::enabled_at_with_limits(
        &source.boundary,
        managed,
        &source.document.draft,
        &source.document.items,
        config,
        source.now,
        &ContextRenderLimits::harness_maxima(),
    )
    .expect("selected source prepares")
    .provider_bytes()
    .len()
}

#[test]
fn served_managed_render_admits_the_exact_whole_input_bound_and_refuses_one_byte_over() {
    let (source, config) = render_fixture();
    let assembled = assembled_provider_bytes(&source, &config);
    let reserve = 64usize;

    // Exact boundary: the assembled input plus its reserve is exactly the selected whole-input
    // bound, so the decision still dispatches once.
    let (mut session, _, exchanges, _) = render_test_session(
        source.clone(),
        config.clone(),
        reserved_limits(2, assembled + reserve, reserve),
        Default::default(),
        None,
    );
    let decision = session
        .decide_for(&input(), "decision.live.v1", "context.live.v1")
        .expect("the exact combined bound is admitted");
    assert!(matches!(decision, Decision::Action { .. }));
    assert_eq!(exchanges.load(Ordering::SeqCst), 1);

    // One byte over: the same input, the same reserve, one byte less of bound. The assembled bytes
    // cannot shrink, so this is a refusal and it happens before any provider exchange.
    let (mut session, _, exchanges, _) = render_test_session(
        source,
        config,
        reserved_limits(2, assembled + reserve - 1, reserve),
        Default::default(),
        None,
    );
    let error = session
        .decide_for(&input(), "decision.live.v1", "context.live.v1")
        .expect_err("one byte over the combined bound must refuse");
    assert_eq!(error.code, "context_whole_input_budget_exceeded");
    assert!(
        error.message.contains(&format!(
            "input {assembled} plus output reserve {reserve} exceeds the combined window {}",
            assembled + reserve - 1
        )),
        "the refusal names the served bound and its bytes: {}",
        error.message
    );
    assert_eq!(exchanges.load(Ordering::SeqCst), 0);
}

#[test]
fn served_managed_render_refuses_an_unusable_advertised_reserve_before_provider_exchange() {
    // A published reserve of zero is never unlimited or silently ignored; the admission fails
    // closed with its own code so a misconfigured owner limit is distinguishable from an
    // over-bound input.
    let (source, config) = render_fixture();
    let assembled = assembled_provider_bytes(&source, &config);
    let (mut session, _, exchanges, _) = render_test_session(
        source,
        config,
        reserved_limits(2, assembled + 1024, 0),
        Default::default(),
        None,
    );

    let error = session
        .decide_for(&input(), "decision.live.v1", "context.live.v1")
        .expect_err("a zero reserve is not an admissible bound");
    assert_eq!(error.code, "context_whole_input_budget_invalid");
    assert!(error.message.contains("output_reserve_bytes"));
    assert_eq!(exchanges.load(Ordering::SeqCst), 0);
}
