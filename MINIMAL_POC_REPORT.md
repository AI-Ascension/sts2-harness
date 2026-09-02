# Minimal POC report

Date: 2026-09-02
Status: `test-confirmed` fake/offline proof only.

## Scope and ownership

This report covers one deterministic proof-of-concept vertical slice. The protocol contract was built
and packaged first by `sts2-protocol`; this repository consumes its copied release-like artifact under
`protocol-artifact/poc-v1/`. The harness owns the runner, canonical trace, and evidence report. The
exact requested boundary order is:

```text
harness -> MCP -> gateway -> game-mod -> game-core
```

The slice uses local doubles only. It does not access a live process, game package or file, host
assembly, provider, credential, network, or external service. It makes no runtime, compatibility,
game-legality, or production-readiness claim.

## Artifact lineage

The copied artifact is verified before the runner starts.

| Field | Value |
|---|---|
| artifact | `sts2-protocol/poc-v1` |
| protocol version | `poc-v1` |
| schema source | `schemas/poc-v1.schema.json` |
| generator | `hand-authored` |
| schema digest | `242b8f9233e915a55ea8d2e72ca476c1258169a67e62de72ee5aed848a6a0a19` |
| artifact copy | `protocol-artifact/poc-v1/` |

The harness also copies the normative source schema and conformance case under
`protocol-artifact/schemas/` and `protocol-artifact/conformance/`. The verifier parses the manifest,
schema, all five golden messages, the invalid-action fixture, and the conformance case, then checks
all ten release checksum entries.

The lineage fields identify the consumed artifact; they are not authority to mutate a game.

## Deterministic inputs and expected semantics

The runner uses seed `7`, clock tick `0`, session `session-1`, instance `instance-1`, and lease
`lease-1`. The fake core starts at generation `0` with three available units and zero settled effects.

| Correlation | Operation | Request | Expected result |
|---|---|---|---|
| `corr-0001` | `get_state` | no action | generation `0`, units `3`, effects `0` |
| `corr-0002` | `submit_action` | `use_budget`, units `1` | accepted; generation `1`, units `2`, effects `1` |
| `corr-0003` | `submit_action` | `use_budget`, units `0` | rejected with `sts2.game-core/zero_units`; state remains unchanged |

## Observed trace

The runner emits one canonical JSON line per actual fake-hop crossing: 15 lines total. Each line is finalized from the enter/complete token for the corresponding call, with its ordered boundary, sequence, session, and typed response metadata.

