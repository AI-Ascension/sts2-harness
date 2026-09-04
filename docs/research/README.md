# STS2 harness research

This directory contains self-contained, evidence-labeled research specifications for the
`sts2-harness` target. These documents guide later experiments; they are not game authority,
runtime support declarations, or proof that a proposed state/action surface exists in a target
build.

## Current document

- [`STS2_EXPERT_STATE_INFORMATION_ARCHITECTURE_2026-09-03.md`](STS2_EXPERT_STATE_INFORMATION_ARCHITECTURE_2026-09-03.md)
  — build-pinned research update for a fair-play autonomous harness, including the proposed
  state/action inventory, typed memory, recovery, evaluation, and patch-drift plan.

The research uses the harness evidence vocabulary: `confirmed`, `source-derived`, `inferred`,
`proposed`, `unverified`, and `unsupported`. The existing bounded host probe is linked only as
repository evidence; it does not establish full gameplay, game-rule parity, or exhaustive state
discovery. No proprietary game files, saves, credentials, screenshots, model output, or generated
binary artifacts are retained here.
