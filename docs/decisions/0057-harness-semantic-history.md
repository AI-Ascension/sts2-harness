# ADR 0057: Queryable semantic run history with causal provenance

Status: accepted for the harness-owned history contract; native capture and an
end-to-end query over one controlled run remain unverified.

## Context

`sts2-harness#128` asks for queryable semantic combat and run history with causal
provenance. The game-mod companion states the bounded semantic event vocabulary --
the host says what it authoritatively observed -- and was merged separately as
`semantic_event_reference` (`c881883`). Nothing in the harness owned the history
behind that vocabulary, so the vocabulary had no durable record to be asked
about: a consumer could not ask what happened in a run, in what order, or why a
value changed.

The hazard here is not a missing feature but a dishonest one. A history that
infers a cause from the difference between two snapshots, renumbers a sequence
to make it look contiguous, closes a capture gap with an invented event, or
reports an unobserved quantity as zero would answer every question and be wrong
in each. A history that guesses is worse than a history that admits a gap,
because the guess is indistinguishable from an observation once stored.

## Decision

1. **History is harness-owned and appended under a strict identity and order.**
   One store serves one owner binding, and each event is appended against its
   run, branch, episode, authority epoch and host sequence. Within one epoch the
   sequence must strictly advance; a jump is admitted only when the window
   declares the intervening span as a gap, and advancing the epoch is the single
   operation that resets sequence expectations, because a new epoch may restart
   host sequencing honestly. A sequence that moves backwards or repeats is
   refused rather than renumbered.
2. **Coverage is disclosed, never closed.** The capture window states where
   capture began and which spans were dropped or are unsupported, and a recorded
   gap keeps its sequence number. A captured event may not sit inside a declared
   gap, no two spans may overlap, and no gap may precede `capture_start`, so an
   interval this boundary did not observe is reported as unknown rather than as
   an empty result, an invented event or a zeroed value.
3. **A causal parent is stated or absent, never inferred.** A parent is either
   explicitly named by the host or explicitly stated as unknown; the two arms may
   never be mixed, so "we do not know why" cannot decode as "this was the
   cause". A stated parent must exist on the same branch and epoch and strictly
   precede its child, and an imported event may never state one, because an
   imported event's causality was settled when it was captured.
4. **Appends are idempotent and retention redacts rather than destroys.** One
   identity with identical content replays the recorded outcome and writes
   nothing; the same identity with different content is refused as a conflict
   rather than accepted as a correction. Retention replaces a value payload with
   an explicit `Unavailable` while keeping the event, its coverage and its causal
   link, and leaves the content digest as recorded, so neither privacy nor replay
   can turn one event into two or a value into a different one.
5. **There is one supported read path.** History is reachable only through a
   harness-owned port whose request vocabulary contains no storage coordinate. A
   caller that asks to read storage itself is refused a port, and a granted port
   re-checks the source's owner and authority epoch on every read rather than
   treating the grant as a durable capability, so a source that later drifts
   cannot keep answering through a stale grant.

## Consequences

- **The harness owns a new durable record series.** `semantic_history` is a
  harness-owned store and query surface, separate from provider conversation
  history and from `decision_replay`. No provider message can become an event,
  and nothing here reads or writes a provider transcript.
- **An unreadable or unauthoritative input fails closed.** An event whose
  identity could be read as a host path is refused, as is a subject outside the
  live-instance namespace and a quantity-changing kind that states no value. The
  refusals are the contract, not incidental validation.
- **Bounded reads are explicit.** A page or causal traversal that stops at a
  limit reports that it is truncated, so a caller cannot mistake a bounded answer
  for a complete one, and a traversal refuses a cycle rather than looping.
- **This establishes the component contract only.** It does not prove that the
  host emits the vocabulary, that a capture ran, or that a query answered over a
  live run.

## Verification

`crates/harness/tests/semantic_history_core.rs`,
`semantic_history_sequence.rs`, `semantic_history_causal.rs`,
`semantic_history_validation.rs`, `semantic_history_values.rs`,
`semantic_history_readonly.rs` and `semantic_history_page.rs` cover the
decisions above deterministically, with no socket, process, clock or live game:
strict sequencing and declared jumps, gaps that stay declared, the stated/absent
parent rule and its same-branch, same-epoch, strictly-preceding requirements, the
imported-parent refusal, idempotent replay versus conflict, retention that keeps
the event and its digest, the query bounds and digest binding, and the granted
port that re-checks owner and epoch and refuses a caller naming storage directly.

Each guard is falsified, not merely asserted: flipping the lineage-depth check,
the causal-parent arm refusal, the `..` traversal refusal, the live-instance
subject rule, the quantity-value requirement, the conflict-on-different-content
rule, the retention redaction, the per-read epoch re-check, the declared-gap
refusal and the undeclared-jump refusal each fails exactly its named test and
nothing else, and the file is restored byte-identical afterwards.

These tests do not execute the game-mod producer, capture a native run, or answer
an end-to-end query over a controlled run. Those remain separate gates, and
`sts2-harness#128` stays open until they are met.
