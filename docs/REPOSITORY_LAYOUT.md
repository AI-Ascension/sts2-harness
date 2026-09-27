# Repository Layout

## Current status

This is the distinct `sts2-harness` target in the STS2 project workspace. Wave 2 retains the
responsibility directories and adds one real target-owned harness package plus its copied POC artifact
and report. It does not turn unrelated directories into implementation claims.

```text
.
├── crates/harness/       # target-owned coordinator ports, records, and deterministic tests
├── protocol-artifact/     # copied, release-like poc-v1 contract consumed by the harness
├── contract-artifact/     # per-capability consumer contract pins; some carry golden vectors and `SHA256SUMS`
├── contracts/            # reviewed wire pins shared with companion owners, plus effective-limit pins
├── fixtures/             # small original synthetic fixtures for the deterministic test suites
├── experiments/          # experiment definitions, local harnesses, and controlled-run guidance
├── schemas/              # harness-owned record/artifact contracts
├── conformance/          # implementation-neutral cases and mutations
├── docs/                 # architecture, policy, decisions, research, and testing guidance
│   ├── CHANGELOG-ARCHIVE.md           # verbatim completed changelog entries moved out of the active file
│   └── CHANGELOG-ARCHIVE-<date>.md    # one verbatim closed wave per archive pass
├── .github/              # required policy, CI, and lane workflows plus the pull-request template
├── tools/repo-policy/    # current Rust foundation checker
└── MINIMAL_POC_REPORT.md # exact offline trace and evidence classification
```

Deterministic component and integration tests live beside the code they cover, under
`crates/harness/tests/`; this repository has no top-level `tests/` directory. The
`conformance/` tree carries the implementation-neutral cases, and `fixtures/` carries the
synthetic corpora the suites consume.

The `experiments` directory is preserved. Each subdirectory is an explicit boundary experiment with
its own README, and none of them grants the harness game or host authority.

## Planned responsibility placement

| Responsibility | Initial home | Rule |
|---|---|---|
| Coordinator policy | a real module or crate under `crates/harness` | no transport, host, or provider implementation in core policy |
| MCP/gateway consumption | named adapter module | declared port; no direct game process access |
| Provider execution | named provider port/adapter | scoped credentials, budgets, cancellation, redaction |
| Episodes/trajectories | harness-owned records | independent IDs, versions, event ordering, provenance |
| Replay/scoring | separate cohesive modules | deterministic inputs and explicit divergence/evaluator versions |
| Artifacts/datasets | artifact/lineage module | hashes, manifests, retention, license, and consumer identity |
| Research specifications | `docs/research/` | self-contained, evidence-labeled architecture and evaluation research; generated package is a requirements baseline, not runtime proof |
| Tests/conformance | `crates/harness/tests/` and `conformance/` | fakes and bounded fixtures; no proprietary host files |

This table is not permission to create empty placeholder crates or duplicate another repository's
implementation. Any additional component needs a requirement, owner, dependency review, and tests.

## Dependency and runtime boundaries

Compile-time dependencies point from coordinator adapters toward stable record/port abstractions.
Runtime communication is:

```text
harness -> MCP server -> gateway -> game-mod/host
provider -> harness
harness -> artifact store
```

The game mod owns host authority; the gateway owns lifecycle and routing; MCP is a thin adapter; the
harness owns coordination and artifacts. `sts2-protocol` is the accepted sixth implementation target
for narrow shared contract artifacts and conformance; it is not a runtime service or generic-common
owner. The harness consumes its versioned profiles and retains authority over harness-specific records.
See the decision records under `docs/decisions/`.

## Generated and local material

Build output, editor state, credentials, host SDKs, private data, model artifacts, datasets, and
temporary reports are ignored or kept outside the repository. Generated/snapshot material is admitted
only with exact provenance, a license, a generator, and a policy exception where its size requires it.

## Naming authority

The aggregate NAMING_CONVENTIONS.md and naming-registry.yaml define shared
casing and identity rules. Harness-owned run, episode, trajectory, model, artifact, record, and
trace names must retain their separate semantic namespaces.
