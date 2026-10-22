# ADR 0093: complete content-manifest delivery and retained replay

Status: accepted for the Harness source and synthetic boundary-test lane.
Owner: sts2-harness. Related feature: [GameMod #83](https://github.com/AI-Ascension/sts2-game-mod/issues/83).

## Decision and upstream pins

Consume the separate `game-information-content-manifest-v1` profile from Protocol commit
`1879e0b00993c9b6d662d1f015ef59d6810d82de`, with schema SHA-256
`416a39769445e6e462c5d5b5504f29010c255e2116a73094e55c7268e47f2ba6`.
This supplies the public codec and exact inert artifact, not another repository's implementation.
The [artifact provenance](../CONTENT_MANIFEST_PROVENANCE.md) records all copied inputs.
The dependency integration requires the independently reviewed Harness jsonschema 0.59.0 update;
the Protocol codec retains its own 0.58.1 dependency. No schema, serde_json, sha2 or other
workspace dependency is upgraded as a side effect of this feature.

The producer route is already represented in GameMod source. Gateway PR #87 landed at
`2d7f758b8a64744b0b83a2598f1028267d73b3a9`; MCP PR #76 landed at
`c468e76192f0c9046bb82806c2b7c99cbd12b9ba`. Their fixed tool is
`sts2.game_information_content_manifest`, a read-only member of the existing
`game-information-query-v1-mcp` catalog. Harness sends only the four owner-selected
instance/session/lease arguments; the model supplies no content, locale, scope or host selector.
MCP continues through the gateway's bodyless content-manifest route. Harness adds no host access.

The runtime refreshes `tools/list` before each manifest request, validates the negotiated
composition and read-only descriptor, and requires the manifest capability. The complete supported
composition is 17 unique tool names; older closed catalogs keep their original validation.
An owner policy check surrounds the catalog/read operation and runs again before accepting the
reply. Revocation while the MCP call is blocked discards its response. No automatic requery occurs.

## Whole-source admission

The Protocol profile permits 16 MiB, but the selected Gateway/MCP route admits only 128 KiB.
Harness enforces that smaller response bound before decoding. An over-bound catalog is a typed
failure. It is never shortened and then labelled complete.

Schema, duplicate-key, profile/digest, correlation, success/refusal and MCP wrapper checks precede
retention. Successful manifests must match the selected owner's `content_manifest_id` through
`inventory_revision`, and match its locale exactly. Harness also verifies unique package IDs and
orders, unique family and definition identities, complete per-family counts, declared families,
and internally consistent known package versions. Unknown origins/versions and unhandled families
remain explicit values. Package orders need not be contiguous. No displayed name is an identity.

The semantic, localized-text, inventory and content-set revisions remain producer claims. Harness
does not recalculate them from unavailable host definitions, interpret them as game-state hashes,
or promote source conformance into native compatibility. Locale spelling is never normalized:
the old query profile still requires its existing BCP-47 syntax, while this profile admits its own
locale syntax. A combined session needs the same producer/owner locale acceptable to each selected
contract; incompatible spelling is a refusal, not an implicit alias.

`ascension.game-information-content-manifest-record.v1` records owner scope/profile/content/locale,
operation ID, transport correlation, exact raw SHA-256/length, ordered memory chunk references,
derived-view SHA-256 and sanitized failure. It deliberately carries no snapshot attestation.
There are at most 32 manifest records per session, with unique operation IDs. Query records and
their 256-record, 64-KiB-per-source limits remain separate and unchanged.

Admission stages the complete source in a cloned bounded `MemoryCorpus`, with at most two existing
64-KiB entries, and computes the complete view before replacing the caller's corpus. A capacity or
view-budget failure publishes no partial source. This uses temporary memory up to the existing
bounded corpus size; it adds no unbounded queue or store API. A valid typed producer refusal is
retained exact source data. Malformed, unavailable or rejected responses retain only the sanitized
failure record, without raw diagnostics, source hash or invented catalog bytes.

Small catalogs reach the agent as complete structured data. Larger catalogs yield a retained-source
reference with exact length and hash. Bounded reads expose exact raw bytes; chunks may split UTF-8
and must be reassembled before text decoding. The game information remains untrusted data in a
tool feedback channel. Legal actions and owner authority continue to come from the host.

## Additive agent and replay profiles

The explicit `sts2.exo-lookup-wire-v4-content-manifest` profile adds
`sts2_lookup_content_manifest` and `sts2_lookup_manifest_read` to the existing query, read,
bootstrap and history surface. It is selected by `STS2_EXO_LOOKUP_CONTENT_MANIFEST=1` and the
matching `--lookup-content-manifest-*` bridge mode. Existing selected profiles reject both new
turns. A bootstrap or history turn keeps its own older wire pin even inside the new profile;
feedback must echo the answered turn's pin. Ordinary query/read/decision frames retain v1.
Both in-org Rust wire consumers, both bridge relays, the executor's native-event allowlist and
the TypeScript extension are updated together. The integration test compiles the exact in-org
executor wire and selector sources without importing the external Exo implementation.

Manifest feedback uses `manifest_record_ordinal`, distinct from query `record_ordinal`.
Reads select only that ordinal and a byte offset. They cannot select another owner or artifact.
The Exo feedback cap remains the smaller of 7,000 bytes and the approved policy budget, with at
most 3,000 raw bytes per hex read. The generic loop still has at most 32 read/decision turns.
A 128-KiB catalog needs 44 maximum-size reads, so complete delivery is retained and addressable
but cannot be consumed entirely in one such loop. Exhausting the loop is an explicit bound
failure. No larger budget, implicit summary or partial successful catalog is invented.

Query-only archive exports remain the exact closed `ascension.game-information-archive.v1`.
Sessions with manifest records select `ascension.game-information-archive.v2-content-manifest`.
Its optional query member contains the entire unchanged v1 transcript. Manifest-only archives
carry no snapshot, and importing them clears any caller snapshot. Export publishes sources and
approved policy dependencies through the existing encrypted store, reloads them, and verifies all
records before returning an archive. Store writes are not one transaction; a failed write yields
no usable archive. Every sanitized failure has its own authenticated owner metadata entry, including
failures interleaved with successful sources or query records. This metadata can anchor a failure-only
corpus generation, but is never a catalog or producer source. Imported failures keep their original
metadata and retention window when exported again. The archive byte ceiling remains 262,144.

Replay requires the owner-selected scope/content/locale, approved policy, original clock window,
independently retained archive digest and exact encrypted sources. It checks chunk order, lengths,
hashes, closed profile semantics, operation order and derived views. Missing, expired, revoked or
changed retention fails closed. Manifest reads cannot precede their corresponding replay turn.
The replay API has no MCP callback and never discovers a replacement catalog from today's game.
Replay distinguishes an authenticated stored failure from a fresh integrity error: a recorded
`Scope`, `Divergence` or `MissingRetention` is delivered as the same agent error, only after validating
its exact retained metadata and current policy. An integrity error aborts without advancing the
manifest cursor. Failure records retain no source hashes, references or catalog bytes.
Reading an authenticated failed record produces the same `MissingRetention` tool feedback as live
execution because it has no catalog bytes. An offset beyond an authenticated successful source
produces the same `Bounds` feedback. Each read rechecks current policy and retention; a fresh
integrity failure remains fatal and is never relabelled as either deterministic tool response.

## Evidence and remaining qualification

Synthetic tests cover the pinned valid/invalid Protocol vectors, all closed refusal mappings,
the exact MCP argument and wrapper shapes, whole 128-KiB retention/reassembly, atomic capacity
failure, encrypted SQLite restart, combined v1/v2 archives and byte-identical replay feedback.
Failure regressions exercise live admitted callbacks followed by encrypted restart and exact Exo
feedback serialization, plus altered owner/record/source claims, missing metadata, current policy
revocation and metadata revocation/expiry. The corresponding tests fail before the replay repair and
pass with the authenticated outcome check. A request refused before callback admission creates no
failure record that replay can accept.
Read regressions also cover error-then-read feedback, successful-source offset refusals and policy,
owner or retention changes after the recorded manifest turn. These read cases fail before their
repair and pass after it, without changing the older query replay behavior.
Subprocess tests exercise the production Exo host loop and runtime MCP port, selected-profile
refusals, fresh/missing/stale capability catalogs, correct feedback pins, and owner revocation
before and during the actual MCP read. They use original inert synthetic peers and no game/provider.

The external Exo checkout is unavailable in this org-scoped task. The complete isolated executor,
upstream TypeScript loader/typecheck/lint and actual provider execution therefore remain unverified.
The changed owned wire/selector sources and extension mock tests provide bounded consumer evidence,
not a claim that the native external loader was exercised. Hosted checks and independent full-diff
review are required before merge; local gate outcomes belong in the PR evidence.

GameMod #83 stays open for its original native acceptance. The remaining matrix is:

| Criterion | Harness source/test contribution | Evidence still needed |
| --- | --- | --- |
| Identical installations; locale affects text but not entity identity | Separate revisions preserved; exact content/locale binding checked | Two controlled installations and a locale-only change on the declared game build |
| Synthetic package add/remove/override changes revisions and expires queries | Changed owner content rejected; old sources remain pinned for replay | Actual package reload and producer cursor/cache invalidation receipts |
| Exact-host counts, duplicate names, unsupported families, no omissions | Counts/identities checked; unknown and unhandled rows preserved | Independent owner-registry comparison and complete actual producer → gateway → MCP → Harness delivery |

The producer, build/mode support matrix, native registry contents and reload semantics remain
GameMod-owned. This change closes the bounded Harness source gap without waiving those criteria.
