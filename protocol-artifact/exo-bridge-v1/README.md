# STS2 Exo bridge v1

This is the harness-owned source and contract record for the pinned STS2-to-Exo executor seam.
It is a review artifact, not an Exo distribution, model package, game adapter, or proof of a
native run. The artifact keeps source revision, package/executable digest, STS2 extension digest,
bridge digest, model binding, prompt/tool/configuration digests, contract version, and native
instance identity as independent axes.

## Selected executor path

The only admitted executor runtime is the Responses runtime, reached through a dedicated
operator-owned TypeScript STS2 extension loaded as `agent.typescript.module_path`. The extension is
run by the harness TypeScript runner and enters Exo through:

```text
defineHarness.runTurn
  -> runResponsesHarnessTurn
  -> ResponsesRuntime.runTurn
  -> ResponsesRuntime.complete / completeStream
```

The runtime is model-dependent, not unconditional. The pinned upstream `runtimeFromModelBinding`
selects `AnthropicRuntime` for `claude*` bindings, `ChatCompletionsRuntime` by default, and
`ResponsesRuntime` only when upstream `modelRequiresResponsesApi` is true. That predicate is true
only for a lowercased binding starting with `o1-pro`, `o3-pro`, or `gpt-5-pro`, carrying a
`gpt-5.N` minor version of 3 or greater, or starting with `gpt-5` and containing `-codex`. The
harness mirrors this in `crates/harness/src/exo/contract/preflight.rs::responses_capable`. Trusted
preflight rejects any `runtime` other than `responses` and any approved `model_binding` that is not
Responses-capable, so a `gpt-4o` binding fails closed instead of silently selecting
`ChatCompletionsRuntime`. There is no unconditional forced runtime path.

The extension owns the narrow STS2 prompt/tool projection and returns one terminal decision to the
harness-owned bounded bridge. The extension does not expose raw game objects, host reflection,
private prompts, credentials, saves, shell commands, hidden RNG state, or unrevealed outcomes.

The owned source placement is frozen at
`experiments/exo-agent/extension/package.json` with entry
`experiments/exo-agent/extension/src/index.ts`. The candidate has one root `exo` package; its
`@exo/harness` and `@exo/model-runtime/turn-loop` specifiers resolve through root `tsconfig.json`
path aliases, not `workspace:*` dependencies. The extension README pins Node `22.14.0`, pnpm
`10.26.2`, and locked install/typecheck/lint/test commands. The repository-owned bridge placement is `crates/harness/src/exo_process.rs` and
`crates/harness/src/exo/contract/`; its external executable is operator-owned and identified by
`bridge_digest`. Direct OpenAI SDK, Exo CLI, substrate HTTP, shell, game-host, and private-state
dependencies are outside this contract.

The future module and process bridge must use the closed
`sts2.exo-bridge-wire-v1` envelope after trusted preflight integration. The current Rust runtime
still uses its legacy request/process seam and does not claim this admission or envelope handoff.

The upstream `/health` endpoint is a service probe and is not an executor contract. The upstream
`/request` endpoint carries low-level substrate protocol messages and has no STS2 executor-turn
operation. The CLI `conversation send` command is a human-facing prompt path. Neither endpoint nor
the CLI is a fallback for this contract.

## Closed capability descriptor

`schema.json` defines `sts2.exo-capability-v1` with `additionalProperties: false`. The descriptor
must name the exact contract version and source revision, and must carry all identity axes. It
advertises decision kinds (`plan`, `action`, `wait`, `reobserve`, `recovery`), independent
standard/map/expert profile states, fresh/continuity context modes, an explicit platform list,
independent request/map/response/event byte limits, turn/time/concurrency/tool-round-trip limits,
and separate lifecycle and evidence states.

The checked-in descriptor is `source-derived`: standard/fresh/Linux x86_64 and terminal decision
parsing are supported at the harness boundary. Map, expert, native turn identity, event/usage,
replay, cancellation, and recovery remain `unverified` until a real pinned executor run produces
the corresponding evidence. A preflight with a source-only descriptor therefore cannot admit an
inference deployment.

Preflight is pure. It compares the descriptor to operator-trusted complete identities, profile,
context mode, runtime, platform, and limits without contacting Exo, a model, an HTTP endpoint, or a
game host. It requires `runtime = responses` and a Responses-capable `model_binding`; missing or
mismatched package, extension, bridge, model, prompt, tool, config, or native instance values fail
closed.

