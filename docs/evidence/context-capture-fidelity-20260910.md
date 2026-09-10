# Context capture fidelity evidence

Date: 2026-09-10

This record compares the accepted harness baseline `fc44d3ef65fefa6d13ecd5f690e5335a6ef60080`
with successor `b3ff9d967f5ae5459da409e1babf6512b014bc6c`. Each bridge was built with the pinned
toolchain and run as a real executable against an isolated synthetic downstream. The fake Codex
CLI recorded stdin, output schema, argv and the bounded JSONL outcome. The fake Ollama server
recorded the complete HTTP request and returned one bounded action. Both bridges made one
loopback-only downstream invocation; no external network or real provider was used.

The machine-readable result is [`context-capture-fidelity-20260910.json`](context-capture-fidelity-20260910.json).
The Astra stdin, schema, stdout, stderr and accounting bytes matched exactly. Astra argv and
working-directory fields matched after replacing generated temporary directory/process values.
The Ollama request, body, stdout, stderr and one-connection count matched exactly. Invalid Astra
event and Ollama HTTP-503 cases produced the same exit status, sanitized output, and zero retries.

The bridge unit tests additionally run the actual fake downstream/server through the prepared
capture paths and assert that retained component bytes equal the bytes consumed at the boundary.
The oracle scripts and raw captures remain outside the repository.
