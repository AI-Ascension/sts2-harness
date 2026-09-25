# Changelog archive: 2026-09-25

This file preserves completed `## Unreleased` history that was moved out of
[`CHANGELOG.md`](../CHANGELOG.md) when the active changelog reached its preferred Markdown size
budget. Entries are unchanged from the revision that introduced them apart from relative link paths,
which are corrected so they resolve from this directory; this file is a verbatim record, not a
supported release or a second normative changelog.

### Archived from CHANGELOG.md

- **Enforce the prepared-input token-measurement invariant on the read path.** `TokenMeasurement`
  claimed that `tokens` is `None` exactly when the provenance is `Unavailable`, but its fields were
  public and its derived deserializer accepted any shape, so a record claiming an absent provenance
  beside a byte count deserialized and `PreparedInputBudget::tokens()` reported that byte count as a
  token count. The fields are now private behind read accessors, and deserialization re-validates
  exactly what the constructors validate: `Unavailable` with a quantity, a non-`Unavailable`
  provenance with no quantity, `tokens == 0` and an invalid method are rejected rather than read
  back as a measurement. The Unicode eviction test now discriminates byte accounting from character
  accounting. No durable record, published schema, or consumer pin changes. Refs #381.

- **Refuse a served assembled input that does not fit beside its advertised output reserve.** The
  pre-existing `max_context_bytes` check bounded the request bytes alone; there was no served bound
  over the whole bytes actually sent. `ContextRenderLimits` and the context-owner descriptor both
  gain an optional `output_reserve_bytes`: `None` is exactly the prior contract, where response
  capacity stays bounded by the provider configuration, and a published reserve makes
  `max_context_bytes` the combined whole-input bound. The served managed decision then admits the
  assembled provider bytes against that bound before any dispatch, refusing
  `context_whole_input_budget_exceeded` and an unusable advertised reserve with
  `context_whole_input_budget_invalid`. Compatibility: additive; the field is optional and
  skip-serialized, so a descriptor that does not advertise it serializes byte-identically. See
  [ADR 0058](decisions/0058-served-whole-input-output-reserve.md). Refs #107.

- Add a read-only [frozen Jev pilot profile](../experiments/jev-evaluation/PILOT.md): ten pairs,
  twenty reserved attempts, exact-manifest reconciliation, per-arm refusal/gate diagnostics and
  matched input-token/latency accounting. No policy change or live gameplay benefit is claimed.

- Add an explicitly approved [paired Jev replay runner](../experiments/jev-evaluation/RUNNER.md):
  pinned matching inputs, reserved budgets, independent redacted captures, bounded Unix processes
  and read-only recovery. No game action is dispatched; native/provider benefit remains unverified.
- **Persist authoring-inference reservations in the management store instead of only in memory.**
  Requirement 4 of the bounded authoring-inference surface says a reservation is *persisted* and
  AC4 says a *restart* preserves one operation with an honest cost/outcome state, but the only
  non-test journal was `MemoryAuthoringInferenceJournal` — a process-local map — so the guarantee
  held no further than a re-composition inside one process. The new
  `SqliteAuthoringInferenceJournal` keeps each reservation and its terminal outcome in the same
  `SqliteWorkflowStore` the served composition already opens: `begin` inserts with
  `INSERT OR IGNORE` and reads back, so two callers cannot both start, and `complete` repeats the
  terminal guard as a `WHERE ... json_extract(record,'$.state') = 'pending'` predicate, so a racing
  terminal write cannot be overwritten. The existing restart test could not see the gap: it passed
  the *same* `Arc<MemoryAuthoringInferenceJournal>` to both services. The added test drops the
  first service, reopens the database from disk and re-composes over a **new** journal; the
  counterfactual (the same test with the in-memory journal in both services) fails at the provider
  call count, 2 instead of 1 — i.e. it re-contacts the provider, which is the harm the requirement
  exists to prevent. Additive: the in-memory journal is unchanged, no route or schema is removed.
  Refs #105. The issue stays open for the HTTP-boundary conflict case and for the owner's
  scoping decision on whether AC4's restart is meant to be cross-process.
