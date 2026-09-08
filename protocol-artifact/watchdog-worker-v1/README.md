# Watchdog worker handoff v1 consumer artifact

Owner: `AI-Ascension/ascension-watchdog`, MIT. The schema, manifest and eighteen
synthetic fixtures are unchanged copies of the owner's `worker-handoff-v1` bundle.
Schema publication commit: `b76547b279537cb47d3a7cefe9b6ce85eb7bca84`.
Imported candidate snapshot: `76daced` (not a claim of merged publication).
Schema SHA-256: `bb13d15f6c0e4b8d0f58f7391fe4ba319ebc57a0a09effc06d73ea718bbff4cf`.

Regenerate by copying the owner's complete schema, manifest and fixtures unchanged,
then comparing their exact bytes and rerunning the consumer conformance tests.
Fixtures are hand-authored, test-only synthetic data. They contain no host files,
provider output, credentials or private data. No implementation source is copied.

The harness owns its independent bounded decoder. The schema and manifest jointly
define validation: byte limits, canonical integer spelling and pairwise identity
separation are additional to structural JSON Schema validation.

The consumer-owned `SHA256SUMS` inventory pins all twenty copied JSON files.
Tests verify every digest, the complete inventory, manifest limits and operation,
and JSON Schema validity of all ten positive fixtures. The inventory itself is
pinned in the conformance test; this file is consumer documentation, not copied
upstream metadata.

Current consumer coverage includes all five request decoders and response encoders.
Terminal encoding uses the owner's WHJ-T1 ordered fifteen-field acknowledgment
material. The existing dispatch terminal fixture hashes to
`a0db0fc348623db806d2d053d7724198acb5c5c79f5b3422bf75faf3f653c260`, distinct from
its result digest. This golden is checked by both owner-local implementations.

The authenticated endpoint, durable acknowledgment transition and actual runtime
execution remain separate integration work. Passing these tests does not prove
an executable worker or end-to-end completion acknowledgment.
