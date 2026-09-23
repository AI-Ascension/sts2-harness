# Campaign continuation contract

- Status: Proposed; coordinated consumer migration
- Date: 2026-09-06
- Owner: Harness

## Decision

Consume protocol PR #14 revision `a81ec64d7d14bdb3079b8c7dc3c75e5c88693dfd`.
Its schema digest is `daa216902d3211b9537924105b27e7718dd93dec82969a3c550131a27147c06b`.
The complete MIT artifact, source schema and conformance companions are copied verbatim.
The revision adds `proceed`, `confirm_selection` and `cancel_selection`.
Producer and consumers migrate together; earlier digests remain rejected.

The typed action catalog, Runtime-v3 parser and Exo sandbox admit Proceed, ConfirmSelection and CancelSelection. Each serializes to its neutral argument-free kind. The model may choose one only when the current host catalog advertises it. Existing operation reconciliation remains authoritative.

## Validation and limits

The continuation consumer regression loads each canonical action payload through the real observation parser and checks its typed kind. Extra payload arguments fail closed.

Workspace tests, formatting, Clippy and strict policy pass locally. These are component results.
They do not establish available native controls, live host effects or full campaign completion.
The game-mod owns those separate host evidence requirements. Merge only with the reviewed
producer revision and coordinated consumers.
