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
`crates/harness/tests/semantic_history/{admission,persistence,query}.rs` covers 47 cases: the
coverage-shape and window refusals (decision 2), the causal-shape, absent-parent, non-preceding
parent and imported-causality refusals plus bounded and cycle-refusing traversal (decisions 3 and
4), the cross-branch and cross-epoch read refusals and bounded paging with continuation (decisions
5 and 8), idempotent re-append, conflicting reuse, contiguity and fork lineage (decision 6), the
identity and bound refusals (decision 7), and restart, missing-store and corrupt-document behaviour
(decision 9).

These establish component behaviour over batches the harness constructed. They do **not** prove a
native event capture, a capture window the host actually declared, or that any campaign ran; those
remain separate gates. `sts2-harness#128` requirement 1 and its acceptance criteria stay unmet, and
the producer-side vocabulary is read from the merged `sts2-game-mod` source rather than executed
here.
