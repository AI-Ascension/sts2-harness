# Runtime-v3 gameplay consumer contract copy

Normative owner: `AI-Ascension/sts2-protocol`, commit
`82507361890c1bdce6cffeaf7e616d93e53a7d99` (MIT).
Source: [upstream artifact](https://github.com/AI-Ascension/sts2-protocol/tree/82507361890c1bdce6cffeaf7e616d93e53a7d99/artifacts/runtime-v3-gameplay).
Schema SHA-256: `b37c80f583aeaf4f81ede2083bcfb4129196baf5eb092470e8738173c4b7226c`.

This is a consumer-scoped copy, not an unchanged full upstream repository/package layout.
`schema.json`, `manifest.json`, and all four `golden/` files retain exact upstream bytes.
`conformance.json` retains exact upstream `conformance/cases/runtime-v3-gameplay.json` bytes.
`UPSTREAM_SHA256SUMS` retains the authoritative checksum inventory verbatim. Its two
parent-relative paths refer to the upstream layout, not paths to open in this consumer checkout.
The test maps the source schema to its identical packaged `schema.json` and the conformance
case to `conformance.json`; remaining entries retain their package-relative paths.
Upstream manifest paths similarly describe the upstream package, not a local release claim.

Regenerate by copying these exact files from an explicitly reviewed protocol commit, updating
the production digest pin, provenance, and the checksum-inventory digest in
`runtime_v3_contract_test.rs`. Never regenerate expected hashes from an unreviewed consumer copy.
The golden generator is upstream `hand-authored`; inputs are sanitized synthetic identities,
observations and actions. No host files, provider results, credentials, or private data are present.

Workspace tests verify every upstream inventory entry and exact schema-pin agreement, validate
all four canonical goldens with offline JSON Schema, and pass the two response goldens through
the actual harness observation/receipt parsers. Negative mutations exercise those same parsers.
Request goldens receive schema validation only: the harness does not consume request frames.
This is bounded source-level consumer evidence, not all-vector, transport, host, Exo, or live
gameplay compatibility. The upstream conformance assertions are requirements, not proof that
all consumers or runtime semantics satisfy them.
