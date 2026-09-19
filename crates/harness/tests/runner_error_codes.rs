// SPDX-License-Identifier: MIT

//! A failed episode should say what failed, not only that something did.

#![allow(clippy::expect_used)]

use sts2_harness::{
    EpisodeRunnerConfig, EpisodeRunnerError, PortError, RecoveryController, StabilityBarrier,
};

#[test]
fn a_port_failure_names_its_code() {
    // "episode observation failed" alone was the whole of what a failed run reported, and the
    // reason had to be guessed at. The code is a &'static str chosen at compile time, so naming it
    // adds no observation, identifier or provider text to the message.
    let error = EpisodeRunnerError::Observe(PortError::new("observe_failed", "detail here", false));
    let rendered = error.to_string();
    assert_eq!(rendered, "episode observation failed: observe_failed");
    // The free-form message is host text and stays out of it.
    assert!(!rendered.contains("detail here"));
}

#[test]
fn every_port_carrying_variant_names_its_code() {
    for (error, expected) in [
        (
            EpisodeRunnerError::Launch(PortError::new("launch_failed", "m", false)),
            "episode launch failed: launch_failed",
        ),
        (
            EpisodeRunnerError::LegalActions(PortError::new("catalog_failed", "m", false)),
            "episode legal-action request failed: catalog_failed",
        ),
        (
            EpisodeRunnerError::Dispatch(PortError::new("dispatch_failed", "m", false)),
            "episode action dispatch failed: dispatch_failed",
        ),
        (
            EpisodeRunnerError::GameInformationBinding(PortError::new(
                "binding_failed",
                "m",
                false,
            )),
            "episode game-information binding preparation failed: binding_failed",
        ),
    ] {
        assert_eq!(error.to_string(), expected);
    }
}

#[test]
fn a_repeated_situation_says_what_it_was() {
    assert_eq!(
        EpisodeRunnerError::RepeatedSituation.to_string(),
        "episode reached one situation repeatedly without progressing"
    );
}

fn config() -> EpisodeRunnerConfig {
    EpisodeRunnerConfig::new(
        16,
        StabilityBarrier::new(1, 1).expect("barrier"),
        RecoveryController::new(1).expect("recovery"),
        "an objective",
        Vec::new(),
    )
    .expect("config")
}

#[test]
fn both_bounds_are_off_unless_a_caller_asks_for_them() {
    // The library keeps the behaviour it had. Only the runtime opts in, so no existing caller
    // changes and no existing test starts settling or abandoning episodes.
    let config = config();
    assert_eq!(config.max_consecutive_abstentions(), 0);
    assert_eq!(config.max_repeated_situations(), 0);
}

#[test]
fn a_caller_can_bound_repeated_situations() {
    assert_eq!(
        config()
            .with_max_repeated_situations(8)
            .max_repeated_situations(),
        8
    );
    // The two bounds are independent: one counts abstentions on an unchanged state, the other
    // counts visits to a situation whatever was decided there.
    let both = config()
        .with_max_repeated_situations(8)
        .with_max_consecutive_abstentions(3);
    assert_eq!(both.max_repeated_situations(), 8);
    assert_eq!(both.max_consecutive_abstentions(), 3);
}
