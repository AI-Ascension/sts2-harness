# Historical recovery consumer validation — 2026-09-07

Classification: **confirmed, local unit and synthetic subprocess evidence**. This is not an
integrated gateway/MCP/host release, live game recovery, service, reboot or soak result.

## Exact source

- Preserved base: `8f6bf7d030556ed42c28a88eb11fe5d59864ca50`.
- H27 validation integration: `eace7adafdb3676ab3d797d5b14e5c6141333d96`.
- Tested repair source: `431359d39fa05981f036861d7ee8d59643b47e41`.
- Root debug runtime SHA-256:
  `3bc746ded1417739ea3d91ca367e93822d0803c1dea50114f0809e7c2f500d6e`.

The debug binary digest identifies this build, not a portable/reproducible release. The source
commit preserves the stricter canonical-action envelope from the base rather than replacing it
with an older H27 branch. This evidence-only document does not alter tested production source.

## Regression evidence and implementation

Before the repair, a new regression showed that a RECONCILED operation with a SETTLED ticket but
no effect witness was accepted. The focused test exited 101 with that assertion failing.
An inherited positive fixture also used invalid canonical bytes `{}`; it was corrected to a
valid frozen legal-action envelope without weakening production validation.

Gateway producer source at `773da0051d0d815fee814d639bcc937494866c84` uses standard unpadded
base64 and preserves already terminal states when reconciliation returns a different terminal
representation. This is **source-derived** producer evidence, not execution of those binaries.
The harness now uses one bounded decoder, checks original operation context and canonical bytes,
requires bound terminal ticket/witness evidence, reconciles unresolved historical lookups, and
does not fall back to a gameplay poll. No frozen protocol/schema bytes change.

Synthetic stdio tests execute the harness recovery adapter and an owned script subprocess.
They record every tool call and cover seven successful lookup/reconcile state combinations,
three missing/unresolved combinations, and six missing/substituted witness cases. They require
the same operation reference, only historical lookup/reconcile tools, and retained durable
uncertainty on failure. Unknown and missing synthetic replies also use MCP `isError: true`.
These tests use an in-memory harness store; they do not simulate disk loss or process death.

Unit checks additionally cover ticket/authority/digest substitution, top-level versus nested
witness disagreement, invalid tail bits/padding, both permitted base64 alphabets, strict calendar
timestamps and wire integers, recursive duplicate keys, a 262144-byte frame bound, and redaction
of malformed untrusted fields/values.

## Root validation

Using pinned Rust 1.97.1 and an isolated build directory:

| Command | Result |
| --- | --- |
| `cargo fmt --all --check` | exit 0 |
| `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | exit 0 |
| `cargo test --workspace --all-targets --all-features --locked --quiet` | exit 0; 248 tests, including 97 runtime tests |
| `cargo build --workspace --all-targets --all-features --locked` | exit 0 |
| `cargo run --locked --package repo-policy -- --strict` | exit 0; 421 sized files before this evidence document, no warnings/errors |
| `sha256sum --check SHA256SUMS` in each existing copied artifact root | exit 0 for POC, Runtime-v1, Runtime-v2 and Runtime-v3 gameplay |
| `git diff --check` | exit 0 |

The complete 248-test workspace suite was rerun after commit 431359d with exit 0.
Strict policy was also rerun with this evidence document: 422 sized files, no warnings/errors.

An initial Clippy run rejected loading the shared decoder as two modules. Wiring now exposes
one wire-owned decoder to both callers; no lint suppression was added. An attempted checksum
check in a nonexistent co-op artifact directory did not run; this harness checkout has only the
four copied artifact inventories listed above.

## Remaining gates

Independent review of this exact source is pending. Original authority still comes from the
legacy recovery environment: immutable per-operation original context and fresh allocation
authority remain separate unfinished integration work. The current candidate must not be used
as proof of cross-boot recovery. The MCP-generated inner correlation remains an independent
boundary check, not an independently observed harness request correlation in these tests.

No changes from this candidate were pushed, merged, installed or activated for this record.
No game/provider was launched, no host was rebooted, and no live/reboot/soak result is claimed.
