# Harness decision records

Each record in this directory is named `NNNN-short-title.md`. The number is the
citation label used across the repository (for example `ADR 0017`, or a Markdown
link to the file), so it must denote exactly one decision.

`ADR-WF-004-store.md` uses a distinct prefix and is not a numbered record.

## Renumbering 2026-09-16 (issue #203)

Eight numbers were each held by two different records, so a single label denoted
two decisions. The collisions were resolved without changing any decision
content: the record with the stronger citation evidence keeps its number, and the
other record moves to the next free number above the then-current maximum (0032).
The rule applied to each pair, in order:

1. the record with more in-repo citations (number citations plus filename
   citations) keeps the number;
2. if tied, the record with more filename citations keeps the number, because a
   filename citation is unambiguous;
3. if still tied, the earlier-added record keeps the number, because the later
   record arrived when that number was already taken.

| Old number | New number | Record that moved | Reason |
| --- | --- | --- | --- |
| `0004` | `0033` | `0033-provider-map-context-and-image-boundary.md` | Tied at zero citations; the later record (added 2026-09-07) moved and `0004-minimal-poc-harness-runner.md` kept `0004`. |
| `0006` | `0034` | `0034-runtime-v2-multi-instance-coordinator.md` | `0006-exo-full-run-and-evidence-gates.md` had 4 citations against 0. |
| `0010` | `0035` | `0035-context-control-prepared-input.md` | Tied at zero citations; the later record (added 2026-09-10) moved and `0010-legal-catalog-reobserve.md` kept `0010`. |
| `0011` | `0036` | `0036-seeded-run-transport-v1.md` | `0011-historical-recovery-evidence.md` had 1 filename citation against 0. |
| `0015` | `0037` | `0037-provider-session-boundary.md` | `0015-windows-worker-endpoint-boundary.md` had 2 filename citations against 0. |
| `0016` | `0038` | `0038-durable-branch-store.md` | Tied at 1 citation each; the later record (added 2026-09-13) moved and `0016-checkpoint-integrity-admission.md` kept `0016`. |
| `0021` | `0039` | `0039-game-information-consumer.md` | Tied at 3 citations each; broken on filename citations, 3 for `0021-benchmark-manifest-foundation.md` against 2. |
| `0022` | `0040` | `0040-recorded-context-binding-history.md` | `0022-exo-lookup-duplex-bridge.md` had 5 citations against 4. |

Every in-repo citation of a moved record was updated in the same change (eight
citation sites, plus the number in each moved record's own heading), so no `ADR
NNNN` label and no `NNNN-*.md` link is left dangling or ambiguous. The remaining
records were not renumbered; the only edits to them are citation retargets where
they referred to a record that moved.
