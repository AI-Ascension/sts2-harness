# ADR 0053: System One provider lane and its transport

## Status

Proposed for issue [#286](https://github.com/AI-Ascension/sts2-harness/issues/286). No executable
lands with this decision; the bridge that implements it is
[#288](https://github.com/AI-Ascension/sts2-harness/issues/288) and is reviewable separately.

The vendor facts below are `source-derived` from the published documentation retrieved on
2026-09-18. The repository facts are `confirmed` against this revision. The recommendation itself is
`proposed`: no run in this repository has called this provider.

## Context

Every provider lane the harness owns today assumes a model that generates text.
`sts2-ollama-bridge` constrains a chat model with a JSON schema and parses JSON back out of a
message; `sts2-astra-bridge` spawns a CLI and parses its event stream. Both then validate that the
returned `action_id` is in the supplied catalog.

A "System One" provider inverts that shape. TypeSafe's Jev evaluates typed *questions* against a
*state* and returns structured answers; it does not generate text, and the documentation describes
generation as slow and ineffective for it. The published API is a single endpoint,
`POST https://api.typesafe.ai/v1/systemone`, authenticated with a bearer token, taking `state`,
`model`, and a map of named `questions`, and returning `answers` keyed by question name plus token
`usage`. A `choice` question carries a `criteria` map of option identifier to description and
returns the selected identifier, a probability across every option, and a calibrated `confidence`.

That maps onto this repository's existing bridge request more directly than a chat model does. The
request already carries `legal_action_ids`: a closed, host-generated catalog bounded at 256 entries
in the declared model-view vocabulary. "Pick one of these identifiers" is the provider's native
primitive rather than a constraint layered onto a text generator, and the returned distribution is
evidence the text lanes cannot produce.

Three properties of the provider shape the decision:

- **Published limits.** `jev-1.13.0`, with aliases `jev-latest` and `jev-preview`; `64k` tokens per
  request, of which `32k` covers the state plus the longest question; `1,200` requests per minute;
  `250,000` tokens per second; `$0.042` per million input tokens with output tokens not charged.
  At roughly eight thousand tokens per decision, a thousand-decision run costs on the order of
  `$0.34`. Rate and cost are not constraints on a turn-based game loop.
- **Published weaknesses.** The vendor documents that the model is not a calculator, that counting
  error grows with list size, that it cannot judge numeric proximity, that it treats dates as text,
  and that unrelated detail in the state degrades every answer in the call. These are design
  constraints on the *state*, not prompt-engineering problems, and they are why issues
  [#287](https://github.com/AI-Ascension/sts2-harness/issues/287) and
  [#290](https://github.com/AI-Ascension/sts2-harness/issues/290) exist: move arithmetic into code
  and narrow the option set before asking.
- **No Rust SDK.** The published SDKs are JavaScript and Python. A Rust bridge speaks the HTTP API
  directly.

Two repository facts constrain how it can be admitted:

- **The reviewed envelope cannot take it as-is.** [ADR 0017](0017-exo-executor-bridge-contract.md)
  binds the admitted route to `provider = openai`, the reviewed host `api.openai.com`, and a model
  binding that `responses_capable` accepts, and fails closed on every other provider, host, and
  route. That behaviour is correct and stays. A System One provider satisfies none of those axes.
- **There was no TLS stack in the workspace when this was first decided.** `Cargo.lock` at that
  revision contained no `rustls`, `native-tls`, `openssl`, `reqwest`, `hyper`, or `ureq`. The Ollama
  bridge writes plaintext HTTP/1.1 to `127.0.0.1:11434`; the Astra bridge spawns a CLI.
  `api.typesafe.ai` is HTTPS, so this is the first lane that needs an outbound TLS client, and the
  transport decision below is what introduced one.

## Decision

### The lane

A System One provider is admitted as a distinct **local bridge kind** (`typesafe-jev`) on the same
legacy lane that `ollama` and `openai-astra` use, with every existing guard intact: the SHA-256
digest pin computed from the bytes at `STS2_EXO_BRIDGE_BINARY`, the per-provider argument
allowlist, and the explicit combat-demo gate. It is not promoted to live-episode mode, which stays
Astra-only, and it is not admitted to the reviewed envelope.

Admitting it to the reviewed envelope later requires three things that are deliberately out of
scope here: a route axis for a non-Responses provider, a capability predicate generalized past
`responses_capable`, and a question-set digest standing in for `prompt_digest`. The last of these is
a better fit for the envelope than the text lanes are: a question set is data, so its digest is
exact, whereas a prompt digest binds a string that says nothing about what was asked.

### The transport

**The bridge owns the contract and the transport.** The bridge serializes the request, performs
exactly one HTTPS exchange with a pinned `rustls` client, and validates everything it reads. There
is no second executable: `--transport` is gone and the runtime admits `["--model", MODEL]`.

This supersedes the first version of this decision, which spawned an operator-owned transport
executable, and it is taken for the reason that version recorded as its migration path. A run
carried two digests and the runtime verified one; now one self-contained artifact carries the
exchange and is the artifact the runtime verifies. The operator no longer installs and pins a
second thing beside the bridge.

Every decision that matters — request construction, catalog membership, the confidence gate,
bounds, fail-closed refusals — stays inside the digest-pinned Rust binary, and only TLS and socket
handling moved into a dependency rather than out of a process boundary.

The offline tests remain honest without a fake transport: a TLS server in CI would not be a
deterministic fixture, so the response classes the provider can return are asserted in-process
against bytes, and the credential assertions run at the real process boundary.

**What this cost the compiled-bridge lane, stated rather than absorbed.**
`experiments/jev-evaluation/compiled-bridge.integration.mjs` drove the real compiled binary
end to end by staging a socket-free synthetic `--transport` beside it, and asserted the paired
runner's accounting of provider attempts across all eight response classes. Since the bridge
performs the exchange itself, that staging became inert: host, port, and root store are
compile-time constants with no injection seam, so a synthetic peer cannot satisfy the binary, and
a local server presenting a substituted CA is refused by design. Nine of its eleven cases could no
longer reach a successful answer and were retired rather than left failing; the two that are
genuinely provider-independent — the runner's digest admission of the real binary, and the
bridge's pre-exchange refusals — are kept.

The response-class coverage those nine carried is not dropped, it moves: framing, status,
`chunked`, and `close_notify` are asserted against literal bytes in `jev_tls_transport_tests.rs`,
catalog and confidence refusals in `sts2_jev_bridge_tests.rs`, and the ordering that only a real
peer can show — handshake completing before a request is written, and a peer that never answers
being refused on the deadline — in `jev_tls_transport_loopback_tests.rs` against a loopback TLS
server. The `transport` entry the manifest schema used to require is gone: `runner-contract.mjs`
is at `ascension.jev-paired-runner.v2`, the field is in neither the admitted key set nor any
verification or report path, and a manifest that still carries one is refused by the name
`runner_retired_transport` rather than having the field quietly dropped.

The gap this leaves is real and is not papered over: **no automated lane exercises a provider
exchange end to end through the compiled binary.** Restoring one would mean adding a test-only
seam to production TLS code — a feature-gated host and root override, or an injected transport —
which widens the production surface to serve a test and needs its own review. Until that is
decided on its own merits, the compiled lane proves admission and pre-exchange refusal, and the
in-process and loopback suites prove the exchange.

**The crypto backend is `ring`, and the "pure Rust" condition could not be met.** `rustls` 0.23
declares exactly two crypto providers and no others — `crypto::ring` and `crypto::aws_lc_rs`,
each behind its own feature, with `custom-provider` reserved for a provider the caller supplies.
Both compile C and/or assembly: `ring` 0.17.14 carries 17 `.c` and 73 `.S` files, and
`aws-lc-sys` 0.39.0 carries 662 `.c`/`.h` and 849 `.S` files. No `rustls` configuration is free of
compiled code, so a literal pure-Rust requirement is unsatisfiable rather than merely awkward, and
this is stated as a deviation rather than presented as compliance. `ring` was taken over
`aws-lc-rs` because it adds about 1m36s of release compile against the latter's 3m33s, and this
workspace already compiles C regardless — `rusqlite` is used with `bundled` sqlite3, and the
`Rust quality gates` job installs a C toolchain for that dependency alone.

Unsafe code inside these dependencies is not covered by the workspace `unsafe_code = "forbid"`
lint, which applies to workspace members; that distinction is stated here rather than left for a
reviewer to work out.

### The credential

The bearer token stays with the operator and enters by name only, through the existing isolated
environment allowlist, consistent with [ADR 0001](0001-harness-ownership-and-dependency-boundary.md)
stating that the harness owns provider-neutral ports "without owning provider SDKs or credentials".
It is never written to a record, log, capture, error, or `--describe` output.

### The rationale field

The bridge contract requires a nonempty `rationale`. Jev emits no text, so the bridge composes that
field from the returned distribution — the chosen identifier, its probability, the runner-up, and the
confidence — and labels it as bridge-authored evidence. A natural-language rationale presented as
model reasoning would be a fabricated artifact, which the contribution rules forbid.

## Alternatives considered

**A pinned pure-Rust TLS client inside the bridge** (`rustls` plus a root store). **Taken.** The
three grounds on which this was first rejected have all been answered: the lane now has a recorded
live run (`docs/evidence/system-one-live-exchange-20260918.md`); the CI budget objection is moot
because the `rust` job's `timeout-minutes` is 25, not the 10 this decision recorded, and recent
runs take about 8.5 minutes; and the dependency cost is a dozen-odd pinned crates with no native
build step beyond the C toolchain the workspace already requires for `rusqlite`.

The one ground that did not survive contact with the ecosystem is the "pure Rust" wording itself.
Neither `rustls` backend satisfies it, so it was never a choice between backends but a condition no
candidate met. `ring` was taken as the smaller and better-reviewed of the two real options.

**An operator-run loopback TLS terminator**, leaving the bridge byte-identical in shape to the
Ollama one. Rejected: it puts an unpinned network element inside the trust boundary and produces a
run whose evidence cannot state what terminated the connection.

**A full sidecar that also makes the decision**, using the vendor SDK to build questions and
interpret answers. Rejected: it moves catalog membership, bounds, and the confidence gate outside
the pinned binary, which is exactly the part that must stay reviewable here.

## Consequences

- One more provider kind, one more executable, and one more document; no change to the reviewed
  envelope, the game boundary, the gateway, or the MCP path.
- A run using this lane records one digest, the bridge. The retired second artifact is refused at
  admission instead of admitted and ignored.
- The offline test suite covers request construction, response framing, decision mapping, and every
  fail-closed refusal. It does not cover TLS, a live endpoint, or answer quality.

## Evidence and provenance

| Claim | Label | Source |
| --- | --- | --- |
| Endpoint, auth, request and answer schemas | `source-derived` | <https://docs.typesafe.ai/api> |
| Model identifiers, context, rate, and price | `source-derived` | <https://docs.typesafe.ai/models> |
| Confidence semantics and gating guidance | `source-derived` | <https://docs.typesafe.ai/confidence> |
| Documented model weaknesses | `source-derived` | <https://docs.typesafe.ai/model-jaggedness/jev-1.13> |
| Published SDKs are JavaScript and Python only | `source-derived` | <https://docs.typesafe.ai/sdk> |
| No TLS stack in the workspace | `confirmed` | `Cargo.lock` at this revision |
| Reviewed envelope is bound to one provider, host, and route | `confirmed` | ADR 0017 and `exo/contract/preflight.rs` |
| This lane plays Slay the Spire 2 | `unverified` | no run exists |
| Jev decision quality on real combat states | `unverified` | no measurement exists |

## References

- [ADR 0001](0001-harness-ownership-and-dependency-boundary.md) — ownership and dependency boundary.
- [ADR 0017](0017-exo-executor-bridge-contract.md) — pinned executor and bridge contract.
- [ADR 0031](0031-runtime-exo-admission-gate.md) — runtime admission modes.
- [`docs/OLLAMA_MODEL_SELECTION.md`](../OLLAMA_MODEL_SELECTION.md) — the existing local bridge lane.

## Amendment 2026-09-21: live-episode promotion no longer follows the lane's name

The decision above records that this lane is not promoted to live-episode mode, and that live-episode
mode "stays Astra-only". That is no longer the rule:
[ADR 0060](0060-live-episode-capability-admission.md) replaced the name-keyed check with a capability
the provider kind declares, which admits the Exo lane for a live episode through the reviewed
envelope and refuses a `STS2_PROVIDER_KIND` no lane implements. This lane is unchanged by that
record: `typesafe-jev` declares no live-episode capability, so a live episode is refused on it by
capability, and it is still not admitted to the reviewed envelope.
