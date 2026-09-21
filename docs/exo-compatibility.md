# Exo executor bridge compatibility

This page holds the Exo-specific compatibility record split out of [`COMPATIBILITY.md`](COMPATIBILITY.md):
the pinned executor bridge axes, the restricted profile, and the runtime admission gate. The
general rules (independent axes, classifications, lineage, promotion evidence) remain in that file.

## Pinned Exo executor bridge

The harness-owned `sts2-exo-bridge-v1` contract is documented in
[ADR 0017](decisions/0017-exo-executor-bridge-contract.md) and frozen in
[`protocol-artifact/exo-bridge-v1`](../protocol-artifact/exo-bridge-v1/README.md). Its source
revision, package/executable, dedicated TypeScript extension, bounded bridge, model binding,
prompt/tool/configuration digests, contract version, and native instance identity are separate
compatibility axes. The selected executor path is the extension's
`defineHarness.runTurn` → `runResponsesHarnessTurn` → `ResponsesRuntime.complete` chain; HTTP
substrate requests, `/health`, and the human-facing CLI are not fallback executors.

The closed capability/preflight descriptor and request/turn envelope drive the production
admission gate recorded in [ADR 0031](decisions/0031-runtime-exo-admission-gate.md). The reviewed
`STS2_EXO_ADMISSION=envelope` mode refuses the run while a required capability, digest, revision,
route or schema is not admitted, and it refuses before the durable store, the gateway, the MCP
session, the provider or any game effect exists; `STS2_EXO_ADMISSION=legacy` is an explicit
operator acknowledgement of an un-admitted raw-wire bridge. That gate cross-checks the identity
**inspected** from the launch's own artifacts against the operator pin
([ADR 0032](decisions/0032-inspected-admission-identity.md)), so a swapped package, extension or
bridge artifact fails closed and a pinned axis the inspection did not bind refuses as
`UnboundIdentity` instead of being admitted on the declaration alone. Standard/fresh/Linux x86_64 and
strict terminal decision parsing are source-derived;
map/expert, continuity, cancellation/recovery, event/usage, replay, native package/model
identity, live Exo connectivity, and STS2 gameplay remain `unverified` until the real pinned
executor spike records them. The required `runtime`, `provider`, and `endpoint` identity axes are
classified as a `breaking` required-configuration correction; readers that cannot validate them
fail closed. The tightened expert digest and non-empty legal-action schema, plus parser-only
checks for duplicate action IDs, UTF-8 byte bounds, and `hp <= max_hp`, are a
`safety-correction`. Schema-valid/parser-rejected semantic cases are executable conformance
vectors. Migrations must stage additive records, run schema and trusted preflight checks, retain a
backup until handoff, and restore that backup on failure without reinterpreting new records as
legacy `provider_revision`-only identities.

### Restricted profile: forbidden tools, private state, capability list

The reviewed model tool catalog is empty (`ExoToolCatalog::reviewed()`; its `catalog_digest` is the
`tool_digest` axis). Upstream builds no default registry when the module supplies `registerTools`,
but the extension does not depend on that alone: it seals the actual `HarnessToolRegistry` handed to
every model round, so a registry that already carries a tool, any later `register`, and any
`executePending` dispatch — `shell`, `install_agent_tool`, `uninstall_agent_tool`, `manage_tool`,
`inspect_tools`, `install_skill`, `remember`, lookup-profile tools, and any case or namespace
variant — throws the typed `sts2_forbidden_tool` error before a handler can exist, and the denial
counts are recorded in the `sts2.exo-tool-guard-v1` custom event. The executor requires that event
and maps a non-zero count to receipt `error_code: exo_forbidden_tool` with no decision; the bridge
then fails closed as `exo_bridge_executor_failed` after exactly one model egress. These are recorded
against the real pinned Exo with a synthetic loopback model in
`docs/evidence/exo-executor-process-oracle-20260917.{md,json}` (`forbidden_tool_by_name_*` and
`request_tools_are_empty`); no tool reaches a shell, arbitrary networking, secret/store
administration, raw host access, or game mutation. This is source/process evidence, not a provider
or native run.

`STS2_EXO_PRIVATE_STATE_ROOT` (optional; default `/var/lib/sts2-harness/exo-runtime`) selects the
base of the reviewed `ExoPrivateStatePolicy` that envelope admission validates before inference:
`<root>/state`, `<root>/cache`, and `<root>/temp`, the default quota and retention bounds, and
`0700` permissions. Validation is lexical (absolute path, no `..`, no `home`/`root`/`Users`
component, no system or game-install prefix). The bridge still creates its fresh `0700` per-run
child under the caller's `TMPDIR`; materializing the declared roots, enforcing the quota, and
retention sweeps remain open under #140.

