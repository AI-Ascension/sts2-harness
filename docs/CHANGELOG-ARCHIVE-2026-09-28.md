# Changelog archive: 2026-09-28

This file preserves completed `## Unreleased` history that was moved out of
[`CHANGELOG.md`](../CHANGELOG.md) when the active changelog reached its preferred Markdown size
budget. Entries are unchanged from the revision that introduced them apart from relative link paths,
which are corrected so they resolve from this directory; this file is a verbatim record, not a
supported release or a second normative changelog.

### Archived from CHANGELOG.md

- **Size the jev process-teardown pipe-cleanup bound above host-load jitter.** The paired runner gave
  a killed process group 250 ms to close an inherited pipe and reported `child_closed: false` past
  that, but a clean host's kill-to-close tail already reaches 250-306 ms under load, so the flag read
  "closure unconfirmed" for a process that closed a millisecond later and the offline runner-process
  contract test flaked. The grace is now a documented 1000 ms contract value, and a new escaped-
  session control proves the flag stays false when a bound is genuinely spent, so the assertion was
- Add Linux [Jev streaming mode](../experiments/jev-plays-sts2/STREAMING.md): retain the game with manual resume; preserve timed benchmarks. Automatic terminal progression remains unavailable.

- **Hold one live decision attempt so a lost reply cannot buy a second one.** A live `Decide` node
  took a fresh `ModelExecutionId` on every entry and kept no record of the attempt, so an
  `Unresolved` refusal left the run `NeedsOperator` with `pending_operation: null` and the next
  `Step` paid the provider again. The attempt is now installed and durably recorded before
  `decide_for`, released only by a refusal the provider owner reported before it could write, and
  re-used by a retry that reproduces the admitted request digest. Compatibility: additive; no wire or durable record changes. See [ADR 0059](../docs/decisions/0059-held-live-decision-attempt.md). Refs #108.
