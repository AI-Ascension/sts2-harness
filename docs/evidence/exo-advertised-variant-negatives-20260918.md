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
| `extension_sha256` | `2e5485127f434bdd95a534785a414fa9f357432c924d89fd56d50c96f434b9cd` |
| `executor_sha256` | `244269e0ef7aeb92c71f6b47459f7fc9e41c1449552dd65847b226da6195ca54` |
| `bridge_sha256` | `5aa0d61436f041c40fb9090136772ab89a897fe1d9a462c94df2b25ec276ac00` |
| `oracle_sha256` | `16c3513cfdb479d1e3826eaa85a3633ba69cc04bbf3c047094153bbd4f751022` |
| `harness_revision` | `0003206c96289a71245b833778a4e0c467c5044e` (the revision the harness was at when the recorded run executed, as with the 2026-09-15 record) |
| Node | `v22.15.0` |
| Rust toolchain | `1.97.1`; non-Linux platforms remain unverified |

The `executor_sha256` here is a **local rebuild** and deliberately differs from the `c739ff69…`
recorded for the 2026-09-15 oracle; the earlier record is not extended or restated by this one.

## Advertised capability

`--describe` now publishes machine-checkable support sets instead of leaving a caller to infer
support from a rejection code:

```json
{
  "profiles": ["standard"],
  "profile_support": {"standard": "supported", "map": "unsupported", "expert": "unsupported"},
  "context_modes": ["fresh"],
  "decisions": ["action", "plan", "wait", "reobserve"],
  "decision_support": {"action": "supported", "plan": "supported", "wait": "supported",
                       "reobserve": "supported", "recovery": "unsupported"},
  "unsupported_profile_code": "exo_bridge_unsupported_profile",
  "unsupported_recovery_code": "exo_bridge_unsupported_recovery"
}
```

The advertisement and the guard are the same classifier, so they cannot drift. Both the one-shot
entry point and the lookup relay call `unsupported_profile_axis`; a test enumerates every axis the
classifier can return and asserts each has a negative case.

The lookup relay is terminal on an action id only, so it re-projects the decision fields rather
than inheriting the one-shot set: `--lookup-describe` advertises `decisions: ["action_id"]` and
marks `action`/`plan`/`wait`/`reobserve`/`recovery` `unsupported`. Without that re-projection the
relay would claim support for three decisions it cannot dispatch — the drift this increment exists
to stop, and the reason the capability sets are parameters rather than one shared constant.

## Result

The real-process oracle (`experiments/exo-agent/bridge/tests/advertised_variant_oracle.rs`) passes
with **zero model requests** across four probes: `describe`, a repeated `describe`, a
map-profile request refused pre-inference with `exo_bridge_unsupported_profile`, and a
tampered-config rejection. The synthetic model is armed with a decision that would succeed, so a
probe that wrongly inferred would visibly contact it rather than failing for an unrelated reason.

The request/receipt/decision negative matrix is enforced by two suites, both proven discriminating
by temporarily deleting the guard and observing the failure:

| Suite | Cases | Guard removed | Observed |
|---|---|---|---|
| `crates/harness/tests/exo_advertised_variant_negatives.rs` | 7 tests: profile axes, malformed framing, map refusal, decision parsing, lookup decision advertisement | `unsupported_profile_axis` body emptied | 2 tests fail |
| `crates/harness/src/bin/support/exo_bridge_run_tests.rs` | 8 tests: 13 negative receipts/decisions, advertisement agreement, route containment | `validate_decision` match arms deleted | `negative_receipts_and_decisions_never_produce_a_dispatchable_response` fails on `illegal_action_id` |

`crates/harness/tests/support/exo_contract_process_evidence.rs` re-derives `extension_sha256` and
`oracle_sha256` from the repository bytes and compares them with the committed JSON record and with
the manifest's `process_evidence.advertised_variant_evidence` block, so a stale record cannot
survive a change to the extension or to this oracle. The record is also listed in
`protocol-artifact/exo-bridge-v1/manifest.json` `pin_locations`, because it names the reviewed
candidate revision.

Full workspace validation on the final candidate:

```text
cargo run --locked --package repo-policy -- --strict   → 0 warnings, 0 errors
cargo fmt --all --check                                → clean
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings → clean
cargo test --workspace --all-targets --all-features --locked → 223 targets, 1567 passed, 0 failed
```

The workspace run requires `STS2_EXO_TEST_SOURCE` (a clean checkout of the reviewed revision) and
`STS2_EXO_TEST_NODE`; without them, five pre-existing `runtime_support` tests fail for missing
configuration, identically before and after this change.

## Remaining gap

Map and expert **positive** vectors are not delivered, because the owned extension implements
neither profile. Producing one would require extending the extension and the executor's
profile handling — a materially larger change than this acceptance increment. Until then the
honest state is an explicit `unsupported` advertisement plus a proven pre-inference rejection, and
the positive-vector clause stays open on #141.
