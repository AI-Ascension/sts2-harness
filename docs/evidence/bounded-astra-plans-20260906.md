# Bounded Astra action plans, 2026-09-06

## Confirmed scope

An authorized Windows KVM fixture running STS2 v0.107.1 resumed an isolated standard campaign
at a Byrdonis combat. The visible fullscreen RDP fixture used D3D12 and a 60 FPS cap.
The real OpenAI `gpt-6-astra` bridge returned one plan which produced these four settled moves:

| Model execution | Fresh action ID | Move | Reused model execution |
| --- | --- | --- | --- |
| 1 | `play:6:card:19:none` | Rage | false |
| 1 | `play:7:card:17:enemy:1` | Bash+ | true |
| 1 | `play:8:card:21:enemy:1` | Strike | true |
| 1 | `end:9` | End turn | true |

The runner issued each move through MCP and the gateway, then verified settlement before
dispatching the next. An intermediate unknown receipt was reconciled using the same operation;
the cached plan did not cause a strategic retry. Four distinct settled operation IDs correspond
to the four actions. The immutable 15-record prefix retains each fresh observation and the same
originating model execution ID. This confirms three card plays plus end turn from one real
provider response. It does not establish a wall-clock speedup percentage.

## Artifact identity

| Artifact | SHA-256 |
| --- | --- |
| Astra bridge | `813cb894e99f545de6706bb71efb7cb79bcf087d98d5df33df440dc88c5e7f83` |
| Harness runtime | `4a1f601f8431beb06fcd0da587b5b53ab1ba04c1ba17ded4b050340d6d43f06b` |
| Managed addon | `43eccbd7110c822ea412f844b936e0204f10c8767c7b6335ec3ba59c005a737c` |
| Native addon | `8e46a35ce9ba978275990ff7328f1c48755bea9977ea24c50a8e9dfbb6524d68` |
| Addon manifest | `559e177f0b6e5d82fc44f6b086b1e728353b2f6e437f5e8fae98983d85659984` |
| Pre-run fullscreen capture | `5967c3b7682981f143d909e14b6761900d1183eea3946a080df613eff22d4a85` |
| `first-plan-prefix.jsonl` | `5d48bd65f78376ea7cc13b8d592d771e5b999b0529112546c4d8e35e302002dc` |

Raw observations, screenshots, saves, and provider text remain outside Git. The prefix was
extracted from controlled run `1788678824` before later model calls; its immutable hash is
separate from the still-growing full trajectory. The addon and isolated campaign were backed
up before installation, and all three installed addon hashes were verified. An inspection-only
session caused one pre-provider identity-fence rejection; a fresh process corrected the session
binding before this run. No action or provider retry occurred in that rejected preflight.

## Local validation and limits

The strict policy command, formatting, warnings-denied workspace Clippy, and full workspace
all-target/all-feature locked tests pass for the implementation. `action_plans` verifies current
payload rebinding for combat and shop, explicit settlement gating, rejection invalidation,
new/redrawn cards, changed intents and offers, unavailable next actions, seed changes, parser
bounds, and catalog membership for every proposed action. Episode-runner tests cover settlement
feedback after normal completion, recovery, conflicting receipts, and unresolved outcomes.
The combat replay regression replays separate steps sharing one model execution and rejects
changed semantic payloads. These are synthetic tests, not live shop or fresh-process replay proof.

The campaign continued after the recorded prefix. Full campaign completion, live multi-purchase
shop evidence, Linux gameplay, and fresh-process replay of this new plan remain unverified here.
This document does not promote an incomplete campaign or a build into those stronger claims.
