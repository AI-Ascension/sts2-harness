# Contributing

Thank you for helping build the STS2 experiment and training harness. This project values explicit
ownership, reproducible artifacts, deterministic tests, and honest evidence over implementation speed.

## Start here

Read the documents relevant to your change:

- [`AGENTS.md`](AGENTS.md) for operational rules
- [`docs/PRODUCT.md`](docs/PRODUCT.md) for the harness contract and non-goals
- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for dependency and runtime boundaries
- [`docs/CODING_STANDARDS.md`](docs/CODING_STANDARDS.md) for Rust and size rules
- [`docs/TESTING.md`](docs/TESTING.md) for deterministic evidence
- [`docs/COMPATIBILITY.md`](docs/COMPATIBILITY.md) for independent version axes
- [`docs/LICENSING.md`](docs/LICENSING.md) for provenance and distribution
- [`docs/WORKFLOWS.md`](docs/WORKFLOWS.md) for branch, pull-request, and automation flow
- [`RELEASING.md`](RELEASING.md) for release procedure
- [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md) and [`SECURITY.md`](SECURITY.md) for safe reporting

The current phase is Wave 2 codebase initialization. The target-owned harness package may add pure
ports, coordinator seams, and deterministic fakes. Do not add provider SDK calls, model artifacts,
datasets, direct game access, gateway lease ownership, MCP framing, game rules, or copied/reference
source.

## Discuss first

Open a design discussion and update an ADR before changing:

- identifier, version, lifecycle, error, timing, trajectory, scoring, or artifact contracts;
- provider credential flow, external network reachability, retention, redaction, or privacy posture;
- gateway/MCP consumption, instance allocation, lease, fencing, retry, cancellation, or shutdown;
- crate boundaries, dependency direction, process boundaries, or the protocol-repository decision;
- a public CLI/API, serialized field, schema, evaluator, replay rule, or release artifact; or
- a new dependency or supported model/provider/platform.

Small documentation corrections and focused test improvements may proceed directly when they preserve
the accepted contract.

## Development workflow

1. Start from the current intended base and inspect the checkout before editing.
2. Keep one cohesive responsibility per change and preserve unrelated dirty files.
3. Write or update project-owned requirements and deterministic acceptance tests before product code.
4. Keep external effects behind declared ports with explicit timeouts, capacity, cancellation, and
   error mapping.
5. Update affected documentation, provenance records, and `CHANGELOG.md`.
6. Run policy, formatting, lint, test, and applicable conformance checks.
7. Describe evidence states and unverified runtime boundaries in the pull request.

Use `apply_patch` for file edits. Do not initialize Git or perform commit, push, merge, release,
deployment, installation, provider, game-launch, or game/profile mutation actions without explicit
authorization for that action.

## Contribution license

By submitting a contribution, you represent that you have the right to provide it and license it
under the repository's [MIT License](LICENSE). Identify copied, generated, or adapted material and
retain applicable notices. Do not submit game files, private data, credentials, model weights,
datasets, or code whose terms are unknown or incompatible. The project does not currently require a
separate contributor license agreement or Developer Certificate of Origin sign-off.

## Local checks

Run the policy command first:

```bash
cargo run --locked --package repo-policy -- --strict
```

For Rust changes, run the complete deterministic suite:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
```

The absence of a game, gateway, MCP server, provider, or external artifact store is not a reason to
claim runtime success. Use fakes and bounded fixtures, and report unavailable runtime lanes as
`unverified`.
