# ADR 0087: Served Synthetic Exo Profile Source

Status: Proposed for source review. This source increment does not complete issue #391 or Console
R3 acceptance.

## Context

ADR 0085 defines an opaque guarded synthetic loopback inspection and strict one-shot transport, but
the standalone runtime and served profile source do not select that boundary. The production HTTPS
identity and preflight remain the ordinary route and must not be weakened for a loopback fixture.

## Decision

Add the exact `STS2_EXO_ADMISSION=synthetic-envelope` selector for provider kind `exo`. Absence and
`envelope` retain ordinary reviewed admission; explicit `legacy` retains its existing raw-wire
acknowledgement; all other values refuse. Synthetic selection refuses Harness lookup, lifecycle
configuration and live-episode admission before bridge, provider, model or game execution. The
served service may initialize local owner/storage services before these request-side refusal checks;
this source increment does not establish refusal before all service startup I/O.

Derive schema-only `NativeCapabilities` only from an opaque `SyntheticLoopbackInspection` or
`SyntheticExoAdmissionPlan`. The descriptor binds the full inspected identity and checked-in
provider-session, Exo wire, decision, guarded-config and executor schemas. Its only provider method
is `turn/start`; it makes no model-call, lifecycle, recovery or native-execution claim.

Keep `RuntimeV3AdmissionMode` and the transport sum local to the runtime binary. The ordinary sum
arm retains the full public `AdmittedExoRuntimeTransport<ExoProcessTransport>` and forwards its
existing exchange and close interface. The served direct-provider path remains owned by
`ProviderSessionPolicyOwner`; this decision does not reuse lifecycle `ProviderSessionBroker`.

Add a bounded `--synthetic-provider-capabilities CONFIG DIGEST INSTANCE` read-only bridge command.
It constructs the exact `--synthetic CONFIG DIGEST` process description with an empty inherited
environment, inspects guarded-v2 configuration through the loopback route, and emits bounded
schema-only capabilities. Existing production capability description, HTTPS preflight, public
admission enums, wire schemas and legacy behavior remain unchanged.

## Evidence and gates

Source regressions cover selector and provider fences, identity and route pins, transport exchange
and close forwarding, and the exact `decision.live.v1` / `context.live.v1` dispatch fence. They do
not establish build, runtime, provider, native, deployment, game-action or Console recovery evidence. The
supervisor owns formatting, compilation, policy, tests, review and publication. Issue #391 and
Console issue #18 remain open pending their independent gates.
