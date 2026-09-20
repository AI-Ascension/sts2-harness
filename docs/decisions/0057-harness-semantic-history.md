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

The sibling vocabulary also models an event as naming *both* ends of itself: a
role-tagged actor, a role-tagged target, and, for the kinds that are about
content, a reference resolved against the content manifest. A harness record
that carried one role-less subject and a string episode could not hold what the
host states: a source and a target that happen to share a token would collapse
into one, a target would have nowhere to live, and an episode would be compared
as text rather than as the number the scope carries. Alignment here is therefore
representational, not decorative.

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
6. **A subject is a role, and a kind states the detail it requires.** An event
   carries a list of subjects, each naming its role (actor or target), the
   namespace it was minted in and an opaque identity. Only a live instance may
   be a subject, one end of an event may be named once, and a subject identity
   may not alias the event, the branch or the run, because those aliases would
   make an identity read as a different kind of thing. Each kind then states
   what it needs: an actor, a target, a bounded quantity, a content reference,
   and whether it admits a cause at all. A missing end is refused rather than
   read as "there was none", and a target or a cause the kind does not act on is
   refused for the same reason. Episode travels as the bounded number it is, not
   as an identity to be compared as text.
7. **A history survives the restart it claims to survive, and is restored as a
   validation rather than a cast.** One document carries the owner scope, the
   capture window, the branch lineage and every recorded event. Restoration
   re-derives the rules the append path applies to each record — its schema and
   branch, the shape of its kind's detail, the epoch it claims, where it sits in
   the capture window, its parent, and its own recomputed digest — and refuses
   the whole document when any of them does not hold, so a truncated or edited
   document can never introduce an event the boundary would have rejected, a
   renumbered sequence, a closed gap or a causal link that never held. The
   lineage is restored under the same rule: every edge names a branch and every
   branch has exactly one edge, exactly one branch is the root, and no ancestry
   is deeper than the bound. What is restored is the history the writer had, or
   no history at all.
8. **Saved history is backfilled only through the same owned port, and only as
   opaque bytes.** An importer hands back batch identities and the bytes an
   owner-side capture wrote; it cannot construct an event, name a window, scope
   or epoch, or state a cause, so the harness re-derives every rule from those
   bytes rather than trusting the caller's own structure. A batch is refused on
   its size before it is parsed — empty, oversized or over the batch bound — and
   afterwards when its schema, member shape, identity opacity, branch, event
   count or sequencing does not hold. An event must already say it was imported:
   one claiming a native or derived origin is refused rather than stamped, so a
   backfill can never introduce history that reads as an observation, and what
   an import writes keeps the coverage and the source label the capture was
   taken under. The batch is applied to a copy of the store and committed only
   once the whole batch held, so a batch that fails partway leaves no partial
   history behind.

## Consequences

- **The harness owns a new durable record series.** `semantic_history` is a
  harness-owned store and query surface, separate from provider conversation
  history and from `decision_replay`. No provider message can become an event,
  and nothing here reads or writes a provider transcript.
- **An unreadable or unauthoritative input fails closed.** An event whose
  identity could be read as a host path is refused, as are a subject outside the
  live-instance namespace, a role named twice, a subject aliasing the event,
  branch or run, a kind that omits the actor, target, quantity or content
  reference it requires, a detail no kind admits, a cause on a kind that cannot
  be caused, and a disclosed gap that carries any observed detail. The refusals
  are the contract, not incidental validation.
- **Bounded reads are explicit.** A page or causal traversal that stops at a
  limit reports that it is truncated, so a caller cannot mistake a bounded answer
  for a complete one, and a traversal refuses a cycle rather than looping.
- **This establishes the component contract only.** It does not prove that the
  host emits the vocabulary, that a capture ran, or that a query answered over a
  live run. The restart above is a restart of this store, written out and read
  back through its own bytes; a capture that reloads and rejoins through the
  game-mod port is a separate gate.

## Verification

