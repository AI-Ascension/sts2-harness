// SPDX-License-Identifier: MIT

#![allow(clippy::unwrap_used, clippy::panic)]

#[allow(dead_code)]
#[path = "benchmark_manifest/fixtures.rs"]
mod fixtures;

use std::cell::{Cell, RefCell};

use fixtures::{bytes, context_variant, document};
use serde_json::{Value, json};
use sts2_harness::benchmark_manifest::{
    Manifest, Mismatch, RerunAdmission, RerunAllocationSeam, RerunGateError, admit_and_allocate,
};

fn parse(value: &Value) -> Manifest {
    Manifest::parse_private(&bytes(value)).unwrap()
}

/// Production seam with a call counter and a record of the admitted declaration,
/// so both the ordering claim and the token-to-declaration binding are observable.
#[derive(Default)]
struct CountingSeam {
    calls: Cell<u32>,
    admitted_digest: RefCell<String>,
}

impl CountingSeam {
    fn calls(&self) -> u32 {
        self.calls.get()
    }

    fn admitted_digest(&self) -> String {
        self.admitted_digest.borrow().clone()
    }
}

impl RerunAllocationSeam for CountingSeam {
    type Output = &'static str;
    type Error = &'static str;

    fn allocate(&self, admission: &RerunAdmission) -> Result<Self::Output, Self::Error> {
        self.calls.set(self.calls.get() + 1);
        *self.admitted_digest.borrow_mut() =
            admission.declaration().artifact_digest_private().to_owned();
        Ok(admission.evidence())
    }
}

/// One controlled field change with the exact refusal it must produce.
fn refusals() -> Vec<(&'static str, Value, Vec<Mismatch>)> {
    let base = document();
    let mut cases: Vec<(&'static str, Value, Vec<Mismatch>)> = Vec::new();
    for (label, pointer, replacement, reason) in [
        (
            "different seed",
            "/gameplay/effective_seed",
            json!("other-seed"),
            Mismatch::Seed,
        ),
        (
            "different unlocks",
            "/gameplay/unlock_progress_digest",
            json!("c".repeat(64)),
            Mismatch::UnlockProgress,
        ),
        (
            "different profile",
            "/gameplay/profile_artifact/reference",
            json!("profile-artifact-2"),
            Mismatch::ProfileArtifact,
        ),
        (
            "different assemblies",
            "/gameplay/assembly_hashes/synthetic-assembly",
            json!("d".repeat(64)),
            Mismatch::Assemblies,
        ),
        (
            "different component build",
            "/gameplay/components/game_mod/package_digest",
            json!("d".repeat(64)),
            Mismatch::Components,
        ),
        (
            "different game build version",
            "/gameplay/game_version",
            json!("synthetic-game-v2"),
            Mismatch::GameVersion,
        ),
    ] {
        let mut changed = base.clone();
        *changed.pointer_mut(pointer).unwrap() = replacement;
        cases.push((label, changed, vec![reason]));
    }
    for (label, from, to) in [
        (
            "different act order",
            "\"act_1\",\"act_2\"",
            "\"act_2\",\"act_1\"",
        ),
        (
            "different modifiers",
            "\"modifiers\":[]",
            "\"modifiers\":[\"synthetic-modifier\"]",
        ),
    ] {
        let mut changed = base.clone();
        changed["gameplay"]["selected_context"] = context_variant(from, to);
        cases.push((label, changed, vec![Mismatch::SelectedContext]));
    }
    cases
}

/// The accepted seeded-run-v1 context pins `character` (and `game_mode`) to a single
/// value, so a different character is not representable as a validated manifest. It is
/// therefore refused even earlier than the gate, by the strict parser, which is still
/// before any game mutation. The gate's `SelectedContext` equality would reject it too
/// once the declared vocabulary admits more characters.
#[test]
fn a_different_character_is_refused_before_mutation() {
    let mut changed = document();
    changed["gameplay"]["selected_context"] =
        context_variant("\"character\":\"ironclad\"", "\"character\":\"silent\"");
    assert!(Manifest::parse_private(&bytes(&changed)).is_err());
    let mut mode = document();
    mode["gameplay"]["selected_context"] =
        context_variant("\"game_mode\":\"standard\"", "\"game_mode\":\"other\"");
    assert!(Manifest::parse_private(&bytes(&mode)).is_err());
}

#[test]
fn equal_declarations_admit_at_the_declaration_boundary_only() {
    let incumbent = parse(&document());
    let candidate = parse(&document());
    let seam = CountingSeam::default();
    let admitted = admit_and_allocate(&incumbent, &candidate, &seam).unwrap();
    assert_eq!(admitted, "declared_inputs_equal");
    assert_eq!(seam.calls(), 1);
    // The seam received exactly the declaration that compared equal.
    assert_eq!(seam.admitted_digest(), candidate.artifact_digest_private());
    // An admission is not native compatibility and not an authorization to mutate.
    assert_ne!(admitted, "native_compatible");
    assert_ne!(admitted, "authorized");
    assert_eq!(
        incumbent
            .admit_rerun(&candidate)
            .unwrap()
            .declaration()
            .artifact_digest_private(),
        candidate.artifact_digest_private()
    );
    assert_eq!(
        incumbent.admit_rerun(&candidate).unwrap().evidence(),
        "declared_inputs_equal"
    );
}

#[test]
fn each_controlled_difference_refuses_with_its_exact_reason() {
    let incumbent = parse(&document());
    for (label, changed, expected) in refusals() {
        let seam = CountingSeam::default();
        match admit_and_allocate(&incumbent, &parse(&changed), &seam) {
            Err(RerunGateError::Refused(refusal)) => {
                assert_eq!(refusal.mismatches(), expected, "{label}");
            }
            other => panic!("{label}: expected refusal, got {other:?}"),
        }
        assert_eq!(
            seam.calls(),
            0,
            "{label}: allocation reached before compare"
        );
    }
}

#[test]
fn refusal_returns_before_the_allocation_seam_is_invoked() {
    let incumbent = parse(&document());
    let mut changed = document();
    changed["gameplay"]["selected_context"] = context_variant(
        "\"act_1\",\"act_2\",\"act_3\",\"act_4\"",
        "\"act_4\",\"act_3\",\"act_2\",\"act_1\"",
    );
    // One seam across both calls: the refused attempt must leave it untouched, and
    // the admitted attempt must then reach it exactly once.
    let seam = CountingSeam::default();
    let refusal = admit_and_allocate(&incumbent, &parse(&changed), &seam).unwrap_err();
    assert!(matches!(refusal, RerunGateError::Refused(_)));
    assert_eq!(seam.calls(), 0);
    assert_eq!(seam.admitted_digest(), "", "seam saw no declaration");

    let admitted = admit_and_allocate(&incumbent, &parse(&document()), &seam).unwrap();
    assert_eq!(admitted, "declared_inputs_equal");
    assert_eq!(seam.calls(), 1);

    let refusal = incumbent.admit_rerun(&parse(&changed)).unwrap_err();
    assert_eq!(refusal.mismatches(), vec![Mismatch::SelectedContext]);

    // The minted token is bound to the declaration that compared equal, never to an
    // unrelated one.
    let unrelated = parse(&document());
    let forged = incumbent.admit_rerun(&unrelated).unwrap();
    assert_eq!(
        forged.declaration().artifact_digest_private(),
        unrelated.artifact_digest_private()
    );
}
