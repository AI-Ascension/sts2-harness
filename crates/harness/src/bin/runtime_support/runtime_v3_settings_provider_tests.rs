// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use super::{ProviderKind, parse};

/// Every spelling the repository or its documentation already uses stays an admitted kind.
///
/// These are the exact strings in `docs/OLLAMA_MODEL_SELECTION.md`, `docs/JEV_MODEL_SELECTION.md`,
/// `experiments/live-combat/README.md` and the executable-composition fixtures. A kind that is
/// renamed or dropped without those callers cannot compile a run again by accident, which is the
/// point of naming them here.
#[test]
fn each_declared_spelling_is_its_own_kind() {
    for (spelling, kind) in [
        ("openai-astra", ProviderKind::OpenAstra),
        ("ollama", ProviderKind::Ollama),
        ("typesafe-jev", ProviderKind::TypesafeJev),
        ("exo", ProviderKind::Exo),
        ("synthetic", ProviderKind::Synthetic),
    ] {
        assert_eq!(parse(spelling), Ok(kind), "{spelling} must name {kind:?}");
        assert_eq!(
            kind.name(),
            spelling,
            "{kind:?} must round-trip its spelling"
        );
    }
}

/// An unimplemented name is refused rather than read as the implicit reviewed-Exo lane.
///
/// Before this, an unknown spelling took the non-bridge branch and ran under the reviewed source
/// revision, so a typo silently selected a lane nobody named. The refusal has to name the value to
/// be actionable, and it must not be reachable by a near-miss of an admitted spelling.
#[test]
fn an_unimplemented_name_is_refused() {
    for rejected in [
        "",
        " ",
        "openai_astra",
        "openai-astra ",
        "OPENAI-ASTRA",
        "Openai-Astra",
        "astral",
        "exo-bridge",
        "ollama2",
        "typesafe_jev",
        "synthetics",
        "legacy",
    ] {
        let error = parse(rejected).expect_err(rejected);
        assert!(
            error.contains(rejected) && error.contains("STS2_PROVIDER_KIND"),
            "{rejected} must be refused by name: {error}"
        );
    }
}

/// The two properties that decide what a lane may do are declarations on the kind.
#[test]
fn bridge_and_live_capability_are_declared_per_kind() {
    for kind in [
        ProviderKind::OpenAstra,
        ProviderKind::Ollama,
        ProviderKind::TypesafeJev,
        ProviderKind::Exo,
        ProviderKind::Synthetic,
    ] {
        // Only the reviewed envelope binds the Exo executor's identity, so it is not a bridge the
        // operator pins beside the declaration.
        assert_eq!(
            kind.is_local_bridge(),
            matches!(
                kind,
                ProviderKind::OpenAstra | ProviderKind::Ollama | ProviderKind::TypesafeJev
            )
        );
        // A probe lane is never a live episode, and neither is a lane ADR 0053 kept out of one.
        assert_eq!(
            kind.admits_live_episode(),
            matches!(kind, ProviderKind::OpenAstra | ProviderKind::Exo)
        );
        // A local bridge's live capability is the digest pin; the Exo executor's is the descriptor.
        assert_eq!(kind.requires_reviewed_envelope(), kind == ProviderKind::Exo);
        assert!(
            !(kind.requires_reviewed_envelope() && kind.is_local_bridge()),
            "{kind:?} cannot require both backings"
        );
    }
}

/// A live-capable kind other than Astra is not exempt from the reason Astra is live-capable.
#[test]
fn only_a_live_capable_kind_is_live_capable() {
    assert!(ProviderKind::OpenAstra.admits_live_episode());
    assert!(ProviderKind::Exo.admits_live_episode());
    assert!(!ProviderKind::Ollama.admits_live_episode());
    assert!(!ProviderKind::TypesafeJev.admits_live_episode());
    assert!(!ProviderKind::Synthetic.admits_live_episode());
}
