# Evidence: advertised variants and zero-model probes (2026-09-18)

Issue: AI-Ascension/sts2-harness#141, acceptance criterion 2. Contract: `sts2-exo-bridge-v1` /
ADR 0017 / ADR 0018.
Machine-readable record: [`exo-advertised-variant-negatives-20260918.json`](exo-advertised-variant-negatives-20260918.json).

Status: reproduced real-process evidence with a **synthetic loopback model and no game**. It is
*not* native-game evidence and *not* real-provider evidence. The previously recorded oracle in
[`exo-executor-process-oracle-20260915.md`](exo-executor-process-oracle-20260915.md) is unchanged
and is not claimed to cover this work.

## What this closes

AC2 has three sub-clauses. This increment addresses two of them and narrows the third:

| AC2 sub-clause | State after this increment |
|---|---|
| Every advertised decision variant and standard/map/expert request path has positive vectors; an ordinary-only capability rejects a map request before inference | **Partially satisfied.** All four advertised decisions (`action`, `plan`, `wait`, `reobserve`) have positive vectors. Map and expert remain **negative-only**: the extension implements neither profile, so there is no honest positive vector. The map request is refused before inference with a typed code, and the advertisement now says `unsupported` rather than omitting it. |
| Wrong identity/generation/revision, oversized/no-EOF input, unknown fields, invalid UTF-8, duplicate/multiple results, invalid action IDs, truncated output, process failure and refusal never dispatch a game action | **Satisfied** at the request, receipt and decision boundaries, with each case proven to fail if its guard is removed. |
| Non-inferencing probes make zero model requests; the smoke test cannot silently use a real configured external provider | **Satisfied.** Probes are re-run against the real process with a model that is armed to succeed, and the synthetic/provider routes are pure predicates with exhaustive negative cases. |

## Path under test

`sts2-exo-bridge` reads one strict `sts2.exo-bridge-wire-v1` request, verifies the pinned source
revision and packaged digests, and calls the separately built `sts2-exo-executor`, which runs the
pinned real Exo TypeScript runtime with the owned, tool-free extension. Model binding is `o3-pro`
against an original synthetic loopback server. No provider, credential, network egress, game, save
or native instance is used.

## Identities

| Field | Value |
|---|---|
| `exo_revision` | `b06869ab789dee3f80ca474b5fa89dbe47ccb859` |
| `extension_sha256` | `bcc034e787972f7ad6eabff5e817ad1456d6f6cab8c7dc42b426bd9f5b33ef3d` |
| `executor_sha256` | `35b214b58d3cd5fdcf250078b6dec1fcc24b6f0bc77b58fbfdb91e103062dc70` |
| `bridge_sha256` | `ee2207a1a0391796fc2cd5e8fbeb1c9c6763e792b849b89c406ea16cc030774f` |
| `oracle_sha256` | `c0da5819ba192b0a202950334a972eacc65c58ec128cbc4fc4433a23829e915d` |
| `harness_revision` | `fdb2d86050bf31c563083dc99e47602ac7b492bb` (the revision the harness was at when the recorded run executed, as with the 2026-09-15 record) |
| Node | `v22.15.0` |
| Rust toolchain | `1.97.1`; non-Linux platforms remain unverified |

The `executor_sha256` here is a **local rebuild** and deliberately differs from the `c739ff69…`
recorded for the 2026-09-15 oracle; the earlier record is not extended or restated by this one.

`harness_revision` names the commit at which the recorded sources were frozen. It is an ancestor of
or equal to the commit carrying this record and is rewritten when the change is squash-merged; the
binding that matters is that the named revision contains every recorded source byte, which the
oracle now enforces at record time (`support::assert_sources_are_committed`).

## Advertised capability

`--describe` now publishes machine-checkable support sets instead of leaving a caller to infer
support from a rejection code:

```json
{
  "profiles": ["standard"],
  "profile_support": {"standard": "supported", "map": "unsupported", "management": "unsupported",
                      "expert": "unsupported"},
  "context_modes": ["fresh"],
  "decisions": ["action", "plan", "wait", "reobserve"],
  "decision_support": {"action": "supported", "plan": "supported", "wait": "supported",
                       "reobserve": "supported", "recovery": "unsupported"},
  "unsupported_profile_code": "exo_bridge_unsupported_profile",
  "unsupported_recovery_code": "exo_bridge_unsupported_recovery"
}
```

The advertisement and the guard are one list, not two that agree. `unsupported_profile_axis` walks
`UnsupportedProfileAxis::ALL`, and `capability_fields` derives `profile_support` from that same
`ALL`, so the axis list *is* the guard and *is* the advertisement. The earlier `management` omission
was possible because the guard was a hand-written `if`-chain and the advertisement read a separate
constant; a new axis could be enforced without being listed. An enforced axis is now published in
the step that enforces it: adding a variant fails to compile until `is_present` and `profile_name`
handle it, and `ALL` is the only list either side reads.

