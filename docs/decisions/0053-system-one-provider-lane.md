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
- **There is no TLS stack in the workspace.** `Cargo.lock` at this revision contains no `rustls`,
  `native-tls`, `openssl`, `reqwest`, `hyper`, or `ureq`. The Ollama bridge writes plaintext
  HTTP/1.1 to `127.0.0.1:11434`; the Astra bridge spawns a CLI. `api.typesafe.ai` is HTTPS, so this
  is the first lane that needs an outbound TLS client.

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

**The bridge owns the contract; an operator-owned executable owns the transport.** The bridge
serializes the request, spawns a pinned transport executable, writes the request body to its
standard input, reads the response body from its standard output, and validates everything it reads.
The transport executable performs exactly one HTTPS exchange and does nothing else.

This follows the precedent this repository already set with `sts2-astra-bridge`, which spawns an
operator-owned CLI under a pinned digest and owns only framing, bounds, and decision validation.
It keeps every decision that matters — request construction, catalog membership, the confidence
gate, bounds, fail-closed refusals — inside the digest-pinned Rust binary, and puts only TLS and
socket handling outside it. It also keeps the offline tests honest: a fake transport executable is
a deterministic fixture, where a TLS server in CI would not be.

A reference transport is documented for operators, using the vendor's published Python SDK. It is
not a dependency of this workspace, is not installed by any build, and is identified by its own
digest in the run's configuration.

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

**A pinned pure-Rust TLS client inside the bridge** (`rustls` plus a root store). One digest-pinned
artifact and no second process, which is the better end state. Rejected for now on three grounds:
it adds roughly a dozen pinned transitive crates and their `THIRD_PARTY_NOTICES.md` entries to a
workspace that pins every version exactly; the usual backends compile C or assembly, which the
`10`-minute CI timeout has no measured headroom for; and none of it can be exercised offline, so the
first version would ship a large dependency change whose only justification is a code path CI cannot
run. This remains the migration path once the lane has a recorded run, and the spawned-transport
contract is deliberately narrow enough that swapping it changes one module.

**An operator-run loopback TLS terminator**, leaving the bridge byte-identical in shape to the
Ollama one. Rejected: it puts an unpinned network element inside the trust boundary and produces a
run whose evidence cannot state what terminated the connection.

**A full sidecar that also makes the decision**, using the vendor SDK to build questions and
interpret answers. Rejected: it moves catalog membership, bounds, and the confidence gate outside
the pinned binary, which is exactly the part that must stay reviewable here.

## Consequences

- One more provider kind, one more executable, and one more document; no change to the reviewed
  envelope, the game boundary, the gateway, or the MCP path.
- A run using this lane records two digests, the bridge and the transport, rather than one.
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
