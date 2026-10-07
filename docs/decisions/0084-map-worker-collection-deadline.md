# ADR 0084: Bound the worker map snapshot read

Status: accepted for the bounded collector source slice; final validation remains with the campaign owner.

Issue: [AI-Ascension/sts2-harness#97](https://github.com/AI-Ascension/sts2-harness/issues/97).

## Decision

The session worker forwards the runner's existing `map_snapshot` request, including its state, generation, and model-execution identity, to the owner-local runtime port. The runner retains the existing order: admit the observation and legal actions, refresh the game-information binding, read one map snapshot when map context is enabled, validate it against the current state and legal actions, then invoke the decision source. A failed or absent snapshot stops that decision; it does not fall back to an unbound map or invoke the provider without required context.

The map process uses only the fixed `runtime-map-v1` profile and exact seven-tool catalog. It performs the existing initialize and catalog calls followed by one fixed `sts2.map_snapshot` call, and accepts exactly one text content item from that tool. Request fields come from runtime configuration and the admitted state generation; caller URLs, tool names, templates, and arbitrary arguments are not accepted. The response's `session_id` remains the gateway session identity; the distinct MCP session ID is sent only as the request's `mcp_session_id` argument.

Before converting the response for the existing decision port, the collector uses the pinned `sts2-protocol` decoder and response validator. That decoder enforces the 256 KiB raw-envelope bound, duplicate-key rejection, typed schema, graph constraints, metadata, and provenance. The MCP process separately retains its 512 KiB outer-frame bound for the escaped JSON-RPC content wrapper. The collector then checks correlation `3`, instance, gateway session, lease, epoch, and requested generation against the call's current configuration.

One monotonic ten-second deadline begins before process startup and covers startup, all three RPCs, decoding, graceful close, and cleanup. Each map-profile RPC is capped at the smaller of its existing five-second timeout and the time remaining after reserving 1.25 seconds for close and reap. The typed decoder is bounded to 256 KiB and a post-validation deadline check refuses a late result; synchronous parsing cannot be preempted. The close wait is at most one second and the force-reap wait at most 250 milliseconds, both capped by the same absolute deadline. Cleanup always signals the owned child, including when no wait time remains; an expired deadline reports cleanup failure and does not claim the child was reaped. Synchronous operating-system process creation itself cannot be interrupted, so a late spawn is followed by no RPC and deadline-bounded cleanup.

## Scope and limits

This change supplies the worker-to-port collection path for the existing map-context option. It does not execute recipe v2 definitions, persist collection receipts, cache results, retry map reads, alter recipe v1 or ordinary gameplay RPCs, or change transition-wait timing. It does not establish native-game, provider, Studio, deployment, or end-to-end issue acceptance. The map-context option remains the only activation boundary in this source slice.

Production-seam regressions cover worker forwarding exactly once, runner ordering and fail-closed behavior, strict decoder rejection of malformed, duplicate-key, oversized, wrong-shape, unknown-field, invalid-graph, and identity-mismatched envelopes, and deadline caps for RPC and cleanup waits. These tests were authored for root-owned validation and have not been executed in this worker task.