One escape hatch remains by design, so it is closed explicitly rather than left implicit. An axis
whose `profile_name()` is `None` is enforced but has no `profile_support` entry. `Revision` is the
only axis that legitimately needs that (its expected value is already published as
`source_revision`), so `profile_name` derives the advertised key from `name()` and the `None` arm
names `Revision` alone; the test pins that exempt set to exactly `["revision"]`. A new axis cannot
opt out of the advertisement silently: it must join `Revision` in that arm, where a reviewer sees it.

`every_unsupported_profile_axis_is_rejected_before_inference` is driven by `ALL` and asserts each
case reports *that* axis; `every_classifier_profile_axis_is_advertised` checks `ALL` against the
published `UNSUPPORTED_PROFILES` in both directions, pins the advertised key set as literals, and
pins the exempt set, so neither shrinking `ALL` nor opting an axis out can widen the guard quietly.

The lookup relay is terminal on an action id only, so it re-projects the decision fields rather
than inheriting the one-shot set: `--lookup-describe` advertises `decisions: ["action_id"]` and
marks `action`/`plan`/`wait`/`reobserve`/`recovery` `unsupported`. Without that re-projection the
relay would claim support for three decisions it cannot dispatch — the drift this increment exists
to stop, and the reason the capability sets are parameters rather than one shared constant.

## Result

The real-process oracle (`experiments/exo-agent/bridge/tests/advertised_variant_oracle.rs`) passes
with **zero model requests** across five probes: `describe`, a repeated `describe`, map-profile and
management-profile requests each refused pre-inference with `exo_bridge_unsupported_profile`, and a
tampered-config rejection. The synthetic model is armed with a decision that would succeed, so a
probe that wrongly inferred would visibly contact it rather than failing for an unrelated reason.
Both refusal probes are schema-valid — the management probe carries `management_profile:
"management-enabled"` *and* the non-null `management_context` the schema requires for it — so the
strict parser admits them and the shared guard is the only thing that can refuse them.

The request/receipt/decision negative matrix is enforced by two suites, both proven discriminating
by temporarily deleting the guard and observing the failure:

| Suite | Cases | Guard removed | Observed |
|---|---|---|---|
| `crates/harness/tests/exo_advertised_variant_negatives.rs` | 8 tests: profile axes, two-way advertisement agreement, malformed framing, map refusal, decision parsing, lookup decision advertisement | `unsupported_profile_axis` body emptied | 2 tests fail |
| `crates/harness/src/bin/support/exo_bridge_run_tests.rs` | 8 tests: 13 negative receipts/decisions, advertisement agreement, route containment | `validate_decision` match arms deleted | `negative_receipts_and_decisions_never_produce_a_dispatchable_response` fails on `illegal_action_id` |

`crates/harness/tests/support/exo_contract_process_evidence.rs` re-derives `extension_sha256` and
`oracle_sha256` from the repository bytes and compares them with the committed JSON record and with
the manifest's `process_evidence.advertised_variant_evidence` block, so a stale record cannot
survive a change to the extension or to this oracle. The record is also listed in
`protocol-artifact/exo-bridge-v1/manifest.json` `pin_locations`, because it names the reviewed
candidate revision.

Like the 2026-09-17 oracle, this one refuses to emit a record whose `harness_revision` does not
describe the recorded bytes: `support::assert_sources_are_committed` fails the run when a recorded
source differs from `HEAD` or is untracked, so the named revision always carries the evidence.

Full workspace validation on `harness_revision` (the revision that carries these recorded bytes;
later commits carry their own totals):

```text
cargo run --locked --package repo-policy -- --strict   → 0 warnings, 0 errors
cargo fmt --all --check                                → clean
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings → clean
cargo test --workspace --all-targets --all-features --locked → 227 targets, 1644 passed, 0 failed
```

The workspace run requires `STS2_EXO_TEST_SOURCE` (a clean checkout of the reviewed revision) and
`STS2_EXO_TEST_NODE`; without them, six pre-existing tests fail for missing configuration — five
`runtime_support` bootstrap admission tests and `exo_lifecycle_runtime_entry` — identically before
and after this change.

## Remaining gap

Map and expert **positive** vectors are not delivered, because the owned extension implements
neither profile. Producing one would require extending the extension and the executor's
profile handling — a materially larger change than this acceptance increment. Until then the
honest state is an explicit `unsupported` advertisement plus a proven pre-inference rejection, and
the positive-vector clause stays open on #141.
