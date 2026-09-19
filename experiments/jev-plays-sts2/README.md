# Jev plays Slay the Spire 2

The operator scripts that run a TypeSafe System One model (`jev-latest`) against a live game, one
episode at a time, restarting the whole session after each one.

They live here because they were lost once. They existed only in a scratch directory and on the two
guests, an editing mistake truncated all three copies at the same moment, and there was nothing to
restore from. Anything that is the only copy of itself is one mistake from gone.

## What is here

| file | lane |
| --- | --- |
| `jev-loop.sh` | Linux guest. Starts the game through the reviewed launcher, which loads the mod. |
| `jev-loop.ps1` | Windows guest. Starts the game directly, because there is no launcher for it. |
| `systemone_transport.ps1` | Windows transport the bridge spawns for one provider exchange. |

Two more files belong with these and are **not** here: `systemone_transport.py`, the Linux transport,
and `jev-context.py`, which reads what either transport recorded. Both are Python, which `LANG001`
prohibits across this organization's repositories, so they remain operator-local -- which is exactly
the condition that lost the Windows loop. Porting them to a permitted language would close that.

The two loops are kept in step: the same credentials per session, the same gateway and harness
identity, the same bounds. Where they differ it is because the platforms differ, and the comment in
the script says why.

## What they expect

Both read `key.txt` beside the script for the provider credential. It is never an argument, never
logged, and must not be placed in any directory that is served over HTTP during deployment.

Each episode generates its own runtime and gateway credentials from the operating system CSPRNG.
Nothing is read from a saved setting and no token has to be known in advance.

## Reading a run

Every episode writes its own directory under `runs/`, holding the game log, the gateway and harness
output, the outcome, and `jev-context.jsonl` -- every exchange with the provider, including the
state, the instructions, every option with its description, and the answer.

```text
python3 jev-context.py <log>            every exchange, one line each
python3 jev-context.py <log> 2          exchange 2 in full
python3 jev-context.py <log> 2 --state  just the state
python3 jev-context.py <log> --options  per option: times shown, times top, average probability
```

## Settings that were arrived at by measurement, not taste

- **Confidence gate 20.** At 35 the model starts a run and picks a map node but refuses every combat
  answer: its calibrated confidence in combat sits near 0.25, so it abstains and re-observes without
  acting.
- **`STS2_CAMPAIGN_EPISODE`, not `STS2_COMBAT_DEMO`.** The combat demo only acts once the host is
  already in combat and never leaves a menu, so against a freshly launched game it polls an
  unchanging main menu and the model is asked nothing.
- **Barrier 40 x 3s.** The default 8 x 1s expires while the game is still loading and fails the
  episode before the first decision.
- **The gateway is given the harness's own instance identity and a token scope.** Without the scope
  allocation is answered with HTTP 401; with a different instance, HTTP 409.

## Known host-side blockers

- The offered set at a reward carries no information, and a skipped reward is offered again:
  AI-Ascension/sts2-game-mod#171.
- No action continues a saved run, so every episode starts over:
  AI-Ascension/sts2-game-mod#172.
- On Windows the mod refuses to initialise unless its user directory holds a fresh profile baseline:
  AI-Ascension/sts2-game-mod#173.
