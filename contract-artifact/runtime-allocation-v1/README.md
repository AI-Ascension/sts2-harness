# `runtime-allocation-v1` consumer fixture artifact

This directory is the harness-owned, offline copy of the gateway allocation recovery-authority
contract. The JSON schema, manifest, and two deterministic fixtures were copied byte-for-byte from
the gateway artifact at source revision `23aa476c3337875b879efa823eb023d18294c8f1` and reviewed at
`6ce8638b99d0cb45b7a6f80a27e17f2ddf1fca10`. The source artifact is
`sts2-gateway/contract-artifact/runtime-allocation-v1`; the gateway remains the authority for
allocation and lease semantics.

Provenance: origin is the checked-in gateway contract artifact above; input identity is the source
repository revision and the exact relative paths recorded in `SHA256SUMS`; generator is none (the
contract and sanitized fixtures are hand-authored deterministic JSON); license is MIT, inherited
from the repository contract artifact. `SHA256SUMS` binds every imported schema, manifest, and
fixture byte. The manifest's `schema_digest` must remain the SHA-256 digest of
`frame.schema.json`; the consumer regression test checks this relationship and the fixture digest.

The producer manifest permits a 256 KiB (`262144` byte) allocation frame, while this harness HTTP
consumer accepts at most 64 KiB (`65536` bytes) for one response body and JSON document. The
smaller consumer bound is intentional: this route carries a small allocation envelope, and the
harness rejects a larger response before semantic parsing. A producer response that needs more
space requires an explicit consumer-contract change rather than silently widening this bound.

These files are contract evidence only. They do not prove gateway, MCP, host, game, or live lease
compatibility.
