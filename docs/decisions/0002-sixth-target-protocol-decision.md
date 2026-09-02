# ADR 0002: Acceptance of `sts2-protocol` as the Sixth Implementation Target

## Status

Accepted. `sts2-protocol` is the sixth implementation target in the current STS2 architecture.
Its implementation remains subject to the foundation, licensing, and conformance gates described
below.

## Context

The STS2 system has five independently owned runtime/application repositories: game core, game mod,
gateway, MCP server, and harness. Their boundaries need a small set of stable facts that can be
consumed by more than one repository without importing implementation types or coupling release
cycles. Those facts include identity and lineage metadata, version/profile descriptors, selected
lifecycle/timing metadata, and boundary error-envelope metadata.

A protocol target is useful only if it remains a contract artifact owner. A generic `common` crate,
host-aware transport library, or collection of duplicated domain structs would blur authority and
make compatibility drift harder to detect.

## Decision

Accept `sts2-protocol` as the sixth implementation target. It owns a narrow, language- and
transport-neutral contract package/schema set consisting of:

- namespace-qualified identity, correlation, and lineage metadata for `instance_id`, `session_id`,
  `run_id`, `episode_id`, `trajectory_id`, `request_id`, `action_id`, `trace_id`,
  `model_execution_id`, and `artifact_id`;
- independent version/profile descriptors, compatibility tuples, digest binding, and explicit
  mismatch outcomes;
- selected lifecycle, deadline, cancellation, sequence, and causality metadata that is shared at a
  boundary, without owning a repository's lifecycle state machine;
- boundary-neutral error-envelope metadata such as origin, code namespace, retryability,
  idempotency, correlation, and side-effect uncertainty, without owning a peer's error registry; and
- schema/package manifests, provenance fields, and implementation-neutral golden/conformance vectors.

The target must not own game semantics, legal actions, host objects, loader metadata, HTTP routes,
MCP framing/tools, gateway allocation or leases, provider requests/credentials, storage engines,
training behavior, or harness-specific scoring/trajectory semantics. It is not a runtime service and
does not become a generic-common implementation owner.

## Named consumers and ownership

Each consumer uses only the profiles mapped to its boundary. The protocol target owns the neutral
representation, version, and conformance oracle; consumers own semantic interpretation and mapping:

| Consumer | Protocol use | Retained local authority |
|---|---|---|
| `sts2-game-core` | shared metadata/envelope profiles needed at its boundary | domain meanings, game rules, and legality |
| `sts2-gateway` | instance/session/lifecycle/lease metadata profiles | process lifecycle, allocation, routing, leases, and fencing |
| `sts2-mcp-server` | session/request/error mapping profiles | MCP framing, tools, capabilities, and MCP authorization mapping |
| `sts2-harness` | run/episode/trajectory/model/artifact lineage profiles | coordination, provider ports, replay, scoring, datasets, and artifact semantics |

`sts2-game-mod` remains authoritative for host access, authoritative state, mutation, and game HTTP.
It is not a current protocol consumer. A later decision may accept a specific neutral artifact for
that boundary, but no such acceptance is implied by this ADR.

The protocol repository must publish a source-of-truth ledger for every field: owner, producer,
consumer, namespace, encoding, bounds, optional/null/unknown behavior, ordering, lifetime, secrecy,
version/profile, mapping, error behavior, security impact, and conformance case. A consumer may reject,
map, or forward a protocol field but may not silently redefine its normative meaning.

## Versioning and distribution

Protocol release version, schema/profile version, consumer release, game-host version, gateway API,
MCP revision, harness record version, scoring version, training/dataset version, and provider profile
remain independent. Protocol packages use a versioned profile identifier and immutable schema digest.
Every consumer pins the protocol revision/profile it consumes and records that revision, profile, and
digest in compatibility, run, trajectory, and artifact lineage records where relevant.

Changes are classified as additive-compatible, deprecated-compatible, safety correction, or breaking.
Additive changes define old-reader behavior and preserve unknown fields/enums where promised.
Deprecated changes provide a replacement and removal window. Breaking changes require a new protocol
major/profile path, migration documentation, coordinated consumer updates, and release evidence.
No consumer infers protocol compatibility from a successful parse, handshake, or acknowledgement.

The distribution artifact may include schemas and generated bindings for approved languages, but
generated bindings are derived outputs rather than normative implementation source. Generation records
must bind generator version, input schema digest, output digest, license, and consumer. Protocol has
no dependency on game, host, gateway, MCP, harness, provider, process, or storage implementation code.

## Conformance and drift gates

The protocol target owns implementation-neutral conformance cases and canonical fixtures. At minimum,
they cover:

1. exact field names, encoding, canonical ordering, numeric bounds, and missing/null/empty behavior;
2. unknown fields and enum values under each declared compatibility mode;
3. every identifier namespace, mapping, lifetime, restart, collision, and redaction rule;
4. independent version/profile mismatch, digest mismatch, and migration outcomes;
5. lifecycle/deadline/cancellation/sequence metadata and late or duplicate event handling; and
6. error origin/code mapping, retryability, idempotency, correlation, and side-effect uncertainty.

Each named consumer runs the applicable vectors against its decoder/encoder or mapping and reports a
visible pass, fail, or unverified result. The protocol target's CI checks schema/golden consistency,
sorted manifests, generated-output reproducibility, license/provenance, and consumer fixture drift.
Compilation, a shared dependency, or a successful handshake is not conformance evidence.

## Alternatives considered

1. **Keep every shared fact owner-local:** rejected because the named consumers need stable,
   implementation-neutral identity/version/envelope mappings and independent conformance vectors.
2. **Place a protocol crate inside the harness:** rejected because the named consumers are
   independent and the contract needs an owner and release identity separate from runs.
3. **Create a generic `common` crate:** rejected because it would accumulate domain, transport, host,
   and lifecycle authority without a bounded contract scope.
4. **Duplicate schemas in each consumer:** rejected because drift, version mismatch, and provenance
   could not be detected by one conformance owner.

## Consequences and review gate

The system now has six implementation targets but only five runtime/application authorities plus one
contract-artifact authority. The protocol target adds a release and compatibility surface, so its
first implementation must provide the ownership ledger, profile/version policy, canonical fixtures,
consumer matrix, provenance, and drift checks before downstream consumers claim support.

This ADR must be revisited if the shared scope expands into domain semantics, a transport or host
implementation, a generic utility layer, or a runtime service. Any such expansion requires a new
decision, an explicit owner transfer, dependency-graph review, migration plan, and updated conformance
cases.
