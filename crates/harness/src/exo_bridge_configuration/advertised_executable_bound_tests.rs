// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

//! `sts2-harness#785`: the advertisement path's size bound is not the production package bound.
//!
//! One constant was answering two unrelated questions — how large a *shipped* bridge package may
//! be before it is refused, and how large the *running test binary* may be before a purely
//! in-memory advertisement test starts failing. The harness debug test binary is already ~182 MiB,
//! so an unrelated build-size increase surfaced as `exo_bridge_package_bound` from tests whose
//! subject is wire-format invariants: a packaging error reported by code that packaged nothing.
//!
//! These tests cover the half of the defect that is observable at runtime. The two bounds are the
//! same number on purpose today, so no runtime assertion can tell the constants apart; what these
//! fix is the *refusal token*, which is what an operator or a failing CI job actually reads.

use super::read_bounded;
use crate::exo_bridge_configuration::{MAX_ADVERTISED_EXECUTABLE_BYTES, MAX_EXECUTOR_BYTES};

/// A scratch file of exactly `bytes` long, removed by the guard on drop.
struct Scratch {
    path: std::path::PathBuf,
}

impl Scratch {
    fn named(tag: &str, bytes: usize) -> Self {
        let directory =
            std::env::temp_dir().join(format!("sts2-harness-785-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("the temporary directory is creatable");
        let path = directory.join("executable");
        std::fs::write(&path, vec![0_u8; bytes]).expect("the scratch file is writable");
        Self { path }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        if let Some(directory) = self.path.parent() {
            let _ = std::fs::remove_dir(directory);
        }
    }
}

/// An oversized advertised executable is refused by name, not as a packaging failure.
///
/// This is the legible-cause half of the defect. Before the split, an out-of-size test binary
/// surfaced as `exo_bridge_package_bound` — the same token the production package loader returns
/// for a bridge that was never shipped — so the failure named a neighbouring subject.
///
/// The maximum is passed explicitly and small: `read_bounded` takes its bound as an argument, so
/// the token can be observed exactly without writing half a gigabyte on every test run.
#[test]
fn an_oversized_advertised_executable_is_refused_by_its_own_token() {
    let scratch = Scratch::named("oversized", 17);

    let refusal = read_bounded(&scratch.path, 16);

    assert_eq!(
        refusal.err(),
        Some("exo_bridge_advertised_executable_bound"),
        "an oversized advertised executable must be refused as the advertisement bound, not as \
         the production package bound"
    );
}

/// A file of exactly the bound is still accepted.
///
/// The refusal above would also pass if the reader refused everything, which is a worse defect
/// than the one being fixed: advertisement would fail on every build. Pinning the inclusive
/// boundary is what makes the refusal mean "too large" rather than "refused".
#[test]
fn a_file_at_exactly_the_bound_is_accepted() {
    let scratch = Scratch::named("atlimit", 16);

    let accepted = read_bounded(&scratch.path, 16);

    assert!(
        accepted.is_ok(),
        "a file of exactly the advertised bound must be accepted, got {:?}",
        accepted.err()
    );
    assert_eq!(
        accepted.ok().map(|bytes| bytes.len()),
        Some(16),
        "the accepted read is the whole file, not a truncated prefix"
    );
}

/// The production package ceiling did not move, and both bounds are real limits.
///
/// The two constants are intentionally equal today, so asserting they differ would be asserting an
/// accident. What must hold is that the production boundary is still the value it had before the
/// split — #785 asks for a decision about which side owns the bound, not for a larger limit — and
/// that the advertisement path is bounded at all.
#[test]
fn the_production_package_ceiling_did_not_move() {
    assert_eq!(
        MAX_EXECUTOR_BYTES,
        512 * 1024 * 1024,
        "the shipped-artifact boundary is unchanged"
    );
    assert_eq!(
        MAX_ADVERTISED_EXECUTABLE_BYTES,
        512 * 1024 * 1024,
        "the advertisement bound is declared independently and is itself a real limit"
    );
}
