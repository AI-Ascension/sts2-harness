# ADR 0078: Ask the reward and its card in one System One call

Status: accepted for the System One bridge production path in `crates/harness`. It adds a second
question to a request that already carried one, in the states where the host disclosed a pending
card choice. It does not authorize a provider, native-host, game or deployment lane, and it claims
no run has exercised this shape against the real endpoint.

## Context

A reward screen is two decisions the host splits across two screens: which reward, then which
card. `build_system_one_request` sent exactly one question, and its module documentation gave the
reason and the cost:

> *"Exactly one question is asked. The provider evaluates many questions per call in parallel, so
> speculative fan-out is close to free in latency, and it is the natural place to put a threat
> reading or a risk gate later. Nothing here consumes such an answer yet, and an unconsumed
> question would spend tokens to produce a number no code reads."*

Asked separately, the first decision is made blind. The model commits to opening a card reward
without knowing what is in it, then cannot rank the cards and skips, and because a skipped reward is
re-offered the screen cycles. That cycle is recorded in `sts2-game-mod#171`.

The question this record answers is therefore not "can the transport carry two questions" — it
could — but "what is the first real consumer of a second question". The answer is the card choice,
and it needed a host that discloses what a reward holds.

That prerequisite has since landed and the earlier blockers are closed: `sts2-game-core#20`
(modelling decision, PR #24), `sts2-protocol#72` (wire contract, PR #74), `sts2-game-mod#255`
(the producer, `LiveCombatSource.RewardDisclosure.cs`), and `sts2-protocol#76` (relics, potions and
card descriptions, PR #80). `sts2-harness#314` already admitted `contents` on an offered entry.
The `card_choice` question is the consumer this repository was waiting for.

## Decision

### The transport carries several questions; the single-question builder is one caller of it

`build_choice_questions_request` owns the question set and the byte ceiling. Every question is
admitted only with a name, an option set and an instruction bound together in `SystemOneQuestion`,
so a caller cannot pair a name with the wrong options. `build_described_system_one_request` is now
one question passed to that builder, which is why the two cannot disagree about what a legal request
is.

The state plus the **longest single question** is still one budget, unchanged: asking two questions
does not make either of them larger, and a budget over the whole set would refuse requests this
contract admits for no safety gain. The state is still never truncated to fit.

### The whole request carries a second, separate ceiling

That reasoning holds for the ceiling the state shares and stops there. It does not cover the other
limit ADR 0053 publishes — `64k` tokens per request, of which the `32k` above is the state plus the
longest question — and the question count is precisely the term that limit is inflated by. A caller
that fills the set towards `MAX_QUESTIONS` keeps every individual question inside the shared budget
and still sends a request past the per-request ceiling, which the provider would refuse with a `422`
this builder exists to pre-empt. Before the card question there was one question per request, so
the two limits agreed; making the map a set separated them and the check did not follow.

The request builder therefore enforces `MAX_REQUEST_BYTES` (`128 KiB`) on the serialized body it
returns, and refuses it as `RequestTooLarge`. Two properties of that placement are deliberate.

**It is a new constant, not a reuse of the shared one.** ADR 0053 states the per-request limit in
tokens, not bytes, so the translation is a choice. `MAX_REQUEST_BYTES` applies the same
two-bytes-per-token reading `MAX_STATE_AND_QUESTION_BYTES` already documents, which makes it
deliberately twice that constant. Reusing the smaller number would have encoded `32k` where the
provider publishes `64k`. The reading is the conservative one: if a token is three or four bytes the
real ceiling is nearer `192 KiB` or `256 KiB`, and this bound refuses requests the provider would have
accepted — the same cost, and the same reason, the shared bound already accepts.

**It is measured on the value that is returned.** The body is serialized with the compact
`serde_json` form the crate writes to disk and sends, so the bound is taken over exactly those bytes
rather than an estimate, and a question added to the returned value afterwards is a value this check
never admitted rather than a silent spend.

This is the first time this builder can refuse for the size of the request as a whole rather than
the size of its longest question. The refusal is a new variant rather than a reuse of `OverBudget`,
because `OverBudget` reports *state and question exceed the budget* and would name the wrong bound.
Nothing a consumer pins moves: `system_one_questions_digest` covers the `questions` object alone, and
the bound reads the body without rewriting it, so every previously admitted request is byte-identical
and every recorded digest still matches.

The two ceilings overlap, and the overlap is narrower than the arithmetic suggests. Three
maximum-size questions still fit inside the per-request ceiling at the state the shared ceiling
allows, so for a set that size the two refusals coincide and neither is independently reachable; four
is the smallest set where the per-request ceiling owns a band of states the shared bound would have
admitted. Live traffic is not near this: the two-question path costs two options per question
against the `2.5%`–`6.5%` of the budget ADR 0078 measured, so the bound refuses nothing it admits
today.

A question set is refused when it is empty, over `MAX_QUESTIONS` (8), carries a name that is empty,
oversized, non-printable or duplicated, or has an option set that would not stand alone. Two
questions under one name are refused because the second would be unanswerable, which is the
unconsumed question the original documentation warned about.

`system_one_questions_digest` is unchanged and still covers the whole `questions` object, so the
digest says exactly what was asked. A test pins that adding or removing the second question moves it.

### The card set is derived from what the host disclosed, and never invented

`disclosed_card_choice` reads one level of `contents` from the offered set the host listed for this
screen. Every option identifier is the entry's own `choice_id`. Nothing is defaulted: a cost the
host reports as negative is omitted rather than printed, and a disclosed card with no usable
identity is dropped rather than given a placeholder name, because an option the model cannot name is
an answer that cannot be resolved back to a card.

Two disclosed rewards yield no question. Unioning them would offer cards from a reward the action
answer has not chosen; picking the first or the largest would be a ranking this module has no
warrant for. The second question waits for a state where the host has made exactly one pending
choice knowable, which is the state in which it is answerable.

### The card answer is advisory, required, and recorded beside the action it qualifies

A card identity disclosed on a reward screen belongs to the **next** screen's catalog. It is not in
this state's legal-action set, so dispatching it would be an action the host never offered. The
answer is therefore read — checked against the disclosed set by the same containment check the action
answer gets, and recorded under `card_choice` beside the decision — and it never becomes an action.
This is what makes the question consumed rather than a spend that buys a number no code reads.

Advisory describes what the answer is *for*, not how strictly it is validated. Once the question has
been asked, its answer is required: a missing answer, a non-`choice` answer, an answer with no
choice, or a choice outside the disclosed set refuses the exchange. Recording such an answer as
`null` and letting the action stand would be the outcome this decision exists to avoid — a run that
believes it ranked the cards and records no ranking — and would also re-create the unconsumed
question the request contract warns against, since the call would be spent and its result dropped.
The refusal fails closed at the seam where the answer is read, so it costs the run its real decision
rather than silently proceeding on a half-answered exchange. No second provider call is spent to
repair it: the same rule refuses a re-ask that the two-stage path would treat as a legitimate retry,
because a card the host has not disclosed cannot be re-ranked.

The action answer stays authoritative and is still required. The card answer qualifies that action;
it never replaces it, and it is never dispatched directly.

### Profiles that own a reviewed question count keep their shape

The evaluation profile (`--tactical`) owns its own question-count contract (`1 + 7 * targets`) and
the capture profile (`--audit-dir`) permits at most one transport invocation. Both keep the single
question they were reviewed with. A second question in either would break a contract already
reviewed, for no gain in that profile.

The existing two-stage split ([ADR 0062](0062-two-stage-option-ask.md)) is unchanged: an
above-bound option set is still asked as a `kind` then an `action` question, and the card question
is not asked on that path.

## Consequences

- On a reward screen where the host disclosed what a card reward holds, the two decisions cost one
  exchange instead of two. Measured headroom on live episodes was 2.5%–6.5% of the 65536-byte
  budget, and 2–8 options per question, so both questions fit comfortably.
- A reward screen that discloses nothing is unchanged: one question, no card answer. That is the
  blind case this record does **not** claim to have solved.
- On a reward screen that *does* disclose, a provider that answers only the action question now
  fails the exchange instead of producing a record with `card_choice.advisory: null`. That is a
  deliberate behaviour change against the permissive shape an earlier draft of this work had, made
  to satisfy #318 T3 ("reject stale or missing answers without inventing an action or silently
  spending another call"). A provider that cannot answer both questions in one call is not yet
  qualified for a disclosed reward screen; that is the finding this exposes, not a bug to hide.
- `selection_mode` and the record schema are unchanged, so an existing reader of a bridge record
  ignores the new `card_choice` field.
- The shape is `source-derived`, not `confirmed`: ADR 0053 records that no run in this repository has
  called the System One endpoint. The first real run exercised against the endpoint, on a host that
  discloses a card reward, is what would retire that caveat.

Refs #318.
