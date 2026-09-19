// SPDX-License-Identifier: MIT

//! A failed episode should say what failed, not only that something did.

#![allow(clippy::expect_used)]

use sts2_harness::{EpisodeRunnerError, PortError};

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
