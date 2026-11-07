# Content-manifest input provenance

The versioned contract consumed by [ADR 0093](decisions/0093-content-manifest-agent-delivery.md)
comes from `AI-Ascension/sts2-protocol` commit
`1879e0b00993c9b6d662d1f015ef59d6810d82de`, under the MIT license.
It is a public, inert schema and hand-authored synthetic fixture set, not game assets or copied
implementation. No host installation, private path, credential, user identity or provider output
was used to create the Harness fixtures.

## Exact upstream bytes

| Harness path | Upstream path at the pin | Role |
| --- | --- | --- |
| `protocol-artifact/game-information-content-manifest-v1/` | Same path | Versioned artifact, README, manifest, checksum inventory and two golden responses |
| `schemas/game-information-content-manifest-v1.schema.json` | Same path | Exact schema mirror for conformance |
| `conformance/cases/game-information-content-manifest-v1.json` | Same path | Exact upstream conformance inventory |
| `conformance/fixtures/game-information-content-manifest-v1/` | Same path | Two valid and six invalid original Protocol vectors |

The schema's SHA-256 is
`416a39769445e6e462c5d5b5504f29010c255e2116a73094e55c7268e47f2ba6`.
The copied case inventory SHA-256 is
`c9f24a504e892eb509a07a90d33a77d287b14809ae9d2eedca22d402b86bf2ed`.
The artifact's `SHA256SUMS` and case inventory retain their exact upstream digests.
`game_information_manifest_conformance_tests.rs` verifies the artifact files, schema mirror,
case inventory, every valid/invalid vector, and public codec acceptance or rejection.
The normal CI artifact inventory also discovers this profile automatically.

To reproduce the copy, check out the exact public Protocol pin and copy only these inert paths
without transformation. Validate with `sha256sum -c SHA256SUMS` from its artifact directory and
the focused Harness conformance test. The historical upstream README/case wording remains
verbatim even where later Gateway/MCP source now exists; ADR 0093 records current consumer facts.

## Original Harness test inputs

`conformance/fixtures/exo-lookup-content-manifest-v1/` contains five hand-authored, MIT-licensed
synthetic frames. The two valid frames request a manifest and the inclusive maximum read offset;
three negatives exercise an old wire pin, an extra model-selected scope and the history pin.
The reproducible inputs are the frame/profile/selector bounds in ADR 0093. No generator or
external native input is involved. Both in-org wire parsers consume the same files in
`crates/harness/tests/content_manifest_wire.rs`.

The Rust tests derive larger catalogs from the copied canonical response by changing only
correlation, synthetic definition identities and family counts. They serialize with the locked
serde_json version, optionally pad to the exact route byte ceiling, and compare full bytes and
SHA-256 after retention/replay. Negative cases mutate one named field or boundary. Embedded
Python strings are original disposable subprocess peers run by tests, not imported source or
a supported provider. Test-created SQLite stores and peers are temporary and contain no secrets.

Retain these small public fixtures with the source tests. Runtime content uses the existing
approved policy, encrypted memory store and caller-selected retention window; no real runtime
content is committed or included in this evidence.