```json
{"boundary":"harness","tool":"get_state","kind":"state_response","sequence":0,"protocol_version":"poc-v1","schema_digest":"242b8f9233e915a55ea8d2e72ca476c1258169a67e62de72ee5aed848a6a0a19","provenance":{"artifact":"sts2-protocol/poc-v1","source":"schemas/poc-v1.schema.json","generator":"hand-authored"},"correlation_id":"corr-0001","instance_id":"instance-1","session_id":"session-1","lease_id":"lease-1","generation":0,"observation":{"available_units":3,"settled_effects":0},"action":null,"status":null,"error_code":null}
{"boundary":"mcp","tool":"get_state","kind":"state_response","sequence":1,"protocol_version":"poc-v1","schema_digest":"242b8f9233e915a55ea8d2e72ca476c1258169a67e62de72ee5aed848a6a0a19","provenance":{"artifact":"sts2-protocol/poc-v1","source":"schemas/poc-v1.schema.json","generator":"hand-authored"},"correlation_id":"corr-0001","instance_id":"instance-1","session_id":"session-1","lease_id":"lease-1","generation":0,"observation":{"available_units":3,"settled_effects":0},"action":null,"status":null,"error_code":null}
{"boundary":"gateway","tool":"get_state","kind":"state_response","sequence":2,"protocol_version":"poc-v1","schema_digest":"242b8f9233e915a55ea8d2e72ca476c1258169a67e62de72ee5aed848a6a0a19","provenance":{"artifact":"sts2-protocol/poc-v1","source":"schemas/poc-v1.schema.json","generator":"hand-authored"},"correlation_id":"corr-0001","instance_id":"instance-1","session_id":"session-1","lease_id":"lease-1","generation":0,"observation":{"available_units":3,"settled_effects":0},"action":null,"status":null,"error_code":null}
{"boundary":"game-mod","tool":"get_state","kind":"state_response","sequence":3,"protocol_version":"poc-v1","schema_digest":"242b8f9233e915a55ea8d2e72ca476c1258169a67e62de72ee5aed848a6a0a19","provenance":{"artifact":"sts2-protocol/poc-v1","source":"schemas/poc-v1.schema.json","generator":"hand-authored"},"correlation_id":"corr-0001","instance_id":"instance-1","session_id":"session-1","lease_id":"lease-1","generation":0,"observation":{"available_units":3,"settled_effects":0},"action":null,"status":null,"error_code":null}
{"boundary":"game-core","tool":"get_state","kind":"state_response","sequence":4,"protocol_version":"poc-v1","schema_digest":"242b8f9233e915a55ea8d2e72ca476c1258169a67e62de72ee5aed848a6a0a19","provenance":{"artifact":"sts2-protocol/poc-v1","source":"schemas/poc-v1.schema.json","generator":"hand-authored"},"correlation_id":"corr-0001","instance_id":"instance-1","session_id":"session-1","lease_id":"lease-1","generation":0,"observation":{"available_units":3,"settled_effects":0},"action":null,"status":null,"error_code":null}
{"boundary":"harness","tool":"submit_action","kind":"action_response","sequence":5,"protocol_version":"poc-v1","schema_digest":"242b8f9233e915a55ea8d2e72ca476c1258169a67e62de72ee5aed848a6a0a19","provenance":{"artifact":"sts2-protocol/poc-v1","source":"schemas/poc-v1.schema.json","generator":"hand-authored"},"correlation_id":"corr-0002","instance_id":"instance-1","session_id":"session-1","lease_id":"lease-1","generation":1,"observation":{"available_units":2,"settled_effects":1},"action":{"action_id":"use_budget","units":1},"status":"accepted","error_code":null}
{"boundary":"mcp","tool":"submit_action","kind":"action_response","sequence":6,"protocol_version":"poc-v1","schema_digest":"242b8f9233e915a55ea8d2e72ca476c1258169a67e62de72ee5aed848a6a0a19","provenance":{"artifact":"sts2-protocol/poc-v1","source":"schemas/poc-v1.schema.json","generator":"hand-authored"},"correlation_id":"corr-0002","instance_id":"instance-1","session_id":"session-1","lease_id":"lease-1","generation":1,"observation":{"available_units":2,"settled_effects":1},"action":{"action_id":"use_budget","units":1},"status":"accepted","error_code":null}
{"boundary":"gateway","tool":"submit_action","kind":"action_response","sequence":7,"protocol_version":"poc-v1","schema_digest":"242b8f9233e915a55ea8d2e72ca476c1258169a67e62de72ee5aed848a6a0a19","provenance":{"artifact":"sts2-protocol/poc-v1","source":"schemas/poc-v1.schema.json","generator":"hand-authored"},"correlation_id":"corr-0002","instance_id":"instance-1","session_id":"session-1","lease_id":"lease-1","generation":1,"observation":{"available_units":2,"settled_effects":1},"action":{"action_id":"use_budget","units":1},"status":"accepted","error_code":null}
{"boundary":"game-mod","tool":"submit_action","kind":"action_response","sequence":8,"protocol_version":"poc-v1","schema_digest":"242b8f9233e915a55ea8d2e72ca476c1258169a67e62de72ee5aed848a6a0a19","provenance":{"artifact":"sts2-protocol/poc-v1","source":"schemas/poc-v1.schema.json","generator":"hand-authored"},"correlation_id":"corr-0002","instance_id":"instance-1","session_id":"session-1","lease_id":"lease-1","generation":1,"observation":{"available_units":2,"settled_effects":1},"action":{"action_id":"use_budget","units":1},"status":"accepted","error_code":null}
{"boundary":"game-core","tool":"submit_action","kind":"action_response","sequence":9,"protocol_version":"poc-v1","schema_digest":"242b8f9233e915a55ea8d2e72ca476c1258169a67e62de72ee5aed848a6a0a19","provenance":{"artifact":"sts2-protocol/poc-v1","source":"schemas/poc-v1.schema.json","generator":"hand-authored"},"correlation_id":"corr-0002","instance_id":"instance-1","session_id":"session-1","lease_id":"lease-1","generation":1,"observation":{"available_units":2,"settled_effects":1},"action":{"action_id":"use_budget","units":1},"status":"accepted","error_code":null}
{"boundary":"harness","tool":"submit_action","kind":"action_response","sequence":10,"protocol_version":"poc-v1","schema_digest":"242b8f9233e915a55ea8d2e72ca476c1258169a67e62de72ee5aed848a6a0a19","provenance":{"artifact":"sts2-protocol/poc-v1","source":"schemas/poc-v1.schema.json","generator":"hand-authored"},"correlation_id":"corr-0003","instance_id":"instance-1","session_id":"session-1","lease_id":"lease-1","generation":1,"observation":{"available_units":2,"settled_effects":1},"action":{"action_id":"use_budget","units":0},"status":"rejected","error_code":"sts2.game-core/zero_units"}
{"boundary":"mcp","tool":"submit_action","kind":"action_response","sequence":11,"protocol_version":"poc-v1","schema_digest":"242b8f9233e915a55ea8d2e72ca476c1258169a67e62de72ee5aed848a6a0a19","provenance":{"artifact":"sts2-protocol/poc-v1","source":"schemas/poc-v1.schema.json","generator":"hand-authored"},"correlation_id":"corr-0003","instance_id":"instance-1","session_id":"session-1","lease_id":"lease-1","generation":1,"observation":{"available_units":2,"settled_effects":1},"action":{"action_id":"use_budget","units":0},"status":"rejected","error_code":"sts2.game-core/zero_units"}
{"boundary":"gateway","tool":"submit_action","kind":"action_response","sequence":12,"protocol_version":"poc-v1","schema_digest":"242b8f9233e915a55ea8d2e72ca476c1258169a67e62de72ee5aed848a6a0a19","provenance":{"artifact":"sts2-protocol/poc-v1","source":"schemas/poc-v1.schema.json","generator":"hand-authored"},"correlation_id":"corr-0003","instance_id":"instance-1","session_id":"session-1","lease_id":"lease-1","generation":1,"observation":{"available_units":2,"settled_effects":1},"action":{"action_id":"use_budget","units":0},"status":"rejected","error_code":"sts2.game-core/zero_units"}
{"boundary":"game-mod","tool":"submit_action","kind":"action_response","sequence":13,"protocol_version":"poc-v1","schema_digest":"242b8f9233e915a55ea8d2e72ca476c1258169a67e62de72ee5aed848a6a0a19","provenance":{"artifact":"sts2-protocol/poc-v1","source":"schemas/poc-v1.schema.json","generator":"hand-authored"},"correlation_id":"corr-0003","instance_id":"instance-1","session_id":"session-1","lease_id":"lease-1","generation":1,"observation":{"available_units":2,"settled_effects":1},"action":{"action_id":"use_budget","units":0},"status":"rejected","error_code":"sts2.game-core/zero_units"}
{"boundary":"game-core","tool":"submit_action","kind":"action_response","sequence":14,"protocol_version":"poc-v1","schema_digest":"242b8f9233e915a55ea8d2e72ca476c1258169a67e62de72ee5aed848a6a0a19","provenance":{"artifact":"sts2-protocol/poc-v1","source":"schemas/poc-v1.schema.json","generator":"hand-authored"},"correlation_id":"corr-0003","instance_id":"instance-1","session_id":"session-1","lease_id":"lease-1","generation":1,"observation":{"available_units":2,"settled_effects":1},"action":{"action_id":"use_budget","units":0},"status":"rejected","error_code":"sts2.game-core/zero_units"}
```

