# ADR 0065: The served composition records through a configured capture surface

Status: accepted for the served managed boundary in `crates/harness`. It records the owner decision
issue [#398](https://github.com/AI-Ascension/sts2-harness/issues/398) leaves open — which sink the
served composition attaches, on what configured surface, and what it retains — and it is ratified when
the change carrying it merges. It does not authorize a provider, native-host, game, deployment or
paid-call lane, and it changes nothing about which provider a managed decision reaches or how many
times it reaches one.

## Context

`#397` (`ea5b828d221cb651493708a8f1afdbb29193280e`) wired the served composition's managed boundary to
`BoundaryCaptureSink::memory_ring()`, and [ADR 0061](0061-served-managed-boundary-recording.md) records
that choice; the durable *receipt* ledger followed in `#411` (`23c29ab`) and `#412` (`5f42363`). Every
literal acceptance criterion of `#398` is met on `main`, and what remains is the item the issue itself
names: "which sink the served composition attaches and on what configured surface ... and what
retention/redaction/path defaults that introduces."

The merged composition carried that decision as a hard-coded call with no operator surface, so no
deployment could select the capture mode or bound its retention. Recording the decision without a
surface would leave it implicit; adding a surface that changed the default would settle a new
production capture mode unilaterally. This record does neither: it records the decision the merged
composition already carries, and makes it operable with the default unchanged.

## Decision

The served composition attaches the sink `STS2_WORKFLOW_CAPTURE_MODE` selects, resolved once at
startup by `crates/harness/src/bin/runtime_support/workflow_service_capture.rs`:

- **Unset** keeps the recording ring the merged composition already attaches: `MemoryCapture`,
  `CaptureMode::Memory`, bounded by the module maxima `MAX_CAPTURE_RECORDS` (128 records) and
  `MAX_CAPTURE_BYTES` (1,048,576 bytes). This is the recorded default, not a new one; the served
  binary's behavior is unchanged when the surface is unset.
- **`metadata`** records the ring's lifecycle and digests without the content bytes.
- **`off`** attaches `BoundaryCaptureSink::disabled()`, so a served managed decision is refused with
  the capability error `prepared_boundary_unsupported` *before* any provider write rather than
  published for a boundary nothing observed.
- **`STS2_WORKFLOW_CAPTURE_RECORDS`** and **`STS2_WORKFLOW_CAPTURE_BYTES`** size the ring. Each must be
  a positive integer no greater than its module maximum; an unset bound is that maximum.
- Every **unrecognised mode**, **empty** value, **non-numeric or out-of-range bound**, and a bound
  combined with `off` is refused at startup rather than silently downgraded, so a misconfigured
  deployment cannot lose a boundary it believes it recorded.

## Retention and redaction defaults

- Retention is bounded and in memory: the ring drops its oldest record once full, stores the exact
  application bytes at `memory`, only digests and lifecycle at `metadata`, and nothing at `off`. No new
  surface writes captured bytes to disk.
- Redaction is unchanged from `context_capture`: content is recorded only at `memory`, `private` mode
  is not offered on this surface (it requires an approved encrypted vault), and no provider-internal
  conversation, hidden context or effective provider window is ever claimed.
- Recorded bytes do **not** survive a process restart. The durable *receipt* ledger is the separate,
  opt-in surface: the served binary attaches it when `STS2_WORKFLOW_DISPATCH_LEDGER` names an image
  path (`#411`/`#412`), and a restart then reloads the receipts and refuses a second write. Durability
  of the recorded bytes themselves remains with
  [sts2-harness#145](https://github.com/AI-Ascension/sts2-harness/issues/145).

## The Ollama boundary stays an explicit residual

The library advertises two exact boundaries, `exo` → `adapter.cli_input` and `ollama` →
`adapter.http_body` (`ADVERTISED_EXACT_ADAPTERS`). The served composition writes and records only the
Exo boundary; the Ollama `HttpBody` boundary is not recorded by the served composition. This record
keeps that as an explicit residual rather than a recorded boundary (`#108` remains the delivery owner
of the boundary set), and the test
`the_served_composition_records_only_the_advertised_exo_boundary` pins the served recording scope so no
exactness is implied for a boundary the composition did not observe. The standalone
`sts2-ollama-bridge` binary records through whatever `CapturePort` its caller supplies and defaults to
`NoopCapture`; it publishes no exactness claim on its own.

## Consequences

- The served capture mode and its retention bounds are now an operator decision on a configured
  surface, recorded here, with the merged default preserved rather than replaced.
- A misconfigured capture surface fails closed: the served binary refuses to start rather than
  silently recording nothing or recording more than it was told to.
- The residual list is unchanged from ADR 0061 except that the sink selection is no longer implicit:
  recorded bytes do not survive a restart, and the Ollama boundary is not recorded.

## Evidence

| Claim | Label | Source |
| --- | --- | --- |
| The merged composition attaches the recording ring | `confirmed` | `workflow_service.rs`, `context_ports::BoundaryCaptureSink::memory_ring` |
| The served default is unchanged when the surface is unset | `confirmed` | `workflow_service_capture.rs`, `workflow_service_capture_tests.rs` |
| An explicit `off`/unrecordable sink refuses before any write | `confirmed` | `production_boundary_tests.rs`, `production_boundary_durable_tests.rs` |
| Restart survival is the durable receipt ledger, opt-in | `confirmed` | `production_boundary_durable_tests.rs`, `production_boundary_file_store_tests.rs` |
| The served composition records only the Exo boundary | `confirmed` | `production_served_composition_tests.rs` |

Refs #398. Related: [ADR 0061](0061-served-managed-boundary-recording.md).
