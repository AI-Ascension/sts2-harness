# ADR 0054: Inference-Profile Revision Adoption Scope

## Status

Accepted for the Harness inference-profile admission boundary. This record does
not authorize a provider, native-host, game, deployment or paid-call lane, and it
does not give the harness authority to edit protected inference configuration.
It records **which object** issue [#104](https://github.com/AI-Ascension/sts2-harness/issues/104)
acceptance criterion 3 governs.

## Context

Issue #104 acceptance criterion 3 reads: *concurrent edits use CAS; adopting a new
revision changes only a new definition, never an admitted run.* Two different
change objects in this repository can plausibly be read as "a new revision":

1. the **inference-profile revision** introduced by this issue — an immutable
   descriptor (adapter, requested/resolved model, prompt/configuration revision,
   supported settings, operation allow-list, context compatibility, continuity,
   effective budgets) carrying an id/version/digest identity; and
2. the **merged #255 idle-rebind session-policy** semantics, where a live session
   rebinds its policy on idle rather than being torn down.

The second is a *session lifecycle* behaviour that [ADR 0046](0046-invocation-context-membership.md)
and the provider-session policy records already own. If AC3 were read as covering
it, an idle rebind would have to be proven never to affect an admitted run, which
would forbid the rebind itself. That is not what the criterion is about: the
criterion constrains *configuration identity* adoption, not session idleness.

## Decision

**AC3 applies only to the new inference-profile revision object.** Reading AC3 as
also covering the merged #255 idle-rebind session-policy semantics is explicitly
rejected.

The scope is therefore:

1. **Immutability.** A served inference profile is an immutable, digest-fenced
   revision. An accepted edit produces a *new* revision; the previous one is never
   rewritten in place.
2. **CAS on edit.** An edit is admitted only against the exact expected revision
   (compare-and-swap), so a concurrent edit loses rather than silently overwriting
   the winner.
3. **Adoption is definition-scoped.** Adopting a new revision changes a *new*
   definition only. An already-admitted run keeps the exact id/version/digest it
   resolved at admission, so a later catalog refresh or a newer revision cannot
   retarget a run that is already in flight.
4. **Session idleness is out of scope.** The #255 idle-rebind behaviour is
   unchanged by this record and is not a revision-adoption event. It is neither
   strengthened nor weakened here.

## Ownership split

Issue #104 is delivered in two lanes:

- **104-A (this lane)** serves the immutable catalog, resolves typed node
  bindings to exact revisions, persists requested/resolved provenance without
  credentials, and refuses unknown id, digest mismatch, revocation and
  unsupported model/settings before any inference. It states the AC3 scope in
  this record.
- **104-B (delivered)** owns the CAS revision journal and the admitted edit
  route that physically implement concurrent-edit CAS and new-definition
  adoption. `POST /v1/inference-profiles/{profile_id}/revisions` requires both
  `workflow:content:write` and an owner-published `grants.edit`, so the read and
  select grants that authorize discovery confer no edit authority. It admits an
  edit only against the exact expected revision digest — one of two concurrent
  edits is adopted and the other is reported as a conflict naming the winner —
  and appends a **new** revision, never rewriting the one it replaced; a
  repeated caller mutation identity is replayed rather than applied twice.
  Adoption stays definition-scoped: the accepted revision is recorded and
  reported as the exact `profile_id:version:digest` reference a new definition
  pins, while an admitted run's provenance is written once at admission, so a
  newer revision refuses an admitted definition rather than retargeting it.

  The request restates only the fields an owner publishes as editable (version,
  prompt revision, settings revision, supported settings, effective budgets) and
  is closed. Every identity and authority field — adapter, requested and
  resolved model, node kinds, operations, context compatibility, continuity,
  state and grants — is inherited from the edited revision, so this record's
  statement that it "does not give the harness authority to edit protected
  inference configuration" still holds: there is no field through which an edit
  could express one.

  The owner remains the only publisher of what it serves. The journal records
  and names an accepted revision; it is not spliced into the owner-served
  catalog, so a newly accepted revision becomes resolvable for new definitions
  when its owner publishes it.

## Consequences

- The AC3 citation in a future change refers to this record, so "revision" cannot
  be silently reinterpreted as "session policy" in either direction.
- `repo-policy --strict` only checks this directory for duplicate four-digit
  number prefixes (`ADR001`); it has no required-file list, so adding this record
  needs no policy entry. The record is kept below the preferred Markdown budget so
  it does not become a size finding.
- 104-A can be complete and reviewed for AC1/AC2/AC4 without waiting on the
  104-B journal, and the two lanes cannot both claim AC3.
- A profile the owner publishes without `grants.edit` cannot be edited through
  this route at all, which is how the one served live profile stays uneditable
  while remaining discoverable and selectable.

## Verification

Documentation-only decision; no code, provider, host, game or credential is
contacted. The AC1/AC2/AC4 halves this record scopes are proven by
`crates/harness/tests/inference_profile_catalog.rs` against
`contracts/inference-profile/catalog.schema.json` with synthetic fixtures only.
The 104-B edit route is proven by `crates/harness/tests/inference_profile_revision.rs`
against `contracts/inference-profile/revision.schema.json` with the same kind of
synthetic fixtures, including the concurrent-edit swap, mutation replay,
immutability of the replaced revision, restart durability of the SQLite journal,
the two independent authorities and the definition-scoped adoption above.
