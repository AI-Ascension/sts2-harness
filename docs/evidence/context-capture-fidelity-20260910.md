# Context capture fidelity evidence

Date: 2026-09-10

This record compares the accepted harness baseline `fc44d3ef65fefa6d13ecd5f690e5335a6ef60080`
with successor `578595baa10a45fd6fcbf712ed0f59975cf47ef7`. Each bridge was built with the pinned
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
Final Astra captures use the canonical `adapter.cli_input` boundary and final Ollama captures use
the canonical `adapter.http_body` boundary. Bridge-local attempt IDs are generated for every
invocation, and lifecycle records receive a distinct event identity with an explicit parent link;
the parent remains unavailable when no upstream snapshot is supplied. Capture snapshot IDs use
length-prefixed hashing so hyphenated execution and attempt IDs cannot collide.
The oracle scripts and raw captures remain outside the repository.
