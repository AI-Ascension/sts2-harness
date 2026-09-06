# ADR 0010: Bounded legal-catalog refresh

Status: accepted; native recovery verification pending.

A native screen may advance after observation but before the legal-action read. The gateway
already validates the host's compact, correlated read refusal. The MCP adapter preserves that
specific error; the harness adapter admits it only for `sts2.legal_actions`, with `isError: true`,
matching correlation, at most 1024 bytes and exactly `correlation_id`, `error_code`, and `recovery`.
Allowed codes are `stale_generation`, `host_not_configured`, and `host_observation_unavailable`;
recovery must be `reobserve`. Other failures remain fatal.

The adapter maps this refusal to retryable `catalog_reobserve`. Coordinator policy permits at
most three consecutive refreshes before failing with owned cleanup. A successful fresh catalog
and policy choice resets this count. Refresh consumes the existing bounded episode step budget.
No provider choice or game dispatch occurs on the rejected catalog. Mutation reconciliation
continues to use the original operation identity and is unchanged.

Deterministic tests cover changed screen/generation, persistent refusal, unrelated retryable
errors, nonretryable refusal, correlation, shape and cleanup. These establish component behavior;
they do not prove a recovered native screen transition or full campaign.
