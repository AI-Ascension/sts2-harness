# Changelog archive: 2026-10-22

This file preserves the oldest completed entries moved from
[`CHANGELOG-ARCHIVE.md`](CHANGELOG-ARCHIVE.md) when that rolling archive reached its preferred
size budget. The entries are verbatim historical records, not a supported release or current
runtime-compatibility claim.

### Archived from CHANGELOG.md

- Persist validated Runtime-v3 action-wait settlement against its original durable operation
  before admitting another model decision. Previously the host could settle the action while
  the durable store retained `unknown`, causing the next decision to fail with a misleading
  provider-malformed error. Unresolved waits and invalid witnesses retain durable uncertainty.
  Synthetic runtime tests cover decision admission, repeated waits, and database reopening;
  native campaign validation remains a separate gate.

- Tighten optional seeded-receipt replay admission before `EpisodeRunner` construction. A receipt
  preamble now requires exact current seed configuration and original operation/fence/context
  equivalence, a fresh canonical-seed run-start witness, and a closed MCP wrapper/result chain.
  This is deterministic source/component validation only; it does not execute replay or invoke a
  provider, MCP server, gateway, or host.

- Digest all serialized Runtime-v3 telemetry lineage identities with domain-separated SHA-256
  values while retaining raw trace lineage only for private OTLP topology derivation. Exporter
  tests cover raw prompt, model-output, credential, path, and proprietary-text sentinels through
  the full serialized OTLP envelope. Backend queries must use the deterministic digest while raw
  mappings remain access-controlled local run evidence; collector/backend and live evidence remain
  unverified.

- Add the bounded `coop-native-v1` cohort coordinator and canonical-peer attribution safety
  correction. A returned observation can be attributed only when its sole local peer equals the
  originally scheduled canonical actor and its instance/session/lease/epoch fence matches the
  original operation. Mismatches retain pending or unknown operations for same-operation
  reconciliation. The frozen artifact, wire profile, digest, and producer goldens are unchanged;
  route credentials remain outside harness records. This is source/component evidence only, not
  native multiplayer transport, settlement, or release compatibility.

- Reject incomplete exact checkpoint manifests, inconsistent dependency sizes and payload
 identities, and source/destination profile mismatches before session admission. Bound reads of
 persisted exact artifacts to 16 MiB. These checks establish component integrity, not live restore
 certification; see ADR 0016.

- Add a pinned runtime-peer CI lane. It builds the candidate harness against immutable gateway and
  MCP executable peers, uses a bounded synthetic mod HTTP endpoint only as downstream, and runs
  positive plus foreign-identity and malformed-envelope rejection cases. Startup and cancellation
  cleanup regressions run in the same lane. This is source-derived synthetic process composition
  evidence, not game-host, provider, or release qualification.

- Require a closed `ExoRestrictedProfile` in the trusted Exo configuration for issue #140.
  Admission now fails closed before inference on any non-empty/unreviewed model tool, duplicate or
  invalid tool names, unsafe or overlapping private state/cache/temp roots, unbounded
  quota/retention, or permissions other than `0o700`, with canonical catalog and profile digests.
  The reviewed model tool allowlist is intentionally empty; TypeScript dispatch, OS containment, and
  native private-state enforcement remain follow-up work.
