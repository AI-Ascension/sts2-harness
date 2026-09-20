# ADR 0057: Retain and query semantic history with causal provenance

Status: accepted for the harness-owned retained-history surface;
native capture and end-to-end verification pending.

## Context

`sts2-game-mod` states authoritative gameplay events and the coverage of its own capture
(`crates/game-mod/src/semantic_event_reference/`, landed as `AI-Ascension/sts2-game-mod#202`). The
producer's vocabulary is closed and its rule is explicit: a boundary that did not observe an event
discloses a gap instead of inventing one, and a causal parent is reported only when the host stated
one. `sts2-harness#128` requires the harness side of that contract -- a durable, queryable history
with causal provenance.

The harness has no compile-time dependency on the game-mod, and the producer runs on a host this
repository cannot execute. So the harness cannot import the vocabulary as a type, cannot observe a
native event, and cannot verify that any campaign ran. What it can own is everything downstream of
the producer's statement: admission, retention, indexing, bounded query, causal traversal, and
idempotent replay across a restart, a re-delivered batch and a branch fork.

The failure mode this record exists to prevent is a history that *guesses*. A gap silently closed by
an inferred event, a causal parent invented from a difference between two snapshots, or a query
answered from another run's history all produce a plausible artifact that is wrong in a way no
consumer can detect later. Refusal is therefore the contract, not a fallback.

## Decision

1. **The producer vocabulary is mirrored, never imported.** `SemanticEventKind` and
   `SemanticEventOrigin` re-declare the producer's closed inventories in the harness. A producer
   name outside those inventories is refused at admission rather than stored as free-form text a
   consumer would have to interpret. The names are the interface; the harness does not grow a
   compile-time edge to the mod to obtain them.
2. **A disclosed gap is a first-class retained record.** A gap keeps its sequence number, is stored
   beside the observed events, and carries no kind, origin, subject, quantity or reference. The
   capture window declares the intervals it could not observe, so coverage is *stated by the
   producer* rather than inferred from absence: a gap outside every declared interval is refused,
   and a captured event inside a declared interval is refused. An absent event is therefore never
   silently equal to an unwatched one.
3. **Causality is stored only when it was stated.** A record carries
   `SemanticCausalParent::not_stated()` or a stated parent identity; nothing infers a cause from a
   value comparison. An event of a kind that admits no cause may not state one, an event of a kind
   that admits one must state its shape, an imported event may not state a parent at all, and a
   stated parent must exist in the same history and precede its child.
4. **A traversal refuses rather than truncates.** `traverse_causes` walks stated parents backwards,
   visits each event at most once, and stops at the first record that discloses no parent. A chain
   that exceeds the depth or visit bound is refused, and a causal graph that revisits an event is
   refused as a cycle instead of being cut short into a chain that looks complete.
5. **Sequence order is monotonic and contiguous inside one scope, and a cross-scope read is
   refused.** A scope is a run, branch, episode and epoch; sequence numbers are only meaningful
   inside one. A read names a `SemanticHistoryFence`, and a history whose scope does not satisfy it
   is refused rather than answered from another run.
6. **Idempotence is by caller operation identity, not by trust.** Re-appending an identical payload
   returns the retained history unchanged; reusing an append identity with a *different* payload is
   refused rather than overwriting the retained evidence. A fork copies its ancestor's history by
   lineage and appends only what is new, so a replayed or re-joined batch cannot duplicate an event,
   and a fork naming its own branch, or no retained ancestor, is refused.
7. **Every stored identity is bounded, and no stored value can become a path.** The identity rule
   refuses empty, over-bound, control-byte, path-separator and traversal-segment values, and labels
   and quantity units are bounded separately. Aggregate retained bytes, event count, declared
   intervals, page size, and causal visits and depth are each bounded, and an over-large page is
   refused rather than truncated into a page that looks final.
8. **A query narrows and never substitutes.** Each filter field is a narrowing: unset means "any",
   and a filter that matches nothing yields an empty page with its matched count, rather than a
   widened page that appears to answer the question asked.
9. **Durability is one versioned document written by atomic rename.** Each write lands in a sibling
   temporary file and is renamed over the store, so a crash leaves the previous document intact. A
   document that cannot be read, is over its byte bound, or does not parse is refused rather than
   reopened as an empty history, because an empty history is indistinguishable from a lost one. A
   missing file is the one case that legitimately opens empty.
