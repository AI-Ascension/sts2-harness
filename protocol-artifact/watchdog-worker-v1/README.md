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

Current consumer coverage: request decoding only, including all five commands.
Response encoding, authenticated endpoint, durable admission and actual runtime
execution remain separate integration work. Passing these tests does not prove
an executable worker or end-to-end completion acknowledgment.
