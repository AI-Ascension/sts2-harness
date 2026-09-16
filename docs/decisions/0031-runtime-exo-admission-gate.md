# ADR 0031: Runtime Exo admission gate

## Status and scope

Accepted for harness issue #139. This records the production boundary that invokes the ADR 0017
preflight and the `ExoAdmittedTransport` envelope, and the mode that explicitly acknowledges the
un-admitted raw-wire process bridges. It does not approve a full-runtime capability claim, a live
provider run, or a silent fallback from an admitted deployment to a weaker one.

## Context

`crates/harness/src/exo/contract/preflight.rs` and `crates/harness/src/exo_admitted_transport.rs`
were contract- and test-complete but had no non-test caller: the runtime-v3 seam built
`ExoProcessTransport` and handed it straight to `ExoProvider`. A missing, malformed, unknown or
unverified deployment could therefore reach a model or game effect, and ADR 0017 recorded exactly
that gap.

Two process-bridge wire families exist and they are not interchangeable:

| Bridge | Request shape | Accepts `sts2.exo-bridge-wire-v1`? |
| --- | --- | --- |
| `sts2-exo-bridge` (harness-owned, ADR 0018) | versioned envelope | yes |
| `sts2-ollama-bridge`, `sts2-astra-bridge`, synthetic probes | raw JSON object | no |

Unconditionally wrapping the seam in the envelope would break every currently executable bridge,
and silently reinterpreting a raw request would be worse. The gate is therefore explicit.

## Decision

`STS2_EXO_ADMISSION` selects the admission mode while `RuntimeV3Settings::from_environment`
assembles settings:

- `envelope` (also the value used when the variable is absent) is the reviewed, fail-closed
  default. The runtime assembles the operator-trusted deployment identity, builds the reviewed
  descriptor, runs the model-free `preflight`, and only then admits a correlated turn through
  `ExoAdmittedTransport`. Any refusal ends the run while settings are still being assembled, before
  the durable store, the gateway connection, the MCP session, the provider or any game effect
  exists.
- `legacy` is an explicit operator acknowledgement of an un-admitted raw-wire bridge. The process
  transport is passed through unenveloped, exactly as before.

The descriptor keeps the capability axes shipped by `ExoCapabilityDescriptor::source_review()`
with the deployment identity axes supplied by the operator-trusted configuration. The runtime does
not promote a capability to `Supported` on the bridge's behalf. Because the reviewed descriptor
still reports `evidence.turn_identity`, `lifecycle.cancellation` and `lifecycle.recovery` as
`Unverified`, the reviewed envelope mode refuses the current deployment with
`RequiredCapability("evidence.turn_identity")`. That refusal is the intended fail-closed outcome:
the criterion "missing/malformed/unknown capabilities, swapped package bytes, wrong revisions and
unsupported schemas fail preflight before model/game effects" is satisfied by refusing, not by
asserting an unverified capability.

## Open decision

`ExoAdmittedTransport` is single-use by contract (`ExoLimits::reviewed().max_turns == 1`) while a
runtime episode performs many model executions. Serving a multi-turn episode over the envelope
therefore needs a per-turn admission boundary (a fresh admitted turn per `exchange`) or an
admission-aware provider, together with the honest capability promotion this ADR withholds. Until
that work lands and is reviewed, the envelope mode stays fail-closed and `legacy` remains the only
executable path. This limitation is recorded rather than worked around.

## Consequences

- A production launch that does not acknowledge the raw wire now fails closed with an actionable
  message instead of silently running an un-admitted deployment.
- The raw-wire development bridges documented in `docs/OLLAMA_MODEL_SELECTION.md` and
  `experiments/live-combat/README.md` require `STS2_EXO_ADMISSION=legacy`.
- The refusal happens before any gateway, MCP, provider or game effect, and is covered at the
  process boundary by `crates/harness/tests/runtime_startup.rs` and at the transport boundary by
  `crates/harness/tests/exo_admission.rs`.
