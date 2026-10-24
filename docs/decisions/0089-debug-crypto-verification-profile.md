# ADR 0089: Optimize Debug Hashing While Retaining Admission Verification

## Status

Source candidate; root review and compiled-runtime qualification are pending.

## Context

`exo_bridge_configuration::load_profile` reads and hashes the configured executor, extension, and
Node binary under their existing byte caps, then checks the source checkout's exact `HEAD` and
tracked clean status before it returns a loaded profile. The Runtime-v3 lifecycle entry calls this
loader before lifecycle admission. These checks bind the current files and source revision on each
load; caching or skipping them would change the admission contract.

Root's standalone Linux cost probe used the unchanged 123,438,592-byte `/usr/local/bin/node` input
with SHA-256 `b2959781cc5a74c357ffa02367efa8a0330cbb1c9cb347732fdfaaaca381cbcd`. It used pinned
`sha2` 0.11.0 and verified the exact identity on both iterations of both profiles. Baseline debug
dependency hashing took 6,805,319 and 6,263,459 microseconds; the isolated `sha2` opt-level 3
profile took 70,192 and 71,833 microseconds. The corresponding read times were 431,628/402,388 and
202,398/197,549 microseconds. Debug assertions and overflow checks were enabled in both profiles.
The probe was separate from the harness and is cost evidence only. Its read times and overall
effect depend on platform, filesystem cache state, compiler, and machine.

Separate root instrumentation measured the two Node verification phases together at 13,787 ms of
14,271 ms before allocation against the existing 15-second first-acceptance and 30-second child
budgets. That report is instrumentation only: it did not implement a product change or establish
that every historical delay had this cause. Its recorded `profile.rs` and lifecycle source hashes
differ from the files at this task's allocated base, so the timing is context for that run rather
than a source-pinned performance result for this candidate.

## Decision

Apply `opt-level = 3` only to the pinned `sha2` package in Cargo's `dev` and `test` profiles. Keep
`debug-assertions = true` and `overflow-checks = true` explicit in both profile overrides. Leave the
harness/coordinator/test crates, release profile, target CPU, backend, dependency versions/features,
and verification calls unchanged. Cargo selects `dev` for normal build/run commands and `test` for
tests; the explicit test override avoids relying on inheritance for this package setting. The
[Cargo profile reference](https://doc.rust-lang.org/cargo/reference/profiles.html) documents named
package overrides, the test profile's dev inheritance, and the package-override setting limits.

## Compatibility and evidence

No public API, schema, protocol, dependency, feature, or lockfile changes are introduced. Debug and
test `sha2` code generation changes, while release settings remain as before. Every admission retains
the existing bounded reads, exact digests, source revision check, and typed refusal behavior. This
change does not authenticate owner provenance or supply native, provider, or game acceptance.

The standalone measurements do not prove a 15-second result across machines or a shipped-runtime
improvement. Root must run the compiled shipped-runtime lifecycle test, the source/artifact identity
and bounds regressions, strict policy, formatting, Clippy, workspace tests, and documentation gates
on the final candidate. Those checks are not reported as run by this source change.
