# R3 workflow fixture preflight

The owner decision and evidence limits are recorded in
[ADR 0091](decisions/0091-r3-workflow-fixture-preflight.md).

The runtime binary has an effect-free fixture command:

    sts2-harness-runtime validate-r3-fixture workflow_definition

It reads one JSON document from standard input, capped at 65,536 bytes. It does
not accept a path, consult runtime environment variables, start a service, or
create a worker namespace. The owning workflow decoder and graph validator
check the complete typed definition. The R3 lane additionally requires strict
mode, sts2-live-v1, and exactly one decide node named decide bound to
decision.live.v1 and context.live.v1.

The command writes one compact response (at most 8,192 bytes). Success includes
the lowercase SHA-256 of the exact raw input. That digest is source-fixture
consistency metadata only; it does not prove owner provenance, process
admission, game authority, served compatibility, or issue acceptance. Refusals
use stable codes and never include input bytes or decoder diagnostics.

Stable refusal codes are invalid_arguments, unsupported_role,
input_too_large, input_read_failed, invalid_workflow_json, and
invalid_workflow_definition. output_serialization_failed is reserved for the
constant fallback response. Refusal exits with status 2; acceptance exits with
status 0.
