# `runtime-v1` live host integration evidence

- Date: 2026-09-02
- Evidence level: `confirmed` bounded end-to-end host trace for one exact disposable profile
- Scope: `sts2-harness-runtime` -> `runtime-v1-mcp` -> `sts2-gateway-runtime` -> game-mod ->
  Godot host
- Canonical schema digest: `a76086d7a68668fd4cff53999369d2b450b0d6623827393882f458f2aa1f93eb`

## Component state

| Component | Source state |
| --- | --- |
| Harness | target HEAD `a1a65ee4d857d0c0ccb63c0ffd4952b084cbe4e2` plus uncommitted runtime changes |
| MCP server | target HEAD `ad49b6b3138364882975844d5d0499bce01bbb2f` plus uncommitted runtime changes |
| Gateway | target HEAD `57ebddcc103a1631147346c4d15a6309918c9f09` plus uncommitted runtime changes |
| Game-mod | target HEAD `97f3a2068452d2c1616c531a7dfad51fbd484cac` plus uncommitted runtime changes |

The binaries were run from isolated worktrees. No cross-repository path dependency, provider,
credential, game file, host assembly, save, or generated artifact was added to a repository.

## Host and package

| Field | Observed value |
| --- | --- |
| Game | Slay the Spire 2 `v0.107.1`, release commit `59260271` |
| Host | Windows `10.0.26200`, x86-64; Godot `4.5.1-m.12`; .NET `9.0.7` |
| Host assembly | `sts2.dll`, SHA-256 `a1f9e653f1e28e4076558fee1e60d218619cb7e057b887c6417f62c62c6d7a52` |
| Runtime package | managed `dd37873ca45a8a69058137b661a4c9dc0d7a66cafe6806b90423db12a35e9d46`; native `6d518a3f018c6f2d6553cd765b147eb3a3017457d4e55c665421794b84ba4444`; manifest `a75717d4de14cf87d48b54b15fe45a3c58c231ef7395781b2e780d0a5e8c2985` |

The host assembly was used only as an operator-local build/reference input and was not retained.
The profile baseline was copied to a disposable profile before the host launch. The runtime token
was set inside the Windows launch process; it was not stored in the trace or repository.

## Passing sequence

The Windows host was launched with normal rendering, dummy audio, compatibility rendering, and a
bounded lifetime. The coordinator then allocated the configured instance lease, spawned the real
MCP process, initialized it, verified the `runtime-v1-mcp` catalog, read state at generation 0,
submitted `show_runtime_probe` at generation 0, repeated the stale generation, read fresh state,
closed MCP, and released the gateway lease.

The passing sanitized trace was:

~~~json
{"accepted_effect":{"generation":1,"kind":"status_overlay_visible"},"after_generation":1,"before_generation":0,"instance_id":"instance-1","observation":{"action_count":1,"host_ready":true,"overlay_visible":true,"screen":"host"},"protocol":"runtime-v1","session_id":"session-1","stale_rejection":"sts2.game-mod/stale_generation"}
~~~

The accepted result came from the live managed main-thread callback and its host overlay witness.
A contemporaneous operator window capture also showed the `AI-ASCENSION STS2` live-runtime overlay.

The first immediate coordinator attempt ended before producing a passing tool result during startup;
no success was claimed from it. After the host listener was confirmed ready, the clean retry above
passed all coordinator assertions.

## Cleanup and limits

The test game and gateway were stopped after the passing trace. The original profile and prior
addon-owned file hashes were restored. Disposable profile copies were moved aside rather than
deleted, and no STS2 process, gateway, or runtime listener remained.

This confirms the bounded client-to-host runtime path and the safe probe effect for the recorded
host. It does not confirm gameplay-rule mutation, provider execution, process supervision/restart,
multi-instance scheduling, another host build, another platform, release support, or full game
conformance.
