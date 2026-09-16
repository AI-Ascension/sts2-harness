# ADR 0032: Inspected admission identity for the runtime Exo gate

## Status and scope

Accepted for harness issue #139 criterion 3. It supersedes the identity paragraph of
[ADR 0031](0031-runtime-exo-admission-gate.md) and keeps every other part of that gate. It records
that the runtime admission boundary cross-checks the identity it *inspected* from the deployment it
is about to launch against the operator's pin, and that it refuses a pinned axis no inspected
artifact backs. It does not promote a capability, approve a live provider run, or approve admitting
an un-inspected artifact.

## Context

ADR 0031 landed the runtime admission gate. Its independent review
(`lane-records/l139-pr205-review.md`, non-blocking finding 1) recorded that the digest cross-check
was inert: `ExoAdmissionPlan::reviewed_descriptor` copied the operator-trusted identity *into* the
descriptor before `preflight` compared the two, so `compare_optional_identity` compared a value with
itself. "Swapped package bytes" was therefore covered by digest *format* validation only, and
nothing hashed the bridge executable against `STS2_EXO_BRIDGE_DIGEST`.

Two further properties hid a genuine mismatch. `require_minimum_capabilities` ran before the
identity comparison, so a real digest mismatch surfaced as
`RequiredCapability("evidence.turn_identity")` rather than as an identity failure; and the reviewed
descriptor advertised the operator's declaration rather than anything the harness had observed.

## Decision

1. `ExoAdmissionPlan` carries the **inspected** identity separately from the operator pin.
   `ExoAdmissionPlan::inspected` derives it from exact artifact bytes through
   `ExoInspectedArtifacts::identity`: each digest axis is the SHA-256 of the inspected bytes, and the
   source revision and contract version are the harness-reviewed constants. `reviewed_descriptor`
   advertises the inspected identity, so `preflight` compares two independent values instead of one
   value with itself.
2. `preflight` compares the identity **before** the capability gate. A swapped artifact, a repinned
   digest or a wrong instance identity is refused as `IdentityMismatch(axis)` instead of being
   masked by an unrelated capability refusal. Both variants now name the axis in their message.
3. A pinned axis the inspection did not bind is refused as `UnboundIdentity(axis)`. A deployment is
   never admitted on the operator's declaration alone, which is what makes a swapped artifact fail
   closed rather than pass on a matching declaration.
4. The runtime seam inspects what it can read: the bytes of the bridge executable it is about to
   launch. It deliberately does **not** copy the operator's environment into the inspected identity,
   because that would restore the vacuity for every axis it copied. `package`, `extension`, `prompt`,
   `tool`, `config`, `model_binding`, `provider`, `endpoint` and `native_instance_id` therefore stay
   unbound at this seam, so the reviewed envelope refuses the current deployment with
   `UnboundIdentity("package_digest")` before the capability gate is reached.

## Consequences

- The reviewed envelope now refuses for a strictly stronger reason: it refuses a deployment it could
  not inspect, not merely one whose capability axes are unverified. The refusal is still produced
  while settings are being assembled, before the durable store, the gateway, the MCP session, the
  provider or any game effect exists.
- Both refusal reasons remain fail-closed, so the envelope still refuses every deployment today and
  `legacy` remains the only executable path, exactly as ADR 0031 records. `docs/COMPATIBILITY.md`,
  `CHANGELOG.md` and ADR 0017 are corrected to state the identity reason instead of the capability
  reason.
- To admit a real deployment, a later increment must inspect the remaining artifact bytes and obtain
  the non-byte axes from the bridge (for example a handshake) rather than from the operator's
  environment. Per-turn admission over a multi-turn episode remains open per ADR 0031.
- The bytes of the bridge are read once per launch to compute the digest. A very large executable is
  read whole; the local-provider digest check in `runtime_v3_settings` already bounds that read, and
  unifying the two reads is left to the increment that inspects the remaining artifacts.

## Evidence

- `crates/harness/tests/exo_admission.rs` proves the identity binding at the production boundary:
  swapped package bytes and swapped bridge bytes are refused as `IdentityMismatch` with zero
  transport dispatch, and an unbound pinned axis is refused as `UnboundIdentity` with zero dispatch.
- `crates/harness/tests/exo_contract.rs` and `tests/support/exo_contract_preflight.rs` prove every
  identity axis fails closed at the contract boundary.
- `crates/harness/tests/runtime_startup.rs` proves the refusal reaches the process boundary before
  the gateway, MCP or provider boundaries.
