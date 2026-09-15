# Private benchmark manifest library

This library is an opt-in, effect-free foundation for #121. It validates declarations,
exports immutable private bytes and compares inputs. It does not allocate, launch,
provision a profile, generate a seed or rerun gameplay.

The source contract and synthetic input examples are in
[`benchmark_manifest`](../crates/harness/src/benchmark_manifest/mod.rs) and
the [test fixture builder](../crates/harness/tests/benchmark_manifest/fixtures.rs).
[ADR 0021](decisions/0021-benchmark-manifest-foundation.md) records authority,
canonicalization, compatibility and remaining integration gates.

The test fixture builder is original hand-authored MIT test data. Its production-canonical
configuration digest is `871d1751d9aef2ccc2a21629b7612b524af664892dcc7a9324259ff234943c76`,
experiment digest `cc1aff908b8d82c98c5338494b17a438a28a68e03d2e304686d932be4468f164`,
and artifact digest `b245f9f8f10c39851e12b38990077b1da4fab276289ade6cd6fa6f594d5371c8`.
These exact values are pinned in the canonical round-trip test. The unchanged seeded context
comes from the existing protocol golden with context digest
`d57563180f198b73970510427981504a9df10c62931577e601f9dcce6275fbe9`.

## Owner API

1. Call `Manifest::parse_private(bytes)` on a private `ascension.benchmark-manifest.v1`
   document. Missing/null required inputs, extra fields and duplicate object members
   are errors. Successful parsing means declared inputs are structurally valid.
2. Save `export_private()` bytes with an authorized private artifact adapter and retain
   `artifact_digest_private()`. This library does not perform or acknowledge persistence.
3. Parse a candidate and call `compare` before requesting allocation. Every returned
   `Mismatch` is a stable machine-readable category without private values. An empty
   result means exact declaration equality, not permission to mutate or native support.
4. Bind the original occurrence with `plan_trial`. Retain the original plan before any
   separately authorized native call. The occurrence uses distinct process/run/episode/
   instance/gateway-session/MCP-session/lease/operation fields and the original request fence.
5. Call `bind_seed_receipt` with a retained legacy seeded receipt. The result is a new
   `SeedReceiptBound` association only. Errors leave the original plan unchanged; identical
   canonical receipt bytes are idempotent, and changed bytes conflict with a bound record.
6. For public/model payloads use only `public_projection`, with an owner-held secret key.
   The resulting reference is HMAC-SHA-256 with a distinct benchmark domain. The projection
   contains no settings, seed, profile reference, input digest or raw occurrence identifier.

Private trial exports contain the manifest identities, occurrence and receipt digest.
They are not a shortcut for verification: reload the original manifest/occurrence and
recheck retained receipt bytes. A changed manifest is a new artifact, never an in-place edit.

## Bounds and semantics

| Input | v1 rule |
| --- | --- |
| Complete manifest, occurrence or receipt JSON | at most 65,536 bytes; strict existing parser limits depth to 32 and nodes to 100,000 |
| Identity/version tokens | 1..128 ASCII alphanumeric or `_.:/-` |
| Profile artifact reference | 1..128 ASCII alphanumeric, `_` or `-`, starting alphanumeric; never a path or URI |
| Requested/expected effective seed | 1..64 UTF-8 bytes, no control characters; no trimming/folding/derivation |
| Digests | exactly 64 lowercase hex characters; component revisions exactly 40 lowercase hex |
| Assembly inventory | 1..32 named digests; must be complete according to separately agreed coverage |
| Seeded context | existing pinned seeded-run-v1 schema and context digest; fresh standard Ironclad, ascension 0..20, 1..8 unique ordered acts, at most 32 sorted unique modifiers |
| Inference parameters | explicit map, at most 32 entries; names are identity tokens, values 1..128 bytes without controls |
| Provider/model revision | explicit `known` with bounded value, or `unavailable` |
| Inference seed | explicit `not_requested`, `unavailable`, or `requested` with value and `best_effort`/`unavailable` guarantee |
| Budgets | decisions 1..1,000,000; tokens 1..1,000,000,000; duration 1..604,800,000 ms |
| Occurrence fences/timestamp | nonnegative safe integers up to 9,007,199,254,740,991; entry ordinal 0..1023 |
| Receipt wrapper | closed recognized legacy fields; at most 64 reconcile exchange objects and 1,024 bytes of optional start-error text |
| Public projection key | 32..64 secret bytes, provisioned and retained by owner |

These are parser/admission ceilings, not new runtime budget defaults. Missing values never
mean unlimited; no values are clamped. Inference strings preserve declared provider lexical
values without interpreting provider parameters or promising deterministic sampling.

The expected effective seed and seed contract must already be supplied by the owner.
An unavailable native normalization/readback contract cannot be filled in by this parser.
Platform, package, coverage and configuration digests are declarations, not native attestations.
Settings/unlock payloads remain separately retained private inputs bound by their digests.
This library cannot reconstruct those payloads or prove that an assembly inventory is complete.

The accepted receipt has no harness run/episode/process or MCP-session fields. The owner must
enforce that an operation/fence belongs to one retained occurrence. All available wire fields
are compared, but illicit reuse of the entire same tuple under a different harness-only ID
cannot be detected from these receipt bytes. `SeedReceiptBound` does not prove authentic
native settlement, full profile/platform readback or complete hidden gameplay/RNG identity.
