# Worker command admission

## Scope

The private `worker_command` policy sits after authenticated transport admission
and after the frozen worker-handoff decoder. It maps the five closed commands to
the existing durable worker APIs while pinning deployment, owner, profile,
release, configuration, worker-boot and watchdog-boot identity to one approved
launch. The decoder remains a syntax boundary; a decoded request is not an
authenticated peer, a capability, or an executable admission.

The transport must supply the capability and owner proof after its protected
peer checks. The command module has no public authentication constructor or
boolean authentication shortcut. Probe is read-only and reports ready only
when the durable control row names the configured boots and is authenticated,
admitting, running, and at a non-zero sequence. Lookup and acknowledgment use
the current authorized worker connection but preserve the complete retained
tuple. Neither command starts or resumes an episode.

The decoder's bounded request timeout is only an upper bound supplied by the
peer. Transport must create one monotonic deadline before authentication and
only shorten it with this value; it must not reset the budget per phase. This
policy module does not perform socket I/O or claim native deadline evidence.

## Approved dispatch preparation

Dispatch preparation accepts an owner-supplied `ApprovedWorkerExecution`, not a
fingerprint assembled from the request. It must contain a validated lineage,
job identity, attempt number and complete execution fingerprint (seed, build,
state, config and provider). The handler checks the request against that
material and the immutable launch binding. It idempotently creates missing
episode/job rows only from the approved material, validates retained rows
exactly, and requires an active episode plus an admitted job for a fresh
handoff. Frame data never supplies seed, provider, fingerprint or executable
configuration.

The repaired worker ledger returns an atomic `Acquired { handoff, permit }` or
`Duplicate(handoff)` outcome. Only the fresh winner produces an accepted result
owning the non-cloneable, non-serializable `WorkerExecutionPermit` together
with its exact tuple and admission context. Duplicate, running, unknown and
terminal rows map to retained status without a new permit; completed and
failed terminals use their corresponding typed dispatch statuses. The runtime
bridge must take that reservation after the response is written and consume it
outside connection handling through `mark_worker_handoff_running`, which
repeats the durable current-control fence immediately before execution.

## Evidence boundary

`worker_command_tests.rs` exercises actual in-memory control and handoff rows,
capability/boot/config mismatches, idempotent preparation, fresh-winner and
duplicate admission, pre-consumption control fencing, durable terminal
projection, and acknowledgment idempotency. These are component tests only.
The protected Windows/Linux transport, opaque peer witness construction,
server wiring, provider/game runtime, and live end-to-end execution remain
unverified until their separate integration gates pass.
