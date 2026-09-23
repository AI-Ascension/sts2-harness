# ADR 0064: A Jev runner teardown reports closure unconfirmed inside a host-load-dependent bound

Status: accepted for the `experiments/jev-evaluation` runner process contract. It records the owner
acceptance of the 1,000 ms teardown cleanup bound required by issue
[#394](https://github.com/AI-Ascension/sts2-harness/issues/394); the bound is already on `main`
(#396 `e1f5338bd8df72d6f6530ba8b2607fce62621006`) and the strict closure assertion is unchanged. It
does not authorize a provider, native-host or game lane. It is the campaign disposition owner's
decision, and it is ratified when the change carrying it merges.

## Context

`runner-process.mjs` decides `child_closed` from a race between two producers: the direct child's
`close` event, which calls `finish(..., true)`, and a fixed cleanup-grace timer armed by `stop()`,
which calls `finish(..., false)`. `runner-process.test.mjs:76` asserted `child_closed === true` for a
fixture whose descendant inherits stdio and outlives the deadline, so the assertion effectively
required process-group teardown plus event-loop delivery of `close` to complete inside the grace
window. Under load the timer wins, the flag is `false`, and the test reds; the suite is retried in CI,
so the red was maskable in the same way #388's was. The issue closed with the constraint that
weakening `:76` to a disjunction "would delete the pipe-leak signal the comment exists to protect", so
the repair had to be a decision about what `child_closed` must guarantee, not an assertion relaxation.

## Decision

The cleanup grace is **1,000 ms** (`runner-process.mjs:10-15`), sized above the kill-to-close tail
the issue measured under load rather than just above its median, and it is a **host-load-dependent
contract value**:

- `child_closed: true` means the direct child's `close` event fired — its pipes closed — within the
  bound.
- `child_closed: false` means closure was **not confirmed within the bound**; it does not mean cleanup
  failed.
- The assertion at `runner-process.test.mjs:76` stays a strict `=== true`; it is not relaxed to a
  disjunction.
- The test's real deadline-bound claim is a separate assertion and stays unaffected:
  `assert.ok(performance.now() - start < 1800)` at `runner-process.test.mjs:77` bounds the whole
  invocation to well under 1,800 ms, and the 1,000 ms bound leaves it true (worst case is the 300 ms
  deadline plus the 1,000 ms bound plus fixture overhead). The issue's third assertion was never the
  deadline bound; it was a bet that pipe-close delivery fits inside a fixed window, which is the bet
  this decision replaces with a documented, sized bound.
- The issue's escaped-descendant negative control became the regression test
  `an escaped descendant reports closure unconfirmed rather than a fabricated close`
  (`runner-process.test.mjs:80-104`): a descendant detached into its own session keeps its inherited
  pipes provably open past the bound, and the test asserts both `child_closed === false` and
  `elapsed_ms >= 1000`, so the flag cannot be fabricated in either direction.
- No later arm is launched when closure is unconfirmed.

`RUNNER.md`'s "Process and failure behavior" section records the bound and the flag's meaning. The
value is a contract, not a performance target: a host that reproducibly shows the kill-to-close tail
above 1,000 ms reopens this decision rather than silently widening the bound.

## Consequences

- The strict closure signal is preserved, and the flag's `false` branch is now covered by a
  deterministic regression test instead of only by a race.
- The bound is documented as host-load-dependent, so a later reader does not mistake
  `child_closed: false` for a cleanup failure.
- A `child_closed` flag establishes the direct child's `close` event and closed pipes, not that every
  escaped descendant has terminated. Descendants that deliberately create a new session still require
  an external reviewed OS sandbox; the flag is not a liveness proof.

## Sampling limitations retained

This acceptance does **not** claim the flake rate fell from the issue's 2-of-32 to zero. The lane's own
differential did not red the specific `:75` assertion in 60 pre-fix executions — consistent with the
issue's 0/12 and 0/40-isolated controls, i.e. it is the rarer member of the family — and only 3 of
those 60 landed in the mid-band load the issue records, so the mid-band rate is not spoken to. What
the differential does establish is mechanism-level: the 250 ms bound was load-bearing on real sibling
assertions in the same file (5 of 60 pre-fix executions red at load ≥ 81, three of them the same
`child_closed` signal at `runner-process.test.mjs:39`/`:67`), and the 1,000 ms bound produced 0 of 40
reds under the same host load. A 1-in-16 rate is not measurable at these sample sizes; the decision
rests on the measured kill-to-close tail (250-306 ms under load, ~565 ms in the escaped-session
control) and on keeping the assertion strict, not on a measured rate change.

## Evidence

| Claim | Label | Source |
| --- | --- | --- |
| The bound is on `main` and the assertion is unchanged | `confirmed` | #396 `e1f5338bd8df`, `runner-process.mjs:15`, `runner-process.test.mjs:76` |
| The 250 ms bound was load-bearing on sibling assertions | `confirmed` | pre-fix differential: 5/60 red at load ≥ 81 |
| The 1,000 ms bound removed them | `confirmed` | post-fix differential: 0/40 red, 297/297 per run |
| The rate dropped from 2/32 to 0 | `unsupported` | not measurable at these sample sizes |

Refs #394. Related: [ADR 0063](0063-jev-runner-first-arm-admission.md).