The truthful supported-capability list is the bridge `--describe` descriptor
(`sts2.exo-one-shot-capability-v1`): the pinned source revision; bridge, executor, extension, and
Node digests; the empty-catalog `tool_digest`; the configuration digest; the configured model and
endpoint; `profiles: [standard]`; `context_modes: [fresh]`; the four decision kinds; `max_turns: 1`;
`max_tool_round_trips: 0`; and `full_runtime_admission: false`. Source freeze and re-admission: any
change to the pinned source tree, the extension bytes, the executor, Node, the configuration, or the
model binding changes a digest, so the operator pin no longer matches and admission refuses before
any durable store, gateway, MCP, provider, or game effect (ADR 0031/0032). Adopting a changed
artifact means reviewing and pinning the new identity through the owner control path, never editing
an admitted deployment in place.

## Runtime Exo admission gate

[ADR 0031](decisions/0031-runtime-exo-admission-gate.md) wires the ADR 0017 preflight and the
`ExoAdmittedTransport` envelope into the runtime-v3 transport seam. This is `breaking` for
operator configuration: `STS2_EXO_ADMISSION` now selects the admission mode, the reviewed
`envelope` mode is the default when it is absent, and it requires the complete operator-trusted
deployment identity (`STS2_EXO_PACKAGE_DIGEST`, `STS2_EXO_EXTENSION_DIGEST`,
`STS2_EXO_BRIDGE_DIGEST`, `STS2_EXO_MODEL_BINDING`, `STS2_EXO_PROVIDER`, `STS2_EXO_ENDPOINT`,
`STS2_EXO_PROMPT_DIGEST`, `STS2_EXO_TOOL_DIGEST`, `STS2_EXO_CONFIG_DIGEST`,
`STS2_EXO_NATIVE_INSTANCE_ID`, `STS2_EXO_MODEL_EXECUTION_ID`, `STS2_EXO_REQUEST_ID`,
`STS2_EXO_TURN_ID`), plus one required artifact locator, `STS2_EXO_PACKAGE_PATH`. A missing or
unverified deployment, or an absent or empty package locator, ends the run while settings are
assembled, before any gateway, MCP, provider or game effect, and no request bytes are emitted.
`envelope` inspects the exact bytes at `STS2_EXO_PACKAGE_PATH` and the exact bytes of the bridge
executable it is about to launch, and hashes both into the inspected identity, so a swapped package
is refused as `IdentityMismatch("package_digest")` and a swapped bridge as
`IdentityMismatch("bridge_digest")`. The inspected digests are always computed from the located
bytes; `STS2_EXO_PACKAGE_DIGEST` and `STS2_EXO_BRIDGE_DIGEST` are never substituted for the
observation. The identity comparison precedes the capability gate and `package_digest` is evaluated
first. The envelope now requires launch arguments `--run`, an absolute bridge configuration path,
and its digest. The shared bridge loader verifies that configuration, the executor, the reviewed
extension, Node and source revision. The package locator must resolve to that same executor.
Extension and prompt identity bind the complete reviewed extension source; tool identity uses the
reviewed tool catalog; model, provider route and configuration identity come from the verified
launch configuration. The instance identity comes from the gateway runtime configuration and is
subsequently subject to gateway allocation validation. It is not native acceptance evidence.
This is a breaking operator-configuration change, with unchanged wire schemas. It removes the
unbound extension obstacle but does not promote unverified cancellation/recovery capabilities;
those still prevent full admission until their runtime composition is verified.
`STS2_EXO_PACKAGE_PATH` is a
backward-incompatible addition to the reviewed envelope contract — every deployment that does not
supply it now fails closed with `STS2_EXO_PACKAGE_PATH is required`. The already-documented raw-wire
development bridges
(`docs/OLLAMA_MODEL_SELECTION.md`, `experiments/live-combat/README.md`) must set
`STS2_EXO_ADMISSION=legacy`, which is an explicit acknowledgement of an un-admitted bridge rather
than an admission. Rollback is to set `legacy`; no wire field, schema, contract version or durable
record changes. Per-turn envelope admission for a multi-turn episode remains open.

### Live-episode admission

A live episode is a capability the provider kind declares rather than a spelling beside the lane
([ADR 0060](decisions/0060-live-episode-capability-admission.md)). `openai-astra` and `exo` declare
it; `ollama`, `typesafe-jev` and `synthetic` do not, and a run that declares `STS2_LIVE_EPISODE=true`
on one of them is refused. The Exo lane's capability is backed by the reviewed envelope's
inspection of its descriptor, so `exo` carries a live episode only under `STS2_EXO_ADMISSION=envelope`;
the raw-wire `legacy` acknowledgement does not inspect the descriptor and cannot stand in for it.
The Astra lane is unaffected and still takes its live episode on the raw-wire lane under the digest
and argument checks it always applied.

A `STS2_PROVIDER_KIND` this runtime does not implement — `openai_astra`, `OpenAI-Astra`, `exo-bridge`
or any typo — is refused while settings are assembled, before the durable execution store, the
gateway, the MCP session or the provider exists. Previously such a name took the non-bridge branch
and ran under the reviewed Exo source revision, so it selected a lane nobody named. The admitted
live mode is installed once per process and is what the replay stream and the live diagnostics read;
a process that inherits the variable without the admission behaves as a standard run. The lanes that
are admitted and their requirements are otherwise unchanged: this is a refusal of an unimplemented
name, not a new lane.
