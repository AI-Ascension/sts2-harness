# Redacted Jev decision capture

This opt-in bridge profile supplies metadata to the offline evaluator added in #369.
It is **one normal decision invocation plus a local sidecar**, not a shadow player,
paired provider runner, native witness, or evidence of better gameplay.

## Invocation and admission

The bridge accepts `--audit-dir DIR`. Runtime admission permits this suffix only after
the existing model/transport pair, optional gate, and optional `--tactical`, in that order.
The directory must already exist, be absolute and canonical without symlink components,
and have Unix mode `0700`. Output files are created with mode `0600`. The writer does not
create, chmod, clean, delete, or rotate the destination directory. Use an operator-approved
private location on a local filesystem; do not point it at a shared folder or network share.

The first profile is Unix-only. Windows capture is refused before provider egress; baseline
and tactical invocations without the new flag keep their existing behavior on Windows.
This restriction is intentional: Unix permission bits must not be mistaken for reviewed
Windows ACL enforcement. A separately reviewed ACL implementation is still needed.

For an already reviewed deployment, the argument shapes are:

```text
--model REVIEWED_MODEL --transport ABSOLUTE_TRANSPORT --gate 20 --audit-dir PRIVATE_DIRECTORY
--model REVIEWED_MODEL --transport ABSOLUTE_TRANSPORT --gate 20 --tactical --audit-dir PRIVATE_DIRECTORY
```

Pass the shape through `STS2_EXO_BRIDGE_ARGS_JSON` only after rebuilding, checking and
re-pinning the bridge through the existing admission procedure. No script in this change
edits installed binaries, starts a game, changes a save, or enables capture by default.
`--describe` is read-only and reports the capture schema, not the destination. `--record`
and `--audit-dir` are mutually exclusive. Runtime admission still rejects `--record` and
`--describe`; stdout stays exactly the existing bare decision object.

The opt-in authorizes local storage of the listed metadata. Review data classification,
retention and directory access before enabling it. It does not grant provider credentials,
authorize an additional call, widen game observations, or bypass the MCP/gateway boundary.

## Capture lifecycle and limits

The bridge validates the input and reserves `attempt-NNNN.pending.json` **before** invoking
its existing decision path. At most one transport invocation is allowed. Forced choices
may make zero calls. There is no retry, duplicate provider request, or second policy call.

A completed or failed attempt creates `attempt-NNNN.result.json` without replacing an
existing file. Pending records remain alongside final records. An interrupted reservation
has unknown attempts and elapsed time, not zero. Failed completed attempts contain a
bounded attempt count but no error text; failure-stage detail and failed-response usage
are deliberately unavailable. A crash while writing a final file can leave an invalid
partial file; the importer rejects it rather than treating it as complete.

The writer has 4,096 create-only reservation slots and a 16 KiB limit per record, at most
8,192 files and 128 MiB of record content per directory. Pending/failed slots count toward
the cap and are never reused. Exclusive creation arbitrates concurrent writers. The limit
is not an OS disk quota and does not include files written by other programs. Exceeding it,
an invalid directory, or storage failure refuses the decision rather than silently losing
capture evidence. Restarting with another directory requires an operator decision.

Files and directory entries are synced before returning. Power-loss guarantees remain
filesystem-dependent. This is not an OS sandbox against a privileged process or a hostile
owner concurrently replacing directory components, changing ACLs, or deleting reservations.
Keep the root and ancestors under trusted control. A digest of the located bridge binary
is not remote attestation of mapped executable pages or an independently authenticated record.

## What is retained

The closed schema records source-binary identity, requested/returned model fingerprints,
separate `model_execution_id` fingerprint, input/catalog fingerprints, selected profile and
gate, status, observed transport attempts, elapsed time, available input-token usage,
provider-request/question fingerprints, action **indices** and numerical tactical diagnostics.
The indices refer to the bridge's sorted complete host catalog; the catalog itself is not stored.

Raw state, objectives, hard-constraint text, prompts, provider-response bodies, rationales,
action names/IDs, descriptions, model names, credentials, local paths and error strings are
not copied to sidecars. No price or cost is guessed. Missing token usage stays null.
`elapsed_ms` measures the existing decision/record calculation, excluding binary hashing,
reservation and final-file writes. It is not end-to-end runtime latency.

