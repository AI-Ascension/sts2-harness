// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

//! `sts2-harness#782`: the advertised `bridge_sha256` is a property of the running executable.
//!
//! Every advertised profile embeds this digest, and each lookup profile description is derived
//! from the one-shot one. Hashing the executable once per derivation re-read the whole file each
//! time, and under `cargo test` the executable *is* the test binary — a debug test binary of this
//! crate is ~190 MB, so each derivation costs a full read plus SHA-256 of that file. That made
//! `lookup_advertisement_does_not_borrow_the_one_shot_decision_set`, a purely in-memory
//! advertisement test, pay for four of them.
//!
//! #782 reported a hang and set no bound. These tests assert nothing about wall-clock time: the
//! cost of a read scales with the size of whatever binary is running and with machine load, so
//! timing it would be flaky. The deterministic proof that the digest is computed once lives in
//! `exo_bridge_configuration/digest_cache_tests.rs`; what is asserted here is the property that
//! makes caching safe — the value is still the real digest of the running executable.
//!
//! It lives in its own file so `exo_advertised_variant_negatives.rs` stays under its preferred
//! size budget rather than growing a policy waiver.

use serde_json::json;
use sts2_harness::exo_bridge_configuration as config;

fn loaded() -> config::Loaded {
    config::Loaded {
        config: config::Configuration {
            schema: "sts2.exo-lookup-config-v1".to_owned(),
            executor: "/executor".into(),
            executor_sha256: "0".repeat(64),
            source_root: "/source".into(),
            extension: "/extension".into(),
            extension_sha256: "0".repeat(64),
            node: "/node".into(),
            node_sha256: "0".repeat(64),
            model: "o3-pro".to_owned(),
            endpoint: "http://127.0.0.1:8080".to_owned(),
        },
        digest: "0".repeat(64),
    }
}

/// The memoised digest is still the real digest of this executable.
///
/// This is the assertion that makes caching honest. The expected value is computed here by
/// independently reading the file, so a cache that returned a constant, a value from a previous
/// build, or the digest of some other file would fail. The defect was the cost of the derivation,
/// never the value — nothing about the digest was ever wrong, only how many times it was
/// recomputed — so this test is what holds that line.
#[test]
fn every_advertised_profile_reports_the_running_executable_digest() {
    let loaded = loaded();
    let executable = std::env::current_exe().expect("the running test executable");
    let bytes = std::fs::read(&executable).expect("the test executable is readable");
    let expected = sts2_harness::sha256_hex(&bytes);

    let one_shot = loaded.description().expect("one-shot description");
    let lookup = loaded.lookup_description().expect("lookup description");
    let bootstrap = loaded
        .lookup_bootstrap_description()
        .expect("bootstrap description");
    let history = loaded
        .lookup_history_description()
        .expect("history description");

    for (profile, description) in [
        ("one-shot", &one_shot),
        ("lookup", &lookup),
        ("lookup-bootstrap", &bootstrap),
        ("lookup-history", &history),
    ] {
        assert_eq!(
            description["bridge_sha256"],
            json!(expected),
            "{profile} advertised a bridge digest that is not this executable's"
        );
    }
}

/// Repeated description calls are stable, so the cache cannot serve a value that drifts.
///
/// Every profile above is derived from a shared body, so a per-process cache is what keeps this
/// consistent across calls rather than an accident of each call computing the same thing twice.
#[test]
fn repeated_description_calls_agree() {
    let loaded = loaded();
    let first = loaded.description().expect("first description");
    let second = loaded.description().expect("second description");
    assert_eq!(
        first, second,
        "two descriptions of the same Loaded must be identical"
    );

    let lookup = loaded.lookup_description().expect("lookup description");
    let lookup_again = loaded
        .lookup_description()
        .expect("second lookup description");
    assert_eq!(lookup, lookup_again);
}
