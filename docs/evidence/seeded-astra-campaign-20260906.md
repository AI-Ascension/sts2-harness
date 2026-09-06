# Confirmed uninterrupted seeded Astra campaign, 2026-09-06

An authorized visible Windows v0.107.1 fixture ran a fresh practice campaign with
seed `AIASCENSIONV3FULL1`, using OpenAI Astra model `gpt-6-astra`. The first model
action was `start_run` from Setup. The invocation had no continuation source and
no replay input. It reached Defeat at zero HP on floor 17, and the harness exited zero.
This is a complete setup-to-defeat episode, not a victory or completion of every act.

The trajectory records 399 episode steps, 334 action decisions, and 333 settled
operations. Those decisions reference 191 distinct model execution identities;
143 decisions reused an existing model plan. One rejected admission did not produce
a settled mutation. Every unknown operation subsequently had a settlement record;
none remained unresolved. The terminal record and visible native defeat screen agree.

| Artifact | SHA-256 |
| --- | --- |
| Complete trajectory | `8d283f475990cf6e7e1a0aa77b02f540c635019e7154c6f2ee92c51c7a6307d8` |
| Native defeat capture | `ea97378d18e2bd2715985db46d50c49fc0eee64ae28ef67d3b5b09c89e2137d5` |
| Installed Windows addon | `8e3c2f1b90593c2d1207f64deebd01ce59e798cdd9d614dccb3da248383ad7e3` |
| Harness binary | `36f51e73d36fd83df8146e879eb52edd3ef5358b68e34a8f3cdc60e8b5289c84` |
| Astra bridge binary | `401ca1100315965c5f93ce68fd6739dbf89db6581ede41ed43226ecd88524695` |
| Gateway binary | `66c66172b0bfed303d5a44efb6fdadaeb49277e126c089ce28b2f4288bc02539` |
| MCP binary | `c4ae2ceb6dc7d1c11e891eebdf812a775fee649581336ee5b350a263a74d2474` |

The isolated profile, addon, launcher, and logs were backed up after the completed
run. Private trajectories and native captures remain outside the repository. The game
was visible through the existing accelerated Windows RDP session, and native game
controls drove actions through harness, MCP, gateway, and mod. No OS input automation
or save restoration drove this episode.

A first fresh replay settled 306 actions before a native selection grid reordered one
otherwise identical card choice. Replay stopped before dispatching the next action.
Selection catalog normalization now preserves identities and multiplicity while allowing
that layout order to differ; player pile order remains significant. Its fresh full replay
subsequently completed successfully, as recorded below.

## Confirmed fresh complete replay

The fresh Windows process replayed all 333 actions through the ordinary MCP/gateway/mod
path in 399 episode steps and exited zero. Its `episode_replay_verified` record binds
the complete source digest above, counts one skipped rejected admission, and reports
zero provider calls. There are 333 replay decisions and no model-decision records.
Every unknown operation was reconciled; no operation remained unresolved.

An independent comparison of the two terminal observations excluded only `generation`
and `state_id`. All remaining fields matched exactly, including the seed, Defeat state,
full player content and ordered card piles, and empty legal-action catalog. The native
capture separately confirms zero HP, floor 17, and the defeat screen. Decorative defeat
wording differs between captures; this is gameplay replay evidence, not pixel equality.

| Replay artifact | SHA-256 |
| --- | --- |
| Complete replay trajectory | `aaa36356d904c4ee578ce316f22c96349775048100ad76aa656b99782d37efc1` |
| Native replay defeat capture | `903b04372ba895ba8924cd6ba7afa4f7c1fec4f4a7bd6ff70fe78063ef2e53f1` |
| Replay harness binary | `98ea00fbcdc21e45eaa8d34201f1972cbb5f3e94b59597394a3487fdeddca856` |

The replay used the same addon, MCP, and gateway binaries as the original campaign.
No profile or save was restored to drive the replay. The recorded source and failed
earlier replay were preserved. The later [Linux campaign and replay evidence](linux-seeded-campaign-20260906.md)
records its separate controller-restart and card-rebinding scope. Other seeds and paths,
a model-played campaign win, and post-reboot GPU readiness remain separate unverified work.
