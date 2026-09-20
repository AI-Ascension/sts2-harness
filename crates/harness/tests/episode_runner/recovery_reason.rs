// SPDX-License-Identifier: MIT

use super::*;

/// A recovery observation as the runtime-v3 boundary builds it: non-actionable, with the host's own
/// reason token bound when one is present.
fn recovery_state(code: Option<&str>, generation: u64) -> State {
    let state_id = format!("recovery-{generation}");
    let mut observation = EpisodeObservation::new(
        state_id.clone(),
        generation,
        EpisodeStage::Recovery,
        false,
        true,
        false,
        projection(&state_id, generation, EpisodeStage::Recovery),
    )
    .expect("recovery projection is valid");
    if let Some(code) = code {
        observation = observation
            .with_recovery_code(code)
            .expect("a recovery code is an identity token");
    }
    State {
        observation,
        actions: EpisodeLegalActionSet::new(state_id, generation, vec![])
            .expect("an unactionable recovery offers no legal actions"),
    }
}

/// A refused launch contract, an unconfigured host and an unavailable observation all reach the
/// runner as `EpisodeStage::Recovery`. The stage alone cannot tell them apart, so the host's reason
/// token has to survive into the episode failure; otherwise the whole distinction is lost and an
/// automated run reads a listener that answered as progress.
#[test]
fn recovery_failure_names_the_host_reason_code() {
    let refused = recovery_state(
        Some("launch_contract_refused_isolated_user_dir_mismatch"),
        0,
    );
    let mut runtime = FakeRuntime::new(vec![refused]);
    let mut model = FakeModel::default();
    let error = runner()
        .run(&mut runtime, &mut model)
        .expect_err("recovery never clears");
    assert_eq!(
        error,
        EpisodeRunnerError::RecoveryRequired {
            code: Some(String::from(
                "launch_contract_refused_isolated_user_dir_mismatch"
            )),
        }
    );
    assert_eq!(
        error.to_string(),
        "episode requires recovery before policy can continue: \
         host recovery code launch_contract_refused_isolated_user_dir_mismatch"
    );
    assert_eq!(model.calls, 0);
    assert_eq!(runtime.dispatches, 0);
    assert!(runtime.released && runtime.mcp_closed && runtime.gateway_closed);
}

/// The reason is reported only when the host supplied one; the message must not invent a condition.
#[test]
fn recovery_failure_without_a_host_code_stays_generic() {
    let mut runtime = FakeRuntime::new(vec![recovery_state(None, 0)]);
    let mut model = FakeModel::default();
    let error = runner()
        .run(&mut runtime, &mut model)
        .expect_err("recovery never clears");
    assert_eq!(error, EpisodeRunnerError::RecoveryRequired { code: None });
    assert_eq!(
        error.to_string(),
        "episode requires recovery before policy can continue"
    );
}

/// A reason token is part of the recovery contract, not a free-text field, and only a recovery
/// observation may carry one. Both refusals keep a normal observation from acquiring a reason that
/// would misdescribe its stage.
#[test]
fn only_a_well_formed_recovery_observation_carries_a_code() {
    for stage in [
        EpisodeStage::Setup,
        EpisodeStage::Combat,
        EpisodeStage::Victory,
    ] {
        assert_eq!(
            state(stage, 0)
                .observation
                .with_recovery_code("host_not_configured")
                .err(),
            Some(ObservationError::UnexpectedRecoveryCode),
            "{stage:?} is not a recovery observation"
        );
    }

    let long = "x".repeat(513);
    for code in ["", "not a token", "line\nbreak", long.as_str()] {
        assert_eq!(
            recovery_state(None, 0)
                .observation
                .with_recovery_code(code)
                .err(),
            Some(ObservationError::InvalidRecoveryCode),
            "{code:?} must not be accepted as a reason token"
        );
    }
}

/// A recovery code is the same wire identity the observation schema requires of `state_id`, so one
/// predicate governs both.
///
/// The two are pinned against each other at the boundaries rather than each against its own copy of
/// the rule. A rule that drifted wider would let a value the host's own schema rejects be named in a
/// failure; a rule that drifted narrower would refuse a code the host legitimately composes and put
/// the operator back in front of an anonymous recovery.
#[test]
fn the_recovery_code_is_held_to_the_state_identity_rule() {
    let identity_512 = "x".repeat(512);
    let identity_513 = "x".repeat(513);
    for value in [
        "",
        "host_not_configured",
        "launch_contract_refused",
        "launch_contract_refused_isolated_user_dir_mismatch",
        "a.b:c/d-e_f",
        "with space",
        "line\nbreak",
        "%",
        "caf\u{e9}",
        identity_512.as_str(),
        identity_513.as_str(),
    ] {
        let as_state_identity = EpisodeObservation::new(
            value,
            0,
            EpisodeStage::Setup,
            false,
            true,
            false,
            projection(value, 0, EpisodeStage::Setup),
        )
        .is_ok();
        let as_recovery_code = recovery_state(None, 0)
            .observation
            .with_recovery_code(value)
            .is_ok();
        assert_eq!(
            as_state_identity, as_recovery_code,
            "{value:?} must be judged exactly as a state identity is"
        );
    }
}

/// The host composes a refusal code as the prefix, `_`, and a reason of at most 64 ASCII
/// alphanumerics, `_` or `-`, and answers the two older conditions unchanged.
///
/// Every code that composer can produce therefore sits well inside the identity rule and must be
/// admitted. The rule is deliberately not narrowed to the composer's own 64-byte reason bound: the
/// observation schema admits a 512-byte identity, and refusing a code the schema carries would trade
/// a silent failure for an unparseable one.
#[test]
fn every_code_the_host_composes_is_admitted() {
    let longest_reason = format!("{}-_{}", "a".repeat(30), "b".repeat(32));
    assert_eq!(longest_reason.len(), 64);
    for reason in [
        "isolated_user_dir_mismatch",
        "campaign_required",
        longest_reason.as_str(),
    ] {
        for code in [
            String::from("launch_contract_refused"),
            format!("launch_contract_refused_{reason}"),
            String::from("host_not_configured"),
            String::from("host_observation_unavailable"),
        ] {
            assert!(
                recovery_state(None, 0)
                    .observation
                    .with_recovery_code(code.as_str())
                    .is_ok(),
                "{code} is a code the host composes and must be admitted"
            );
        }
    }
}