## Evidence classification

- `test-confirmed`: the copied artifact identity, typed wire shapes, fixture lineage, and exact
  source/package/conformance checksums verify; two runner executions
  produce identical trace bytes; the accepted action changes units `3 -> 2`, generation `0 -> 1`,
  and effects `0 -> 1`; the rejected zero-unit action preserves state and reports the stable error;
  the actual ordered five-hop ledger and required metadata are present.
- `source-derived`: the runner, trace ledger, and doubles are local code in `crates/harness/src/poc/`;
  no sibling repository implementation crate is imported or invoked.
- `proposed`: a future authorized integration may replace each double with its declared boundary
  adapter while retaining the same path and evidence fields.
- `unverified`: live MCP framing/transport, gateway allocation and fencing, process/listener and host
  loading, game compatibility, action legality, effect settlement, provider execution, and production
  deployment behavior.

## Reproduction and gate results

Run from the repository root:

```text
cargo metadata --locked --no-deps --format-version 1
cargo run --locked --offline --package repo-policy -- --strict
cargo fmt --all --check
cargo clippy --locked --offline --workspace --all-targets --all-features -- -D warnings
cargo test --locked --offline --workspace --all-targets --all-features
(cd protocol-artifact/poc-v1 && sha256sum -c SHA256SUMS)
```

The expected successful results are: metadata exits `0`; policy reports zero warnings/errors; format,
Clippy, and tests exit `0`; and all ten artifact checksum lines report `OK`. These commands are
offline source/test checks and do not upgrade any `unverified` claim.
