# Authored-workflow native acceptance recipe

**Status:** `proposed`. This is the executable preparation half of the native acceptance gate
in [sts2-harness#94](https://github.com/AI-Ascension/sts2-harness/issues/94). It binds the
component pins and names the witnesses an operator must capture. It records **no** native
result: nothing here has been executed against a game, and running it is a separate,
separately authorized step.

## What this document is, and is not

Issue #94 separates its native gate into two tasks. T1 is preparation — reuse the recorded
AC1–AC4 served-path evidence, bind its component pins to a disposable-host recipe, and identify
the native action and independent settlement witness AC5 requires. T2 is the execution itself,
which requires host authorization this repository does not have.

**This document discharges the first half of T1 only.** It states what to run, at which pins,
and what must be captured. It does not claim any of it has been run, and satisfying it does
not close #94, discharge T2, or satisfy AC5.

## Why the pins below are the ones that matter

AC1–AC4 are served-path properties. They are only meaningful against a specific composition:
the served binary, the compiled bridge it launches, the gateway and MCP peers it talks to, and
the policy and context-owner stores it opens. A run against different bytes is a different
run, and its result does not transfer. The environment below is transcribed from the served
process composition the repository already exercises
(`crates/harness/tests/support/runtime_v4_executable_composition_process/served/session.rs`),
which is the closest thing this repository has to a production-shaped invocation, so an
operator reproducing it is starting from a known-good shape rather than from prose.

## Component pins to record

Record all five. A record missing any of them is not a #94 AC5 record.

| Component | What to record | Why it is load-bearing |
|---|---|---|
| `sts2-harness` | exact commit and the release binary's SHA-256 | the served binary under test |
| Bridge | exact commit and `STS2_EXO_BRIDGE_BINARY` path plus SHA-256 | `STS2_EXO_REVISION` pins the bridge from its bytes; a run carries two digests and only one is runtime-verified |
| Gateway | exact commit and the address used | owns process authority for the game instance |
| MCP server | exact commit and the binary path | the harness must not bypass the MCP/gateway path to reach a game instance |
| Stores | paths for workflow, execution, policy, and context-owner stores | restart and replay evidence is meaningless without knowing what was persisted where |

Repository policy already records a reviewed Exo source revision and verifies copied artifact
checksums during CI. The bridge revision is a separate fact from the harness revision and must
be recorded separately; do not collapse them into one "version" field.

## Environment for the served process

`sts2-harness-runtime serve-workflow` is the entry point. Four settings have no default and the
process refuses to start without them:

| Variable | Requirement |
|---|---|
| `STS2_WORKFLOW_LISTEN` | must parse as a socket address and **must be loopback**; the process refuses a non-loopback listen address |
| `STS2_WORKFLOW_STORE` | workflow store path |
| `STS2_WORKFLOW_AUTH_PROFILE` | selects the authenticator profile; the matching token variable must be set |
| `STS2_GATEWAY_TOKEN` | gateway bearer token; required by `RuntimeConfig::from_environment` |

`STS2_WORKFLOW_PROVIDER_POLICY_CONFIG` is also required, because
`ProviderPolicyConfiguration::from_environment` reads it with `required(...)` and then refuses a
configuration whose `schema_version` is not the expected one, whose `scope` is invalid, whose
`key_reference` is not a valid environment name, whose `selected_profile` does not equal
`capabilities.profile_id`, or whose capabilities fail their own validation. It is bounded by
`MAX_PROVIDER_POLICY_CONFIGURATION_BYTES`.

`STS2_LIFECYCLE_JOURNAL_DIR` is **opt-in, not mandatory**. When it is unset or empty the
process-lifecycle surface stays composed but unavailable, and every lifecycle command is
refused rather than accepted into a state it could not reconcile after a restart. A run that
needs gateway process-lifecycle authority must set it; a run that does not should leave it
unset so the refusal is visible rather than silently degraded.

The remaining surface, transcribed from the served composition, is:

- **Identity.** `STS2_INSTANCE_ID`, `STS2_CALLER_ID`, `STS2_SESSION_ID`, `STS2_MCP_SESSION_ID`,
  `STS2_RUN_ID`, `STS2_EPISODE_ID`, `STS2_TRAJECTORY_ID`, `STS2_TRACE_ID`, `STS2_ARTIFACT_ID`.
  These are separate namespaces and must not be collapsed into one correlation field. Each has a
  default (`instance-1`, `harness`, `session-1`, `mcp-session-1`, `run-runtime-0001`,
  `episode-runtime-0001`, `trajectory-runtime-0001`, `trace-runtime-0001`,
  `artifact-runtime-0001`), so **record the values actually used** — an unrecorded default is
  indistinguishable from an unset variable in the resulting evidence.
- **Authority.** `STS2_LEASE_ID` (default `lease-1`), `STS2_LEASE_EPOCH` (default `1`, must
  parse as an integer), `STS2_GATEWAY_ADDR` (default `127.0.0.1:15525`). The owner binds both
  lease fields and refuses if either changes within a live run.
- **Profile.** `STS2_RUNTIME_PROFILE` defaults to `runtime-v1` and must be one of
  `runtime-v1`, `runtime-v2`, `runtime-v3-gameplay`, `negotiated-composition-v1`,
  `runtime-v4-expert`, `runtime-v4-expert-rest-action`. The default is almost certainly not the
  profile under test; set it explicitly and record it.
- **Provider policy.** `STS2_WORKFLOW_PROVIDER_POLICY_CONFIG`, a JSON document with
  `schema_version` `ascension.workflow-provider-policy-config.v1`, carrying `store_path`,
  `key_reference`, `scope`, `capabilities`, and `selected_profile`.
- **Context owner.** `STS2_WORKFLOW_CONTEXT_OWNER_CONFIG`, a JSON document with `schema_version`
  `ascension.workflow-context-owner-config.v1`, carrying `store_path`, `key_reference`,
  `owner_id`, `owner_version`, `context_ref`, and a `limits` object. It is parsed with
  `deny_unknown_fields`, so an unexpected key is refused rather than ignored. Three further
  fields are optional and default to absent: `render_required` (`false`), `sources` (empty), and
  `membership` (`None`). Record which of them the run set, because a defaulted `membership`
  means this owner enforces no per-invocation inclusion selector.
- **Model lane.** `STS2_RUNTIME_PROFILE`, `STS2_EXO_REVISION`, `STS2_EXO_ADMISSION`,
  `STS2_EXO_BRIDGE_BINARY`, `STS2_EXO_TIMEOUT_MILLIS`, `STS2_EXO_MAX_REQUEST_BYTES`,
  `STS2_EXO_MAX_RESPONSE_BYTES`, `STS2_OBJECTIVE`. `STS2_EXO_REVISION` and
  `STS2_EXO_BRIDGE_BINARY` are read with `required(...)`, so the bridge revision is never
  silently defaulted. `STS2_EXO_ADMISSION` selects the reviewed envelope path and defaults to
  the fail-closed `envelope` mode; record it, because a run that used a different admitted mode
  is a different evidence class.
- **Peers.** `STS2_MCP_BINARY`, `STS2_EXECUTION_STORE_PATH`.

Secret material is referenced by `key_reference` (for example
`STS2_SERVED_PROVIDER_POLICY_KEY`) and must be supplied out of band. Do not write key values,
private prompts, model output, or personal host paths into any record this recipe produces.

## The witnesses AC5 requires

AC5 asks for a real action plus settlement or an explicit failure, and for evidence that is
independently reviewed. That is two different things, and conflating them is the failure mode
this section exists to prevent.

1. **The action witness** — the game actually performing the effect. A submitted command, an
   HTTP acknowledgement, or a model response is *not* this. The game state must change in the
   way the action intends, observed from the game side.
2. **The settlement witness** — an independent observation that the action settled, not a
   restatement of the submission. If settlement cannot be independently observed, record an
   **explicit failure** instead. An unobserved settlement is not a successful one, and a
   recorded trajectory is not proof of semantic correctness.

## Procedure

1. Record the component pins above. Do not proceed on a partially pinned run.
2. Provision a **disposable** game environment. This recipe is written for a disposable
   environment precisely so a failed run costs nothing; it is not a recipe for a valued save.
3. Start the served process with the environment above. Confirm readiness from the process
   itself rather than assuming a bound port means ready.
4. Submit the authored observe → decide → execute_action → terminal graph. Record the exact
   graph digest and node order. AC1 requires that changing the graph changes execution, so
   record both the graph you ran and the digest you expected.
5. Capture the action witness and the settlement witness, or the explicit failure.
6. Record cleanup. An un-cleaned disposable environment invalidates the run.
7. Have the record independently reviewed before it is treated as AC5 evidence.

## What this recipe does not establish

- No native run has been performed. Every statement about runtime behaviour here is
  `proposed` or `source-derived` from the served composition, never `confirmed`.
- Steps 1–3 are preparation. Steps 4–7 are the authorized execution and its evidence, and they
  are out of scope until a host is authorized.
- Passing this recipe does not close #94. AC5 additionally requires that the recorded evidence
  be independently reviewed, and #94 remains open on its T2 as well.
- Synthetic or CI results do not substitute for any step above, and a bounded non-native
  bridge is not the native witness: the repository's own served composition labels its
  raw-wire bridge test-only and non-provider-calling, and an operator run must not carry that
  limitation silently.
