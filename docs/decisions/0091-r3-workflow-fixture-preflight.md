# ADR 0091: R3 Workflow Fixture Preflight

Status: Proposed for owner review and integration. Source-fixture validation only;
Harness #391 and Context Console #18 remain open for their own acceptance criteria.

## Context

The R3 controller must validate a complete workflow before creating private run
state or starting controlled peers. A generic JSON object check and a few profile
field comparisons do not establish the owning workflow schema or graph validity.
The existing Harness library owns strict decoding and graph validation.

## Decision

Add the non-listening command
`sts2-harness-runtime validate-r3-fixture workflow_definition` to the runtime
binary. Dispatch it before service, worker-namespace and runtime-environment
handling. The command accepts stdin only, reads at most 65,537 bytes, and refuses
documents larger than 65,536 bytes. It uses `WorkflowDefinition`, the existing
strict decoder with depth 32, and the existing graph validator.

This specific R3 lane additionally requires strict mode, `sts2-live-v1` and one
`decide` node with `decision.live.v1` and `context.live.v1`. Other fixture roles
are explicitly unsupported by this command. It starts no runtime, service,
provider or worker and reads no path selected by caller input or environment.

The closed response schema is `console18.r3.workflow-fixture-preflight.v1`.
Its six fields are `schema`, `role`, `accepted`, `evidence_class`,
`source_sha256` and `refusal_code`. Output, including its newline, is limited
to 8,192 bytes. Acceptance exits 0 and returns the exact raw-input SHA-256.
Refusal exits 2 with a sanitized code and no input, identifier, path or decoder
diagnostic. The evidence class is always `source_fixture_validation`.

## Compatibility and evidence

This is an additive binary command; existing service/runtime dispatch, library
APIs, provider contracts, game profiles and credentials are unchanged. No
implementation dependency on another owner is introduced. The response hash
establishes caller-input consistency, not owner provenance or authenticated
runtime admission. Complete seven-role validation, inspected provider-policy
binding, real Harness/Console owner integration and independent restart/recovery
remain separate gates. This decision does not authorize deployment or a native
game/provider run.

## Validation

Shipped-binary regressions cover accepted strict input with poisoned runtime
variables, malformed arguments, duplicate and unknown fields, unsafe numbers,
invalid graphs/profiles and the byte limit. Test process cleanup is separated
from fixture assertions; both files remain below the 250-line test budget.
Formatting, strict policy, focused tests and all-target/all-feature Clippy pass
on the source candidate. Full workspace, integration, hosted review, merge and
post-merge acceptance remain pending. See [the CLI contract](../workflow-fixture-preflight.md).