## Bounded bridge wire

The selected bridge wire is `sts2.exo-bridge-wire-v1`. Its request envelope has exactly:

```text
wire_version, request_id, turn_id, request
```

Its terminal response envelope has exactly:

```text
wire_version, request_id, turn_id, outcome, decision, error_code
```

`outcome` is `decision`, `cancelled`, or `failed`. A decision outcome carries one unchanged
`sts2.exo-decision-v1` object; cancellation carries neither decision nor error code; failure
carries only a bounded machine error code. The inner decision is parsed by the existing strict
parser, so no correlation fields are appended to or silently accepted by `parse_decision`.

Both envelopes are one UTF-8 JSON value. The parser rejects empty/oversized frames, invalid UTF-8,
trailing bytes, duplicate keys at any nesting level, unknown fields, wrong envelope versions,
invalid IDs, malformed inner requests, and non-terminal decision shapes. Request IDs and turn IDs
are bounded control-plane identities and are compared before a response is accepted. Run, episode,
agent, conversation, session, and idempotency identities remain in a host-only control receipt;
they are never model-visible.

`schema.json` includes closed `$defs` for `decision_request`, `request_envelope`, `decision`, and
`decision_envelope`. Its `observation` union closes `standard_observation` (with
`legal_actions` requiring at least one entry) and closes `expert_observation` to the full
Runtime-v4 expert shape, requiring `protocol_version: "runtime-v4-expert"` and rejecting unknown
privileged fields. The `hash` definition rejects the all-zero digest. `conformance.json` names
executable vectors for standard/map boundaries, every semantic decision (including each
`recovery_kind`: `reobserve`, `reconcile`, `release_lease`, and `stop_episode`), wrong
schema/wire, swapped package identity, failed outcomes, and capability downgrades. The Rust
`exo_contract` production test executes those vectors.

The ordinary request limit is 131072 bytes, the complete map request limit is 393443 bytes, the
response limit is 8192 bytes, and the supervised turn timeout is 120000 milliseconds. Process
bridges receive one request, then stdin is explicitly shut down (EOF); stdout is bounded and
non-success, timeout, cancellation, and malformed outcomes fail closed. No retry or gameplay
fallback is implied by a transport error.

## Source review

The immutable manifest records the nine commits from the old audit revision
`7801005e6a1ab77008a05dbba80e0a2a7a56e35d` through candidate
`b06869ab789dee3f80ca474b5fa89dbe47ccb859`. All nine commits change Firecracker/sandbox storage
or lifecycle, ExoChat reconnect behavior, workflow image materialization, or guardian tooling.
None adds an STS2 executor-turn machine operation or a bounded terminal decision hook. The
candidate tree is recorded separately from the source commit. This source review is not a
package/executable digest and is not native compatibility evidence.

## Identity migration

The old `provider_revision` setting remains a source revision input for legacy request validation,
but new deployment records must not overload it as a package, bridge, model, or configuration
identity. Consumers migrate to the separate axes listed above and bind the contract version.
Readers that cannot understand the new contract or identity fields reject the deployment rather than
guessing. Historical records may retain the old audit revision as source-derived history.

Old stores migrate by backup → atomic additive identity/contract migration → reviewed-pin and
minimum-capability preflight before any effect. The original store remains untouched until the
new record is admitted; a failed migration removes only staging bytes and restores the backup.
Mixed-version or missing-axis rows are rejected, and rollback never reinterprets a new record as
the legacy `provider_revision`-only identity.

## Evidence and completion

The source and contract fixtures are deterministic component evidence. A real completion requires:

1. build the exact candidate Exo package and record its package/executable digest;
2. build the dedicated extension and bounded bridge and record both digests;
3. bind the model, prompt, tool catalog, and operator configuration digests;
4. run the candidate executor with the original synthetic model endpoint;
5. capture correlated request/turn IDs, one structured terminal decision, event/usage evidence,
   cancellation and recovery outcomes, and a replay/idempotency result;
6. repeat on the declared platform matrix and record the live handoff.

Until those steps occur, native Exo connectivity, model behavior, STS2 extension compatibility,
terminal gameplay, Victory/full-run completion, co-op, compaction, and release compatibility are
`unverified`.
