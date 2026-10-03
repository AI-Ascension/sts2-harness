// SPDX-License-Identifier: MIT

//! The synthetic inference-profile catalog serves every reference the admitted
//! Phase 1 workflow fixtures actually carry (Refs #799).
//!
//! These fixtures live in the Studio repo at
//! `contracts/accepted/phase1/workflows/`. Each names a floating `sts2.*`
//! profile in a `decision_profile_ref` or `planner_profile_ref`, and each
//! `decide` node declares the matching `sts2.<name>.context.v1`. The owner
//! serves those names so a consumer can reach a real publication; before it did,
//! the owner refused every one of these documents with
//! `inference_profile_unknown`.
//!
//! Everything here is synthetic and labelled as such. The catalog is an in-memory
//! owner double with no provider, model, credential or lease. A pass here proves
//! the owner's resolution and admission fences work. It proves nothing about
//! Exo, about a provider, or about the game.

#![allow(clippy::expect_used)]

/// Every `sts2.*` reference the admitted Phase 1 workflow fixtures carry must
/// resolve through the owner's own catalog, and the pin the owner hands back
/// must be the exact sealed identity rather than a guess the consumer made.
/// This is the property #799 asks for: the fixtures name floating ids, so the
/// owner resolves each one authoritatively. Before this catalog served these
/// names the owner refused every one of them with `inference_profile_unknown`
/// and no Phase 1 document could reach publication.
#[test]
fn every_phase_one_fixture_reference_resolves_to_an_exact_owner_pin()
-> Result<(), Box<dyn std::error::Error>> {
    let catalog = sts2_harness::management::synthetic_inference_profile_catalog()?;
    // The synthetic owner's own two labelled fixtures, plus one descriptor per
    // `sts2.*` inference-profile reference the admitted Phase 1 workflow
    // fixtures actually carry. The total is asserted rather than left open so a
    // descriptor cannot be dropped silently: each of these names is a document
    // the owner is expected to resolve, and losing one makes the consumer
    // refuse that document with `inference_profile_unknown`.
    assert_eq!(catalog.descriptors.len(), 13);
    // (profile_id, node_kind, context_ref the fixture's own node declares).
    // An empty context means the node declares none, which is what the two
    // `adaptive_region` fixtures do.
    let references: [(&str, &str, &str); 11] = [
        (
            "sts2.campaign.decision.v1",
            "decide",
            "sts2.campaign.context.v1",
        ),
        (
            "sts2.combat.decision.v1",
            "decide",
            "sts2.combat.context.v1",
        ),
        ("sts2.event.decision.v1", "decide", "sts2.event.context.v1"),
        ("sts2.map.decision.v1", "decide", "sts2.map.context.v1"),
        ("sts2.rest.decision.v1", "decide", "sts2.rest.context.v1"),
        (
            "sts2.reward.decision.v1",
            "decide",
            "sts2.reward.context.v1",
        ),
        (
            "sts2.selection.decision.v1",
            "decide",
            "sts2.selection.context.v1",
        ),
        ("sts2.setup.decision.v1", "decide", "sts2.setup.context.v1"),
        ("sts2.shop.decision.v1", "decide", "sts2.shop.context.v1"),
        ("sts2.combat.planner.v1", "adaptive_region", ""),
        ("sts2.map.planner.v1", "adaptive_region", ""),
    ];
    for (profile_id, node_kind, context_ref) in references {
        let descriptor = catalog.resolve(profile_id, node_kind)?;
        assert_eq!(descriptor.profile_id, profile_id);
        // The descriptor's own context compatibility must admit exactly the
        // context its fixture declares, or the owner's context fence refuses
        // the document later in `resolve_definition`.
        if context_ref.is_empty() {
            assert!(
                descriptor.context_compatibility.is_empty(),
                "{profile_id} declares no context, so it must not claim any"
            );
        } else {
            assert!(
                descriptor
                    .context_compatibility
                    .iter()
                    .any(|value| value == context_ref),
                "{profile_id} must admit the context its own fixture declares: \
                 {context_ref}"
            );
        }
        // One available revision per profile keeps a floating reference
        // unambiguous. A second available revision would make resolution
        // refuse as ambiguous and un-admit the fixtures this catalog exists to
        // serve, so pin the invariant rather than trusting the list.
        assert_eq!(
            catalog
                .descriptors
                .iter()
                .filter(|other| other.profile_id == profile_id)
                .count(),
            1,
            "{profile_id} must keep exactly one available revision"
        );
        // The pin the consumer records is derived from the sealed descriptor,
        // never from the reference string the document happened to carry.
        let pin = format!(
            "{}:{}:{}",
            descriptor.profile_id, descriptor.version, descriptor.digest
        );
        assert_eq!(
            catalog.resolve(&pin, node_kind)?.digest,
            descriptor.digest,
            "re-resolving the owner's own pin must land on the same revision"
        );
    }
    Ok(())
}

/// The extension must stay a fence, not an auto-upgrade. A reference the
/// catalog does not serve, and a pin whose digest does not match the sealed
/// descriptor, must both still fail closed with the owner's own refusal.
#[test]
fn an_unknown_reference_or_a_wrong_digest_still_fails_closed()
-> Result<(), Box<dyn std::error::Error>> {
    let catalog = sts2_harness::management::synthetic_inference_profile_catalog()?;
    assert!(
        catalog
            .resolve("sts2.nonexistent.decision.v1", "decide")
            .is_err(),
        "a known-shaped id the owner does not serve must not resolve"
    );
    let descriptor = catalog.resolve("sts2.combat.decision.v1", "decide")?;
    let wrong_digest = format!(
        "{}:{}:{}",
        descriptor.profile_id,
        descriptor.version,
        "0".repeat(64)
    );
    assert!(
        catalog.resolve(&wrong_digest, "decide").is_err(),
        "an exact pin with the wrong digest must be refused, not silently accepted"
    );
    // The owner's own labelled fixtures keep working; the extension is
    // additive and must not have displaced them.
    assert!(catalog.resolve("decision.synthetic.v1", "decide").is_ok());
    assert!(
        catalog
            .resolve("planner.synthetic.v1", "adaptive_region")
            .is_ok()
    );
    Ok(())
}
