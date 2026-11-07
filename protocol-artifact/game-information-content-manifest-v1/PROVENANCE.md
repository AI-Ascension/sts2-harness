# Harness import provenance

Source-derived data artifacts copied byte-for-byte from `sts2-protocol` commit
`1879e0b00993c9b6d662d1f015ef59d6810d82de` (MIT; see LICENSE).
The source paths are `artifacts/game-information-content-manifest-v1/`,
`schemas/game-information-content-manifest-v1.schema.json`, and
`conformance/{cases,fixtures}/game-information-content-manifest-v1`.

The upstream files are hand-authored synthetic contract/test inputs. No generation
or sanitization was performed during import; no game, personal, or provider data
is included. The schema is a normative pinned contract; goldens and conformance
vectors are test-only. SHA256SUMS is the unchanged upstream five-file inventory;
this provenance note and the preserved upstream license are owner-local additions.
The existing conformance case and all eight fixtures already matched the pin and
were retained unchanged.
