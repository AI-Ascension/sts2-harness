# ADR 0066: Admit trace bundles by closure before comparing, and bound the reproducer

Status: accepted for the harness-owned offline divergence diagnostic. It records the contract the
owner selected for the source-only slice of issue
[#124](https://github.com/AI-Ascension/sts2-harness/issues/124) — admission before alignment, and a
bounded reproducer prefix. It does not authorize a native host, game lane, profile mutation or
provider spend, and it makes no native-completeness claim; the authorized native mismatch stays an
explicit external validation gate (harness [#123](https://github.com/AI-Ascension/sts2-harness/issues/123)).
It is ratified when the change carrying it merges.

## Context

`crates/harness/src/exact_transition.rs` already compares two validated `TransitionTrace`s by an
aligned linear scan — not endpoint equality — and reports the last equal and first unequal boundary.
`crates/harness/src/trace_diagnosis.rs` splits the result into a privileged face (exact state
identities at the mismatch) and a digest-free `PublicDivergenceStatus`. Two gaps remained for an
offline diagnostic owner:

1. comparison started from two bare traces. Nothing proved that a trace was the immutable recording
   its bundle advertised (closure over the commitment chain), and nothing refused two bundles that
   only look comparable because their profiles or action-schema revisions differ; and
2. there was no bounded reproducer and no explicit record/entry/byte truncation accounting, so a
   large or adversarial trace could produce an unbounded privileged diff, and a "reproducer" would
   have to be assembled by hand without proving it ends at the reported boundary.

The issue's work packages name exactly these: T1 offline comparison admission (validate manifests,
closure and compatible coverage; align by logical settled actions, never wall time or runtime ids)
and T3 reproducer and validation (export the recorded prefix ending at the failing boundary; test
reconverging endpoints, missing evidence and truncation).

## Decision

A new module `crates/harness/src/trace_divergence/` owns the before/after envelope of that
comparison, offline and read-only:

- **Closure admission (`admission.rs`).** `TraceBundleManifest::for_trace` derives a manifest from a
  validated trace: profile, the sorted unique set of action-schema revisions, the source-state
  identity, the record count, and the commitment of the last boundary. `admit_traces` revalidates
  both traces, proves each manifest still `binds` its trace, then refuses (`AdmissionRefusal`) on a
  zero limit bound, an unbound closure, a profile mismatch, empty expected coverage, or different
  action-schema revisions — **before** any alignment. Comparison itself stays in `compare_traces`,
  which aligns by ordinal/kind/phase (logical settled boundaries), so wall time, PID and raw runtime
  ids are never alignment keys.
- **Bounded diagnosis (`bounds.rs`).** `DivergenceLimits` carries explicit `max_records`,
  `max_entries` and `max_bytes`. Both traces are truncated to `max_records` (revalidating the
  shortened chain), and the privileged field diff at the failing boundary is capped by the entry and
  byte bounds. `LimitTruncation` reports `records_examined`, `records_truncated`, `entries_kept`,
  `entries_dropped`, `bytes_kept` and `bytes_excluded`, so a bounded result is never mistaken for a
  complete one. `FieldDifference` values may be exact digests, so they are privileged only.
- **Reproducer prefix (`reproducer.rs`).** `export_reproducer_prefix` extracts the shortest recorded
  prefix ending at the failing boundary and fails closed (`PrefixTooLarge`) rather than emit a
  clipped prefix that does not reach its boundary. `ReproducerPrefix::validate_against_source` proves
  the prefix is a byte-identical head of the original trace ending at the reported boundary and that
  the boundary commitment still matches; the source is never mutated. This is prefix extraction, not
  delta debugging, and it makes no globally-minimal-causality claim.
- **Entry point (`mod.rs`).** `diagnose_bundles` admits, compares bounded views, builds the bounded
  privileged diff, and attaches the reproducer on divergence. `AdmittedDiagnosis::public_status`
  delegates to the existing digest-free projection.

## Consequences

- A caller can locate the first unequal comparable boundary, see how much was truncated, and hand
  back a reproducer that replays only up to that boundary, without ever publishing exact identities
  to a transcript, log or model input.
- `cargo test -p sts2-harness --test trace_divergence` is the focused production-boundary suite:
  reconverging endpoints, unequal lengths, non-sequential ordinals, duplicate-ordinal and wrong-count
  closure refusals, incompatible schema/profile refusals, zero bounds, empty coverage, prefix
  verification and immutability, unrecorded/oversized boundaries, record truncation, and bounded
  entry accounting.
- Still open in #124: the RNG-stream/cursor face of the first-divergence classification (the
  transition record carries a catalog witness and an external-input digest but no dedicated RNG
  cursor field), and the authorized native mismatch, which remains gated by #123. This ADR records
  the source-only contract only; it does not claim those criteria.

## Evidence

| Claim | Label | Source |
| --- | --- | --- |
| Aligned linear scan and privileged/public split existed before this change | `confirmed` | `exact_transition.rs`, `trace_diagnosis.rs` on `main` `d892932` |
| No manifest/closure/coverage admission, bounded diff, or prefix export existed | `source-derived` | repository search for `reproducer`/`export_prefix`/`truncat` found none in `crates/harness/src` |
| The new module is additive and offline | `confirmed` | `crates/harness/src/trace_divergence/*` + `tests/trace_divergence.rs` |
| Native mismatch validation | `unverified` (external gate) | harness #123 open |

Refs #124. Related: [ADR 0057](0057-harness-semantic-history.md) (bounded traversals),
[ADR 0038](0038-durable-branch-store.md) (durable branch records).
