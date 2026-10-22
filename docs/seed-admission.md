# Seed admission and legacy recovery

Harness applies the same input rules to a new Runtime-v3 seed plan and an authored explicit V2 seed:
the exact seed must contain 1 through 64 UTF-8 bytes, no control character, and no leading or trailing
Unicode whitespace. Interior whitespace is permitted. A multibyte character uses its encoded byte
length. Harness preserves accepted bytes and never trims, normalizes or replaces a requested seed.

All entries in `STS2_SEED_PLAN_JSON` must satisfy those rules before the plan can admit a new start.
The selected entry, ordinal and complete plan digest retain their existing meanings. The shared
validator is `seed_binding::canonicalize_seed`; the host still owns native canonical seed readback
and authoritative run-start evidence.

## Existing padded-seed reservations

Older Runtime-v3 inputs permitted surrounding whitespace. An existing operation may already have
sent its start, so changing its seed or operation identity would be unsafe. A plan whose only
canonicality failure is surrounding whitespace is accepted solely to reconcile its exact existing
`seeded-run-reservation-v1` record at `STS2_SEED_RESERVATION_PATH`.

Harness verifies the record during configuration and again before using it. Protocol, operation,
plan digest, ordinal, requested seed, mode, selected context and context digest must all match.
The record must carry an allowed lifecycle state, consistent receipt presence and a safe generation.
A valid record retains the original generation and returns the existing `resumed` classification.

This path cannot create a reservation. A missing or conflicting record fails, including when it is
removed or replaced after configuration has been admitted. The runtime's existing resumed branch
only reconciles that operation; it does not dispatch another `start_seeded_run`.

For new work, supply a canonical seed. Preserve a legacy operation's exact input and reservation
for recovery rather than editing its seed or inventing a new identity. Neither the persisted record
format nor the frozen seeded-run wire artifact changes.

## Verification and limits

The regression entry point is:

```bash
cargo test --locked --package sts2-harness --bin sts2-harness-runtime seed_transport::
```

Each environment-parser case runs in a separate child test process. Fixtures cover authored/runtime
agreement for ASCII and Unicode whitespace, interior spaces, controls and 64/65-byte multibyte seeds;
unchanged admitted bytes; original-generation legacy recovery; conflicting records; and deletion or
replacement after admission. They use synthetic plans and local disposable reservation files.

These checks establish configuration, admission and reservation behavior. They do not establish
native game acceptance, authentic setup-context production, provider behavior, deployment or the
complete workflow seed feature tracked in [issue #103](https://github.com/AI-Ascension/sts2-harness/issues/103).
