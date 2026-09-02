# `runtime-v1` component integration evidence

- Date: 2026-09-02
- Evidence level: `confirmed` component-network integration; host/runtime compatibility remains `unverified`
- Profile: `runtime-v1` / `runtime-v1-mcp`
- Canonical schema digest: `a76086d7a68668fd4cff53999369d2b450b0d6623827393882f458f2aa1f93eb`
- Scope: real harness coordinator, real MCP process, real attached gateway binary, and a short-lived synthetic loopback downstream

## Setup

The run used synthetic identifiers and operator-provided environment values for separate gateway and
downstream bearer tokens. The synthetic downstream implemented only the bounded health/state/action
HTTP shapes needed by this slice. It contained no STS2 files, host assemblies, save, profile, model,
provider, or retained credential.

The gateway was started in attached single-instance mode. The harness spawned the MCP binary rather
than calling a library or a game endpoint. All three product processes used the checked-in runtime
code from their isolated worktrees; no repository history or publication operation was performed.

## Required sequence and observed result

1. Gateway allocation returned the configured instance, caller, session, lease, and epoch.
2. A direct state request with the correct identity but wrong lease epoch was rejected by the
   gateway with HTTP `409` and `lease_fence_rejected` before downstream forwarding.
3. MCP initialize and `tools/list` succeeded, and the catalog revision was `runtime-v1-mcp`.
4. `get_state` returned generation `0` with the host-ready observation.
5. `show_runtime_probe` at generation `0` returned `accepted`, generation `1`,
   `overlay_visible: true`, and `{kind: status_overlay_visible, generation: 1}`.
6. Repeating the action at generation `0` returned `rejected` with
   `sts2.game-mod/stale_generation`.
7. A fresh `get_state` returned generation `1`, action count `1`, and the visible overlay.
8. MCP stdin closed cleanly and the gateway lease release succeeded.

The emitted sanitized trace was:

~~~json
{"protocol":"runtime-v1","instance_id":"instance-1","session_id":"session-1","before_generation":0,"after_generation":1,"accepted_effect":{"kind":"status_overlay_visible","generation":1},"stale_rejection":"sts2.game-mod/stale_generation","observation":{"host_ready":true,"overlay_visible":true,"screen":"host","action_count":1}}
~~~

## Limits

This proves the coordinator/MCP/gateway network path and response mapping only. It does not prove the
native mod listener, managed callback, Godot main-thread dispatch, actual STS2 host state, visible
effect in the game window, gameplay mutation, process supervision/restart, or disposable game-profile
cleanup. Those gates remain `unverified` and require separate authorization and exact host evidence.
