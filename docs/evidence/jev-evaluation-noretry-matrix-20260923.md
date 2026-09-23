# Jev-evaluation Node suite, no-retry matrix — 2026-09-23

Evidence for work package **T4** of issue
[#388](https://github.com/AI-Ascension/sts2-harness/issues/388): run the
`experiments/jev-evaluation` Node suite **without retry masking** and retain first-attempt results.
Every listed execution is a first attempt: each command ran once, its output was retained, and no
shape was re-run to turn a red green.

Source under test: `sts2-harness` `main` `e597e8d5ad5255ceb4d27b24ab7e9b06a3178774`.
Command under test: `node --test experiments/jev-evaluation/*.test.mjs` from the repository root —
the `Run all offline tests` step of `.github/workflows/jev-evaluation.yml`, with `node --test`'s
default file parallelism (the shape CI uses).

Host: 4 cores; the samples below ran while the ambient load average (1-minute) held at 10–31, i.e.
2.5×–8× oversubscribed. The CI matrix pins Node `22.16.0` and `24.18.0`
(`.github/workflows/jev-evaluation.yml`); both exact versions were fetched and driven directly, so
samples A and B use the CI pins rather than the ambient host runtime.

Invocation for the CI-pinned legs (the `Run all offline tests` step, unmodified):

```
/home/agent/tmpwork/jev-node/v24.18.0/bin/node --test experiments/jev-evaluation/*.test.mjs
/home/agent/tmpwork/jev-node/v22.16.0/bin/node --test experiments/jev-evaluation/*.test.mjs
```

## Sample A — Node v24.18.0 (CI pin), 12 first-attempt repetitions

Ran 15:33–15:45Z. **12 of 12 repetitions passed with no first-attempt failure.**

| rep | load (1m) | tests | pass | fail | duration (s) | rc |
| --- | --- | --- | --- | --- | --- | --- |
| 01 | 17.24 | 297 | 297 | 0 | 70.7 | 0 |
| 02 | 24.52 | 297 | 297 | 0 | 75.9 | 0 |
| 03 | 24.80 | 297 | 297 | 0 | 92.1 | 0 |
| 04 | 23.63 | 297 | 297 | 0 | 85.1 | 0 |
| 05 | 20.84 | 297 | 297 | 0 | 100.7 | 0 |
| 06 | 15.77 | 297 | 297 | 0 | 65.5 | 0 |
| 07 | 19.37 | 297 | 297 | 0 | 64.1 | 0 |
| 08 | 24.38 | 297 | 297 | 0 | 51.3 | 0 |
| 09 | 21.22 | 297 | 297 | 0 | 58.1 | 0 |
| 10 | 23.79 | 297 | 297 | 0 | 46.8 | 0 |
| 11 | 22.26 | 297 | 297 | 0 | 47.1 | 0 |
| 12 | 18.28 | 297 | 297 | 0 | 33.9 | 0 |

## Sample B — Node v22.16.0 (CI pin), 12 first-attempt repetitions

Ran 15:17–15:32Z. **11 of 12 repetitions passed; rep 08 failed on its first attempt.**

| rep | load (1m) | tests | pass | fail | duration (s) | rc | first-attempt failure |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 01 | 13.39 | 297 | 297 | 0 | 84.9 | 0 | — |
| 02 | 26.83 | 297 | 297 | 0 | 77.3 | 0 | — |
| 03 | 25.23 | 297 | 297 | 0 | 70.3 | 0 | — |
| 04 | 22.35 | 297 | 297 | 0 | 75.2 | 0 | — |
| 05 | 21.76 | 297 | 297 | 0 | 83.1 | 0 | — |
| 06 | 12.90 | 297 | 297 | 0 | 92.0 | 0 | — |
| 07 | 18.30 | 297 | 297 | 0 | 74.3 | 0 | — |
| 08 | 25.56 | 297 | 296 | 1 | 113.4 | 1 | `low_evidence stays visible without contaminating independent agreement` |
| 09 | 31.30 | 297 | 297 | 0 | 101.6 | 0 | — |
| 10 | 27.89 | 297 | 297 | 0 | 103.9 | 0 | — |
| 11 | 14.33 | 297 | 297 | 0 | 55.8 | 0 | — |
| 12 | 10.46 | 297 | 297 | 0 | 62.0 | 0 | — |

The rep-08 failure is the sibling load sensitivity recorded in
[ADR 0063](../decisions/0063-jev-runner-first-arm-admission.md): at ~6× CPU oversubscription an arm's
`counts.complete` under-counts against the suite's own duration, reding `runner.test.mjs:112` with
`1 !== 2` (test body at `runner.test.mjs:110`). It is a distinct class from the two repaired defects
(arm admission and pipe-teardown resolution), it is not in the files the earlier 4-of-6 sample named,
and it did not reproduce in eight isolated repetitions of the same test — so it is recorded rather
than de-flaked, matching [#388](https://github.com/AI-Ascension/sts2-harness/issues/388) work package
T1's disposition. The same red did not appear in sample A, so it is not claimed to be version-specific.

## Sample C — ambient host Node v24.16.0 (supplementary), 12 first-attempt repetitions

Before the CI-pinned binaries were fetched, a preliminary no-retry sample ran on the ambient host
Node v24.16.0 (15:09–15:26Z, load average 10.1–28.1). **12 of 12 passed 297/297** (durations
70.1 s–121.8 s). It is recorded for completeness only; sample A is the CI-pinned v24 leg.

The 297/297 count matches the post-#392/#396 `main` (296 before #396; #396 adds the escaped-descendant
control).

## Rest of the matrix job

The `Offline evaluation` job's other steps were run from the same revision:

| step | command | result |
| --- | --- | --- |
| Validate experiment syntax | `for file in experiments/jev-evaluation/*.mjs; do node --check "$file"; done` | pass |
| Generate the synthetic demonstration | `node experiments/jev-evaluation/demo.mjs` | pass (rc 0; synthetic only) |
| Verify Linux benchmark and streaming session control | `node --test experiments/jev-plays-sts2/session-modes.test.mjs` | 4/5 — one out-of-scope failure (below) |
| Run compiled paired replay and frozen pilot diagnostics | `bash experiments/jev-evaluation/compiled-ci.sh` | pass — 12/12, rc 0 |

`compiled-ci.sh` built the real `sts2-jev-bridge` (`cargo +1.97.1 build --locked --package
sts2-harness --bin sts2-jev-bridge`) and ran `compiled-bridge.integration.mjs` against it with the
socket-free transport: 12 tests, 12 pass, 0 fail, in 66.8 s.

The `jev-plays-sts2/session-modes.test.mjs` failure is **out of scope** for #388/#394 (it is a
different experiment): `explicit stream resume retains the same native process and profile` expected
the stdout transcript `['harness-first','harness-progress-retained']` and observed `['harness-first']`
after a 9.4 s native run. It is recorded for honesty, not attributed to any change here.

## Boundary

- This establishes the suite's behaviour on **this** host under the recorded load; it is not a CI
  frequency. Samples A and B use the CI matrix's exact Node pins; sample C used the ambient host Node.
- No provider, transport, game, save, or host state was touched: the runner's shipped tests use the
  socket-free fixture only.
- The first-attempt failures above are retained as observed. Neither of the two repaired defects
  (#388 arm admission, #394 pipe-teardown resolution) reds in the samples.