`crates/harness/tests/semantic_history_core.rs`,
`semantic_history_sequence.rs`, `semantic_history_causal.rs`,
`semantic_history_validation.rs`, `semantic_history_values.rs`,
`semantic_history_readonly.rs`, `semantic_history_subjects.rs` and
`semantic_history_page.rs` cover decisions 1 to 6 deterministically, with no
socket, process, clock or live game:
strict sequencing and declared jumps, gaps that stay declared, the stated/absent
parent rule and its same-branch, same-epoch, strictly-preceding requirements, the
imported-parent refusal, idempotent replay versus conflict, retention that keeps
the event and its digest, both ends of a targeted event with their roles and
namespaces, every per-kind required-detail refusal and the cause-admission
refusal, a subject filter that matches either end and never returns one event
twice, the query bounds and digest binding, and the granted port that re-checks
owner and epoch and refuses a caller naming storage directly.

Each guard is falsified, not merely asserted: flipping the lineage-depth check,
the causal-parent arm refusal, the `..` traversal refusal, the live-instance
subject rule, the duplicate-role refusal, the required-actor refusal, the
unexpected-target refusal, the alias-collision refusal, the quantity-value
requirement, the content-reference requirement, the cause-admission rule, the
gap-detail refusal, the conflict-on-different-content rule, the retention
redaction, the per-read epoch re-check, the declared-gap refusal and the
undeclared-jump refusal each fails exactly its named test and nothing else, and
the file is restored byte-identical afterwards.

`crates/harness/tests/semantic_history_restart.rs`,
`semantic_history_restart_records.rs`, `semantic_history_restart_document.rs`
and `semantic_history_restart_lineage.rs` cover decision 7 the same way. The
restored history serves the same records in the same order, still explains a
stated chain and answers the same bounded query; a re-appended record replays
without writing a second event while a changed rejoin is a conflict; an epoch
advance that restarts host sequencing survives the restart on both a branch that
has already recorded in the new epoch and one that has not; a declared gap
travels with the history and a fork keeps its own records rather than inheriting
its parent's. Each refusal is falsified by mutation as well: skipping the
document's schema check, the owner-scope or window re-validation, the branch
identity check, the record's own schema and branch check, the per-record input
validation, the imported-parent refusal, the epoch, before-capture and coverage
checks, the duplicate-identity check, the sequencing and declared-jump rules,
the stated-parent presence, precedence and epoch rules, the digest comparison,
the end-of-branch sequencing expectation, and the lineage identity, branch
existence, epoch, single-root, bijection and depth rules fails exactly its named
test and nothing else, and the file is restored byte-identical afterwards.

`crates/harness/tests/semantic_history_import.rs` and
`semantic_history_import_boundary.rs` cover decision 8, and
`semantic_history_readonly_reads.rs` covers what a granted port can then walk
beside the admission rules in `semantic_history_readonly.rs`. A saved batch is
appended, served as imported history and still replayed rather than duplicated
after a restart; a gap it carries stays disclosed and an imported event inside a
declared gap, before capture began, or with coverage contradicting the window is
refused; a batch naming another scope, epoch or branch, another schema, an
unknown member, a non-opaque identity, an undecodable document or more events
than the bound is refused, and one that fails partway leaves the store at its
previous length. The granted reader follows a first page's continuation to the
end of a second page, explains a stated chain, refuses an unknown event or a
branch identity that is not opaque before a source is asked, and walks no
further than the caller's own traversal bound. The doubles these suites drive
are shared from `tests/support/semantic_history_import_doubles.rs` and
`tests/support/semantic_history_port_doubles.rs`.

Each of those guards is falsified by mutation as well: skipping the batch or
event member shape, the schema, the byte, batch-count or event-count bounds, the
batch, capture, branch or event identity checks, the owner-scope or epoch check,
the branch-existence check, the imported-origin refusal, or the append path, and
replacing the all-or-nothing copy with an in-place write or dropping the commit
each fails exactly its named test and nothing else. So does dropping the
continuation hand-off through the port, the page or explanation response arm,
the caller's own traversal bound, or the validation that runs before a source is
asked, with the file restored byte-identical afterwards.

These tests do not execute the game-mod producer, capture a native run, or answer
an end-to-end query over a controlled run. Those remain separate gates, and
`sts2-harness#128` stays open until they are met.