Fingerprints use SHA-256 over a domain prefix and Rust `serde_json` bytes. They are content
identifiers, **not anonymization**: low-entropy content can be guessed and records can be
correlated. Treat sidecars as private evidence, not public telemetry. This reader's exported
report omits fingerprints and action indices as well as raw content.

The comparable input fingerprint removes only `model_execution_id`; all other fields remain.
The shared provider-request fingerprint removes only the exact tactical v1 wrapper and added
questions from the bridge-owned record. Different host state IDs, generations, objectives,
candidate catalogs or descriptions are not normalized away. Differences remain incomparable.

## Independent comparison

Run baseline and tactical separately against the same approved bridge input with distinct
`model_execution_id` values, through an authorized execution procedure. This exporter does
not recreate input from a digest, retain raw input for later replay, or schedule the other arm.
Live trajectories normally diverge; arbitrary records from two runs are not comparable just
because the seed or turn number matches. Use an approved in-memory paired invocation or
separately authorized original-input retention/replay to obtain genuinely matching decisions.

Prepare a private `pairs.json` manifest using the exact shape below. Replace the model with
its reviewed explicit pin and each fingerprint with the actual raw-file SHA-256. Common rolling
model aliases are refused. An explicit provider name is not proof of immutable model weights.

```json
{
  "schema": "ascension.jev-redacted-pairs.v1",
  "model": "jev-1.13.0",
  "bridge_digest": "REPLACE_WITH_64_LOWERCASE_HEX",
  "pairs": [{
    "pair_id": "pair-0001",
    "baseline": {"path": "baseline/attempt-0000.result.json", "sha256": "REPLACE_WITH_64_LOWERCASE_HEX"},
    "tactical": {"path": "tactical/attempt-0000.result.json", "sha256": "REPLACE_WITH_64_LOWERCASE_HEX"}
  }]
}
```

Use `null` for an unreported arm, or name its pending record to account explicitly for an
interrupted reservation. Do not supply both a pending and final file as separate executions.
The reader rejects reused execution fingerprints. Relative file paths, bounded reads,
UTF-8/JSON validation and raw-file hashing reuse the #369 reader. A declared missing or
unreadable file is an import error; it is not silently converted into an unreported arm.

```sh
node experiments/jev-evaluation/capture-cli.mjs /approved/private/pairs.json
```

Exit 0 means every pair reached a comparable action/action decision; exit 3 emits an
incomplete report with explicit noncomparable/refusal/missing categories; exit 2 reports an
invalid manifest, record or I/O failure without printing private values. None authorizes
policy promotion. Applied tactical records are checked against the v1 numeric selector,
but provider-request fingerprints remain producer assertions: the redacted reader cannot
re-run the full Rust response validator or reconstruct discarded provider data.

Agreement between independent invocations is separate from agreement with the tactical
request's own action question. Missing-evidence diagnostics are model estimates, not proof
that a particular host field is absent. Drift/failure diagnostics contribute to observed
capture totals but never to the comparable-action denominator. Attempt counts do not prove
that an HTTP request reached a provider; forced invocations have known zero attempts.
This capture format does not replace native settlement witnesses, cohort outcome accounting,
or the Rust benchmark-manifest and receipt validators.

## Validation

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
cargo run --locked --package repo-policy -- --strict
node --test experiments/jev-evaluation/*.test.mjs
```

The new [Jev evaluation workflow](../../.github/workflows/jev-evaluation.yml) runs every Node
test and the synthetic demo. Existing Rust CI includes the new projection, shared golden,
directory, quota, concurrency, failure, CLI and runtime-admission tests. The JSON golden in
`crates/harness/src/bin/support/jev_capture_golden.json` is an original MIT synthetic fixture,
generated from the literal input/body in `capture_projection_matches_the_shared_synthetic_golden`.
Regenerate it by serializing that test's `Identity::complete` output after reviewing changes;
both Rust and Node assert its contract. No game files or actual provider output are fixtures.

This change's package-level validation report states which checks actually ran. Source
tests, file hashes and synthetic records are not live provider, native-game, Windows ACL,
crash/power-loss or gameplay-improvement evidence.