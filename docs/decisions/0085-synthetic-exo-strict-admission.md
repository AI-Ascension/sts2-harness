# ADR 0085: Synthetic Exo Strict Admission

Status: Proposed for source review. This decision covers a typed library boundary only; it does
not enable a runtime selector or complete the Context Console process-recovery handoff.

## Context

Production Exo identity validation requires HTTPS and production preflight checks the reviewed
provider route, capability descriptor, trusted profile and private-state policy. A synthetic
Responses-wire peer must stay on literal loopback while using the same bounded v1 decision
envelope. Reusing production preflight would either reject that route or require weakening the
production endpoint rule.

Issue [#391](https://github.com/AI-Ascension/sts2-harness/issues/391) remains open. This source
slice adds a narrowly typed strict-envelope admission path for a synthetic loopback bridge. It is
not the real owner-process restart, receipt-lookup or no-repeat-effect witness required by the
remaining Console R3 sequence.

## Decision

Keep `ExoIdentity::validate`, production descriptor validation, provider preflight, bridge schemas,
runtime selection, lifecycle broker behavior and the `--run` / `--run-v2` paths unchanged. Add a
crate-private synthetic identity validator that admits only model `o3-pro` at the existing literal
`http://127.0.0.1:<nonzero-port>` route, with provider identity `openai`. It does not admit hostnames,
external addresses, zero ports, or the lifecycle-v2 wire.

The public inspection constructor accepts an `ExoProcessConfig`, an executor package locator and an
operator-supplied native-instance identifier. It accepts exactly the retained `--synthetic`,
absolute config path and 64-digit lowercase config digest arguments, rejects inherited environment
variables, loads the config internally, and requires guarded-v2 private state. It compares the
canonical package locator with the configured executor and hashes the loaded executor, extension
and bridge bytes. The native-instance identifier is bound as supplied; this source slice does not
independently attest its origin.

The inspection result has private fields, no deserialization or public parts constructor, and no
transport parameter. Admission compares its complete identity against the independent descriptor
and trusted identity, checks the exact current contract/source, guarded private-state policy,
standard/fresh/Responses/Linux profile and reviewed limits, then retains the inspected process
configuration. The resulting transport owns `ExoProcessTransport`; constructing the plan or
transport does not start a child. The first valid `exchange` is the process effect.

Production and synthetic transports share one private strict-v1 fence for request bounds, profile
and model-execution binding, request/turn correlation, legal-action validation, response limits and
one-shot consumption. Synthetic capability decisions are limited to those the one-shot bridge
actually dispatches. The synthetic report is a separate type: it has no `model_calls` field and
does not copy or promote lifecycle, recovery, idempotency, native or game capabilities from the
descriptor.

## Compatibility and evidence

Existing production HTTPS admission remains authoritative for production transports. This change
does not alter serialized schemas, legacy profiles, lifecycle-v2, the provider-session broker, or
runtime command selection. Source fixtures may use fake `Loaded` records to test structural
comparisons; such fixtures do not prove a real config load, real bridge launch, provider call, game
action or Console recovery result. Those checks remain separate gates.

Inspection canonicalizes and reads path-backed files before a later process launch; it does not
hold open file handles or prevent local file replacement between inspection and launch. Therefore
this API does not claim race-free artifact pinning. A caller requiring that guarantee needs a
separate reviewed descriptor-based execution boundary. The bridge source pins its configured
executor and extension, but this admission record is not a network sandbox or native acceptance.

## Validation status

Meaningful source tests cover the synthetic route and argument grammar, identity/profile/policy
mismatches, strict response correlation, legal actions and one-shot refusal after an ambiguous
transport error. For this source handoff, formatting, compilation, lint, tests, runtime and hosted
checks were not run. The root owner must review the exact source, run allocated gates and decide
whether to merge.
