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

## The transport

This bridge opens the network itself. It performs exactly one HTTPS exchange with a pinned
`rustls` client whose trust anchors are compiled in from `webpki-roots`, and there is no
`--transport` option and no operator-supplied executable. This is the migration
[ADR 0053](decisions/0053-system-one-provider-lane.md) named as the better end state: a run now
carries one digest-pinned artifact instead of two, and the runtime verifies the one artifact that
performs the exchange.

The contract is one exchange, and nothing else:

1. The bridge builds one `POST https://api.typesafe.ai/v1/systemone` with
   `Authorization: Bearer $TYPESAFE_API_KEY` and `Content-Type: application/json`.
2. The certificate chain and the hostname are verified against the pinned roots for
   `api.typesafe.ai` only. A peer naming another host fails in the handshake, before the request
   is sent.
3. Only a `200` response is read. A non-`200` — including a `429` and a `529` — an unverifiable
   certificate, a body above `128 KiB`, a body that disagrees with its declared `Content-Length` or
   `chunked` framing, a TCP close without a TLS `close_notify`, and a missed deadline are all the
   same outcome: no decision, nonzero exit, nothing written to standard output. The `128 KiB` bound
   is on the body itself, with headers bounded separately at `8 KiB`.
4. There is no retry and no fallback to plaintext. A retry policy belongs to the caller, and a
   downgrade would make a run's evidence unable to say what terminated the connection.

The transport layer never sees the action catalog, never sees the decision, and is never given the
credential as an argument. The credential is read by name from the inherited environment for the
one `POST` and is not written to a record, a log, a capture, or a `--describe` output. A credential
that is missing, empty, longer than `4096` bytes, or carries a control character or a space is
refused **before** any socket is opened, so a misconfigured run never puts a request in front of
the provider and cannot inject a header of its own.

The trust anchor is `webpki-roots`, pinned by version in `Cargo.toml` and carried in `Cargo.lock`.
It is compiled into the binary rather than read from the host's system store, so a run record can
state which roots verified the peer instead of saying "it used the system roots".

### On the pure-Rust requirement

The original issue asked for a pure-Rust TLS client with no C or assembly crypto backend. `rustls`
0.23 declares exactly two crypto providers, `crypto::ring` and `crypto::aws_lc_rs`, and **both
compile C and/or assembly** (17 `.c` and 73 `.S` files in `ring` 0.17.14; 662 `.c`/`.h` and 849
`.S` in `aws-lc-sys` 0.39.0). There is no pure-Rust option, so that requirement cannot be met as
written by any `rustls` configuration. This
workspace already compiles C — `rusqlite` is used with `bundled` sqlite3, and the `Rust quality
gates` job already installs a C toolchain for exactly that reason — so the honest reading is that
the constraint asked for something the ecosystem does not offer, and the choice taken is the
smaller and better-reviewed of the two real backends. `ring` was taken over `aws-lc-rs` because it
adds about 1m36s of release compile against the former's 3m33s, and neither needs the CI timeout
raised. See ADR 0053.

## Runtime configuration

For an already configured, authorized runtime-v3 combat fixture:

```sh
export STS2_PROVIDER_KIND=typesafe-jev
export STS2_EXO_ADMISSION=legacy
export STS2_COMBAT_DEMO=true
export STS2_EXO_BRIDGE_BINARY="$(pwd)/target/debug/sts2-jev-bridge"
export STS2_EXO_REVISION="$(sha256sum "$STS2_EXO_BRIDGE_BINARY" | cut -d ' ' -f 1)"
export STS2_EXO_BRIDGE_ARGS_JSON='["--model","jev-1.13.0"]'
export STS2_EXO_INHERITED_ENV_JSON='["TYPESAFE_API_KEY"]'
```

`STS2_EXO_ADMISSION=legacy` is required and explicit. This bridge speaks the raw request shape, so
the run acknowledges an un-admitted bridge rather than claiming the reviewed envelope admission of
[ADR 0031](decisions/0031-runtime-exo-admission-gate.md). Without it the runtime defaults to the
reviewed `envelope` mode and refuses before any model call, which is intended: the reviewed envelope
binds one provider, host, and route, and this is not that route.

The credential enters by name only. The bridge process is spawned with a cleared environment, and
only the names in `STS2_EXO_INHERITED_ENV_JSON` are passed through, so `TYPESAFE_API_KEY` reaches the
bridge and nothing else does. It is never an argument, never captured, and never recorded.

The argument vector lost `--transport`: a configuration that still carries it is refused at
admission rather than silently narrowed, so an operator upgrading an existing
`STS2_EXO_BRIDGE_ARGS_JSON` is told their four-element vector no longer admits instead of having
the pair dropped and the run proceeding as though the configuration they reviewed were the one that
ran.

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

When two options carry the same probability the answer cannot say which of them it meant, and the
bridge does not decide for it. The decision is unchanged: the provider's own `choice` is what is
returned, the gate is applied to the confidence the provider stated, and a tie at or above the gate
is not converted into a second guess. The tie is visible only in the `rationale`, which names one of
the tied options as the runner-up. That selection is fixed by the probability map rather than by the
answer: the map is a sorted map and the bridge reads it in its own stable order, so the identifier
that sorts last is the one named, the same answer always produces the same rationale, and the order
the provider happened to write its keys in cannot change it. Naming one of two equally likely
options as the runner-up is not a claim that the other is less likely — the probabilities printed
beside both identifiers are the evidence, and neither the decision nor the rationale asserts more
than the distribution carries.

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

That artifact is checked against the mapper rather than trusted: `systemone_evidence_tests.rs`
recomputes the filed decision, whole, and both published digests from the committed bytes, so a
transcribed decision cannot drift from the response beside it. For the next exchange the bridge can
write the record itself — `sts2-jev-bridge --record` prints one object carrying `schema`,
`provider_call`, `provider_request`, `provider_response` and `decision`, so an operator publishes the
bridge's own output instead of copying two fields by hand. The flag changes nothing about the default
output, and the runtime lane does not admit it, because that lane reads this executable's stdout as
the decision itself.

`unverified` for everything beyond that: no gameplay outcome, no decision-quality claim, and no
sustained run. The offline fixtures speak plain pipes to a local script, so the suite itself remains
evidence about framing and refusals rather than about certificates or the real endpoint.
