# Selecting a System One model

The `sts2-jev-bridge` asks one typed question of a System One provider and returns one terminal
decision. It accepts a user-selected model identifier and sends it unchanged. There is no model
allowlist and no fallback to another model. Omitting `--model` selects `jev-latest`.

Build and inspect the selected configuration without making a provider request:

```sh
cargo build --locked --package sts2-harness --bin sts2-jev-bridge
target/debug/sts2-jev-bridge --model jev-1.13.0 --describe
```

`--describe` reports requested configuration. It is not availability, and it is not evidence that
inference used that model. Identifiers must be nonempty, at most 240 UTF-8 bytes, contain no
whitespace or control characters, and must not start with `-`. Unknown, duplicate, or malformed
options fail before input is read or a process is started.

## The transport executable

This bridge does not open the network. It spawns an operator-owned executable, named by
`--transport` as an absolute path, and that executable performs exactly one HTTPS exchange. The
reasons are recorded in [ADR 0053](decisions/0053-system-one-provider-lane.md): this workspace has
no TLS stack, the provider publishes no Rust SDK, and everything that has to stay reviewable —
request construction, bounds, catalog membership, the confidence gate, the decision shape — stays
inside the digest-pinned Rust binary either way.

The contract is one exchange over standard streams, and nothing else:

1. The transport reads one request body, which is JSON, on standard input until end of file.
2. It performs one `POST https://api.typesafe.ai/v1/systemone` with
   `Authorization: Bearer $TYPESAFE_API_KEY` and `Content-Type: application/json`.
3. It writes the response body — the JSON, not an HTTP message — on standard output and exits `0`.
4. Any failure is a nonzero exit. The bridge treats a nonzero exit, an unreadable reply, and a reply
   above `128 KiB` identically: no decision, nonzero exit, nothing written to standard output.

The transport never sees the action catalog, never sees the decision, and is never given the
credential as an argument. A `429` or `529` from the provider is a transport failure like any other;
retry and backoff policy belongs to the caller, not to the bridge.

A minimal reference transport, which is an operator artifact and not part of this repository:

```python
#!/usr/bin/env python3
"""Reads one request body on stdin, posts it, writes the response body on stdout."""
import os
import sys
import urllib.request

body = sys.stdin.buffer.read()
request = urllib.request.Request(
    "https://api.typesafe.ai/v1/systemone",
    data=body,
    headers={
        "Authorization": "Bearer " + os.environ["TYPESAFE_API_KEY"],
        "Content-Type": "application/json",
    },
    method="POST",
)
with urllib.request.urlopen(request, timeout=60) as response:
    sys.stdout.buffer.write(response.read())
```

The vendor also publishes JavaScript and Python SDKs; either may back a transport. Whatever is used,
record its SHA-256 with the experiment configuration: a run on this lane has two digests, the bridge
and the transport, and only the first is verified by the runtime.

## Runtime configuration

For an already configured, authorized runtime-v3 combat fixture:

```sh
export STS2_PROVIDER_KIND=typesafe-jev
export STS2_EXO_ADMISSION=legacy
export STS2_COMBAT_DEMO=true
export STS2_EXO_BRIDGE_BINARY="$(pwd)/target/debug/sts2-jev-bridge"
export STS2_EXO_REVISION="$(sha256sum "$STS2_EXO_BRIDGE_BINARY" | cut -d ' ' -f 1)"
export STS2_EXO_BRIDGE_ARGS_JSON='["--model","jev-1.13.0","--transport","/opt/providers/systemone"]'
export STS2_EXO_INHERITED_ENV_JSON='["TYPESAFE_API_KEY"]'
```

`STS2_EXO_ADMISSION=legacy` is required and explicit. This bridge speaks the raw request shape, so
the run acknowledges an un-admitted bridge rather than claiming the reviewed envelope admission of
[ADR 0031](decisions/0031-runtime-exo-admission-gate.md). Without it the runtime defaults to the
reviewed `envelope` mode and refuses before any model call, which is intended: the reviewed envelope
binds one provider, host, and route, and this is not that route.

The credential enters by name only. The bridge process is spawned with a cleared environment, and
only the names in `STS2_EXO_INHERITED_ENV_JSON` are passed through, so `TYPESAFE_API_KEY` reaches the
transport and nothing else does. It is never an argument, never captured, and never recorded.

These settings are only the provider portion of a runtime configuration. They do not launch the
game or replace gateway/MCP configuration, leases, or fixture authorization. The existing Astra-only
live-episode mode is unchanged; this kind is not admitted to it.

## What the model is asked, and what comes back

One `choice` question is asked per decision, whose options are exactly the host-generated action
identifiers in the request. The provider returns the chosen identifier, a probability for every
option, and a confidence derived from the shape of that distribution.

- At or above the confidence gate the bridge returns an `action` decision carrying the chosen
  identifier and the confidence as a percentage.
- Below it the bridge returns `reobserve`. An answer whose probability mass is spread is not turned
  into an action.
- A choice outside the supplied catalog is refused, even though the option set structurally
  constrains it. The host is the authority on legality; the provider is not.

The `rationale` in the decision is **bridge-authored** and says so. This provider generates no text,
so the field is composed from the distribution — the chosen option, its probability, the runner-up,
and the confidence. Nothing in a run record is a sentence the model wrote, because the model writes
no sentences.

## Known model weaknesses that this lane does not fix

The vendor documents that the model is not a calculator, that counting error grows with list size,
that it cannot judge numeric proximity, and that unrelated detail in the state degrades every answer
in a call. Those are properties of the model and are not repaired by prompting. The harness-side
answers are `context_control::DerivedExactFacts`, which states the arithmetic beside the state, and
`context_control::OptionSelection`, which narrows the option set before it is asked about. Neither is
wired into this bridge yet.

## Evidence

`confirmed` for what the offline suite exercises: option parsing, request construction, the process
transport contract including its deadline and nonzero-exit refusal, decision mapping, the confidence
gate, and every fail-closed refusal.

One live exchange against `api.typesafe.ai` is `confirmed` for 2026-09-18 and recorded in
[`system-one-live-exchange-20260918.md`](evidence/system-one-live-exchange-20260918.md): the request
was accepted, the answer carried the documented shape, the returned choice was one of the identifiers
that were sent, and the confidence gate fired on a real answer. That exchange used the transport's
TLS stack rather than the bridge's, which is the subject of issue #299.

`unverified` for everything beyond that: no gameplay outcome, no decision-quality claim, and no
sustained run. The offline fixtures speak plain pipes to a local script, so the suite itself remains
evidence about framing and refusals rather than about certificates or the real endpoint.
