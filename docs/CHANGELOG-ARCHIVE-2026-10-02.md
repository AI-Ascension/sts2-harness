# Changelog archive: 2026-10-02

This file preserves completed `## Unreleased` history that was moved out of
[`CHANGELOG.md`](../CHANGELOG.md) when the active changelog reached its preferred Markdown size
budget. Entries are unchanged from the revision that introduced them apart from relative link
paths, which are corrected so they resolve from this directory; this file is a verbatim record,
not a supported release or a second normative changelog.

- **Admit explicitly scoped research inspection of hidden checkpoint state.** A new
  `research_inspection` module fixes the source-only contract behind #129 and separates privileged
  research data from the ordinary player-visible boundary: an operator-supplied grant binds one exact
  checkpoint, run, branch and consumer lane to an explicit, bounded set of field groups, so a
  gameplay lane cannot escalate by asking for a different visibility parameter, and revocation is
  monotonic so a replayed request cannot outlive its approval. Fields come from a closed matrix whose
  references refuse paths, queries and unbounded names; admission returns the admitted slice rather
  than fabricating availability, and the native owner's report must match it field-for-field and
  in order. Coverage stays distinct — `NotMaterialized`, `SimulationRequired` and `Unsupported` never
  collapse into zero, empty or an invented value — refusals carry no value, field name or digest, and
  paging is bounded so a partial page is never labelled complete
  ([ADR 0076](decisions/0076-scoped-research-inspection-of-hidden-checkpoint-state.md)). The
  native capture read adapter and the capture-manifest agreement remain open. Refs #129.

- **Bind readiness settlement to its proof, and let a starved wait expire.** The
  `management::readiness_wait` contract behind #96 now admits one `MilestoneObservation`, which binds
  the sealed owner readiness proof to the milestone and process generation that owner reported, so
  the facts that decide settlement are no longer separate `observe` arguments that one owner's proof
  could be paired with a milestone nobody reported. `ReadinessWait::expire_if_elapsed` advances the
  bounded clock without an observation, so a wait that stays starved times out instead of staying
  open forever, and an expired wait can never be satisfied afterwards. Compatibility: additive to the
  milestone vocabulary, the versioned target and the refusal vocabulary; the Studio round-trip and
  the native loading check remain open. Refs #96.