10. **Retention is explicit, reference-aware, and discloses what it removes.** An
    operator-selected `SemanticRetentionPolicy` reuses the shape of the branch store's own retention
    owner -- a protection that must be disabled deliberately, plus a bound below which nothing is
    pruned -- with a count of retained records standing in for an age, because this store keeps no
    clock. A prune is previewed as a `SemanticPrunePlan` before it is applied, and the applied plan
    must equal the one the history produces now: a plan computed against other bytes, or under a
    policy that no longer selects the same records, is refused. What the policy selects is
    **disclosed rather than deleted**: each selected record keeps its identity and sequence number
    and loses every gameplay value, and the window gains a declared span covering it, so a pruned
    span can never read as a measured zero, an empty complete result, or an event that never
    happened. A record another surviving record still names as its stated causal parent is kept
    anyway, and that protection closes transitively over the ancestor chain, so a prune can never
    leave a stated cause unreachable. An append after a prune inherits the spans retention declared,
    because a producer cannot restate a decision the store made.
11. **History leaves the store only through a harness-owned lookup port.** A caller does not reach
    the retained store; it states a `SemanticLookupRequest` to a `SemanticLookupPort`, and the
    harness-owned `RetainedHistoryLookup` serves it from the store it already holds and exposes no
    accessor back to that store. Authority is **not granted by default**: every request is refused
    until a caller is explicitly granted it, so serving history is a deliberate grant rather than a
    capability that arrives with the type. A request that names an unknown field is refused on
    shape, a request over its byte bound is refused, and a result over its own byte bound is refused
    rather than truncated into a page that looks complete. What a lookup returns is a disclosed
    page: a pruned span is stated as retention disclosed and a missing sequence as a gap, so a
   lookup can never read the removal of a record as a measured zero.
12. **Saved native history is restored only through an owned mod port, and it keeps its own labels.**
    History that predates this harness can never be admitted as capture, so it is restored backwards
    through one port the harness owns: the harness declares `SemanticBackfillPort`, the mod
    implements it, and the port yields a batch rather than reaching the store. Authority is **not
    granted by default**, and an ungranted restore is refused *before the port is consulted*, so a
    mod is never asked for native history the harness has not authorized. What comes back is
    admitted under the same vocabulary, identity and bound rules a live batch faces, lands **ahead**
    of the retained capture start rather than being appended to it, and is idempotent by operation
    identity: a re-delivery restores nothing a second time, and reusing that identity with another
    span or binding is refused. Imported history is never relabelled: an observed restored record
    must carry the `Imported` origin so it can never read as an event this harness observed, a
    restored gap must keep the coverage label naming what its source could not see, and a span that
    does not abut the retained start is refused rather than leaving an undeclared hole behind it.

## Consequences

- **A new harness-owned artifact surface.** `sts2_harness::semantic_history` is public API in the
  harness crate. It reaches no host, no game process and no gateway: the store is a local file, and
  the module claims no native event capture.
- **The producer rule is mirrored, and can drift.** Because the vocabulary is re-declared rather
  than imported, a producer that widens its inventory needs a matching harness change; a name the
  harness cannot state is refused, which surfaces the drift as a refusal rather than as stored text.
- **Refusal is visible to callers.** Every refusal is a named
  `SemanticHistoryRefusal` variant carrying the identity that was wrong, so an operator can
  distinguish a gap from a corruption from a scope mismatch; there is no generic failure path and no
  partial success.
- **A re-declared vocabulary is not proof of compatibility.** The harness agreeing with the names
  the merged producer source states does not establish that the host emits them, and no such claim
  is made here.

## Verification

`crates/harness/tests/semantic_history.rs` with
`crates/harness/tests/semantic_history/{admission,persistence,query,retention,lookup,backfill}.rs` covers 97 cases:
coverage-shape and window refusals (decision 2), the causal-shape, absent-parent, non-preceding
parent and imported-causality refusals plus bounded and cycle-refusing traversal (decisions 3 and
4), the cross-branch and cross-epoch read refusals and bounded paging with continuation (decisions
5 and 8), idempotent re-append, conflicting reuse, contiguity and fork lineage (decision 6), the
identity and bound refusals (decision 7), and restart, missing-store and corrupt-document behaviour
(decision 9), and the retention preview/apply pair, the reference-aware pin and its transitive
closure, the stale-plan and reused-identity refusals, the disclosed-span bound, restart, fork and
post-prune append behaviour over a disclosed span (decision 10), and the authority gate, both byte
bounds, the unknown-field refusal, retention disclosure, and the scope fence holding on an empty as
well as a non-empty history (decision 11), and the ungranted restore, the granted restore landing
ahead of the retained capture, the imported-origin and gap-label rules, the non-abutting and
over-long and short span refusals, the binding, fence and scope refusals, the same-rules-as-a-live
batch admission, the re-delivered and conflicting-identity restores, the restore that reaches the
first sequence and leaves nothing before capture, the restart that keeps the restored span where
capture begins, and the port that cannot read saved history (decision 12).

These establish component behaviour over batches the harness constructed. They do **not** prove a
native event capture, a capture window the host actually declared, or that any campaign ran; those
remain separate gates. `sts2-harness#128` requirement 1 and its acceptance criteria stay unmet, and
the producer-side vocabulary is read from the merged `sts2-game-mod` source rather than executed
here.
