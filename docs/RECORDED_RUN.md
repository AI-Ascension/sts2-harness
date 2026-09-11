# Recorded-run export

This inspection exporter supports finalized directories marked
`seed-readiness-controller-release-v2`. It never runs a provider or a game.
The bundle preserves evidence from the recorder; process completion, provider
completion, action settlement, and episode outcome remain independent.

Build and export existing inputs:

```sh
cargo build --locked -p sts2-harness --bin sts2-recorded-run-export
target/debug/sts2-recorded-run-export export SOURCE_DIRECTORY --output NEW_BUNDLE.zip
```

The exporter currently emits proposed `1.0.0-candidate.3`, pinned to protocol
schema SHA-256 `a6c32127290f4d5e670d8863f97a74a7b8e3e411e735d81394b51fe1578b4eb6`.
Protocol artifact inventory SHA-256 is
`580c1cf3be4bb3e4eb37b9acd9166808b7386b0eb84286cc0798a0d88e35bb35`.
The coordinator owns admission and consumer integration. Candidate 2 delivery
copies and prior evidence remain immutable; generated delivery files under
`artifacts/recorded-run/` are ignored and must not be committed.

Optional configured provider/model labels are withheld as private source
metadata until a source-backed public-label allowlist is reviewed. Accounting
identities, status, counts, usage and admitted hashes remain available.

## Input boundary and evidence

Required inputs are manifest.json, result.json, trajectory.jsonl, decisions.jsonl,
and mcp.jsonl. provider-accounting.jsonl is optional. Other files, including
SQLite stores, logs, and directories, are not opened or required. Absence of
accounting is explicitly reconciled and omits the accounting ZIP member.

The root and admitted children are opened with descriptor no-follow semantics
on Linux. Each admitted file is a regular file, at most 16 MiB; the six inputs
share a 64-MiB budget enforced before allocation. Each source row is at most
1 MiB with at most 25,000 rows per stream. Reads are bounded by the initial
file size. Device/inode/timestamps and content hashes are compared after
projection and before publication. This detects changed inputs; it does not
lock out an adversarial writer. Completeness remains partial/unverified.
An output inside the input directory is refused.
Output records are capped at 64 KiB, manifest at 256 KiB, omissions at 1 MiB,
and the complete stored ZIP at 16 MiB. Oversized exports fail without publishing
a bundle. The current exporter is intended for bounded inspection exports;
it does not split larger recordings automatically.

Malformed final JSONL tails retain a valid prefix and one rejected physical
ordinal; interior blank or malformed rows fail closed. Duplicate keys and
excessive JSON depth/value counts are rejected. MCP raw rows are always
filtered. A malformed MCP tail currently fails closed because the pinned
contract requires every MCP row to be filtered, conflicting with the rejected
tail requirement. A protocol revision is needed to represent both.

Accounting statuses are checked against the closed source allowlist; unsupported
rows receive ordinal dispositions. Missing token fields remain absent, unknown
or not-applicable values remain null, and counts never become token usage.
Action receipts retain digested action identity and independent operation
identity; only source Unknown/null-effect receipts map to unknown outcomes.
Field omission counts explain withheld seed, observation, decision text,
provider identity, and error fields independently of row reconciliation.

JCS uses UTF-16 key order and pinned ryu-js ECMAScript binary64 formatting.
Source integer counters retain integer precision until projected to decimal
strings; structured privacy digests use binary64 JCS as required by RFC 8785.
Adapter source identity hashes the enumerated implementation sources and lockfile.
Producer is the release controller with unknown version unless owner evidence
supports a version. Reviewed metadata.heads entries preserve protocol, harness
runtime, and game-mod revisions independently. Scoped release-fix/native-package/
live-retry commits are not collapsed into a producer version. Candidate 3 has no
fields for all repository trees, binary hashes, or seed provenance; additional
portable provenance requires a protocol-owned extension.

## Future controller boundary

```sh
cargo build --locked -p sts2-harness --bin sts2-recorded-run-controller-finalize
target/debug/sts2-recorded-run-controller-finalize RUNS_ROOT BUNDLE_DIRECTORY EXPORTER -- CONTROLLER ARGS
```

Use an explicitly authorized future controller command. The wrapper drains
bounded stdout privately, requires exactly one `Controller running: ` marker,
waits for the entire controller process, then checks final result.json before
invoking `EXPORTER finalize SOURCE --output OUTPUT`. It preserves a failed
controller exit after successful export. It must not be invoked from the
harness child's exit: the inspected release controller writes result.json
only after its harness/watcher/gateway cleanup.

The persisted fake-controller tests close stdout before writing the final result
and exit sentinel. They cover nonzero controller exit, invalid/duplicate markers,
oversized stdout, missing result, export failure, and existing output.
They launch shell fakes only, never a game or provider.

## Verification

Seed receipt validation uses the pinned seeded-run-v1 source identity grammar
`^[A-Za-z0-9_.:/-]{1,128}$`, including slash in private compatibility identities.
It does not substitute the narrower portable recording identity grammar or the
public-label grammar. Compatibility text is never exported. Generation advance,
matching witness/observation, context, provenance and settlement checks still apply.
Raw-seed omission counts include observation.visible_seed as well as seed receipts;
each affected source row counts once per rule, alongside raw-observation omissions.

The exporter module is available on Unix targets, where descriptor-relative
no-follow snapshot reads are implemented. Other targets receive an explicit
unsupported-platform CLI error; no new runtime platform support is claimed.

```sh
STS2_RECORDED_RUN_VALIDATOR=/path/to/sts2-protocol/tools/recorded-run/validate.mjs \
  cargo test --locked -p sts2-harness --lib recorded_run
cargo test --locked -p sts2-harness --test recorded_run_controller
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
cargo run --locked --package repo-policy -- --strict
```
