# Coding Standards

## Goals

Code should make ownership, lifecycle, evidence, and data lineage easy to see. Compatibility and
privacy take precedence over cleverness. These rules apply to product code, ports, fixtures, tests,
repository tools, generators, and workflow logic.

## Toolchain and format

Use the pinned Rust `1.97.1` toolchain and edition 2024 from `rust-toolchain.toml` and the workspace
manifest. `rustfmt` with a maximum width of 100 is authoritative. Keep `Cargo.lock` current for the
workspace tools and do not depend on undeclared globally installed tools for ordinary checks.

Clippy warnings are errors in CI. Unsafe Rust is forbidden unless a separately approved boundary
requires it and documents pointer, ownership, lifetime, thread, and unload invariants. The harness
has no approved host/FFI boundary.

## Modularity and budgets

Split by responsibility: coordinator policy, record validation, provider port, MCP/gateway adapter,
storage/artifact port, replay, scoring, and test support must not become a catch-all module. Avoid
`common`, `utils`, `helpers`, `manager`, and `service` names when a domain name exists.

The policy budgets are:

| Artifact | Preferred | Hard |
|---|---:|---:|
| Production Rust | 300 nonblank lines | 400 |
| Rust tests | 400 nonblank lines | 600 |
| GitHub workflow | 160 nonblank lines | 200 |
| Markdown | 500 nonblank lines | 700 |

Functions should remain at or below 40 lines; refactor beyond 60. Do not compress or split files
artificially to evade a limit. Generated or reviewed snapshots require an exact-path exemption with
origin and regeneration information. Copied reference source is never exempt.

## Types, errors, and records

Use explicit newtypes and enums for IDs, lifecycle states, versions, deadlines, and evidence states.
Validate untrusted input at a boundary and pass validated values inward. Preserve namespace,
optionality, unknown-value, ordering, numeric-bound, and canonicalization rules in serialized records.

Use typed errors for invalid input, stale state, unavailable boundary, timeout, cancellation,
overload, provider failure, storage failure, divergence, and internal defects. Map errors once at a
boundary and never expose credentials, paths, panic text, or arbitrary payloads. Do not retry a
mutating or provider operation without explicit idempotency and duplicate-effect rules.

## Concurrency and external effects

Bound every queue and payload. Give every task an owner, cancellation path, timeout, and shutdown
join. Do not hold locks across an external port, await, callback, blocking I/O, or user code. Use
monotonic time for durations and injected clocks/schedulers in deterministic tests. Accepted work may
not disappear because a caller disconnects or a downstream operation times out.

External effects are behind declared ports. The harness may consume MCP/gateway/provider/artifact
interfaces; it may not access a game process or host object directly. Credential acquisition,
retention, redaction, export, and provider egress are explicit policy decisions.

## Documentation and provenance

Public items document behavior, errors, evidence level, and compatibility. A planned behavior is
labeled planned; an unverified behavior includes a safe validation procedure. Imported or generated
material records source, version/digest, license, generator, inputs, and retention permission.
Never copy, vendor, transliterate, or use reference implementation symbols as a product plan.
