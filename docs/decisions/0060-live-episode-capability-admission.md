# ADR 0060: Live-episode admission from the provider lane's declared capability

Status: accepted for the Runtime-v3 provider admission in `crates/harness`. It authorises the Exo
lane for a live episode once the reviewed envelope has inspected its capability descriptor. It does
not promote the lane by itself: `STS2_LIVE_EPISODE` still has to be declared, and live Exo
connectivity, native-host progression and any gameplay outcome remain `unverified`.

## Context

`RuntimeV3Settings::from_environment` admitted a live episode with raw-string decisions taken inside
one function:

```rust
let provider = optional("STS2_PROVIDER_KIND")?;
let local_bridge = matches!(
    provider.as_deref(),
    Some("ollama" | "openai-astra" | "typesafe-jev")
);
if live_episode && provider.as_deref() != Some("openai-astra") {
    return Err(String::from("Live episode mode requires the OpenAI Astra provider"));
}
```

Three consequences followed, and only the first was intended:

- **The live episode was keyed on a name.** The Exo lane's live capability could not be admitted by
  anything, because the gate asked what the lane was called rather than what it can do. That lane's
  identity *is* inspected — the reviewed envelope binds its package, extension, model, route, tool
  and configuration digests, and its descriptor declares `sts2.exo-capability-v1` — so a lane whose
  capability had been inspected was refused on the spelling of its name. Granting it would have
  required deleting the check, which removes the safeguard rather than generalizing it.
- **An unimplemented name was not refused.** `local_bridge` is false for every name outside that
  list, so an unknown `STS2_PROVIDER_KIND` took the non-bridge branch and ran under the reviewed Exo
  source revision: `OpenAI-Astra`, `exo-envelope`, or any typo selected the reviewed lane instead of
  the lane the operator named. A name that selects a lane nobody implemented is not a spelling the
  runtime may read generously.
- **The decision the gate made was recorded nowhere.** Seven points of use — the bounded replay
  stream, the two idle-transition diagnostics, the MCP RPC failure diagnostic, the negotiated
  catalog trace, the real-peer downstream trace, the primary-error diagnostic, and the lookup
  lane's negotiated-tool dump — re-read `STS2_LIVE_EPISODE` from the ambient environment. Live
  behaviour was therefore a property of the process rather than of the admission, so a second
  live-capable lane would have switched live recording on for every path that inherited the
  variable.

## Decision

Live-episode admission is a declared property of the provider kind, installed once per process.

- `runtime_v3_settings_provider::ProviderKind` names the lanes this runtime implements —
  `openai-astra`, `ollama`, `typesafe-jev`, `exo`, `synthetic` — and carries the two properties that
  decide what a lane may do: `is_local_bridge()`, the digest-pinned executable the operator supplies
  at `STS2_EXO_BRIDGE_BINARY`, and `admits_live_episode()`. Only `openai-astra` and `exo` declare the
  live-episode capability, and an unimplemented name is refused while settings are still being
  assembled.
- `requires_reviewed_envelope()` distinguishes *how* that capability is backed. The Astra lane's
  live claim is the bridge digest and argument vector this module checks; the Exo lane's claim is
  the descriptor only the envelope inspects. `openai-astra` is therefore admitted live on the
  raw-wire lane exactly as it always was, and `exo` is admitted live only under
  `STS2_EXO_ADMISSION=envelope`.
- `runtime_v3_settings::live_admission` resolves the mode from the admitted kind and installs it
  after the whole run is admitted, so a run refused earlier cannot have switched live behaviour on.
  A contradicting second resolution is refused rather than allowed to re-decide the process, and
  re-resolving the same mode is idempotent for a settings assembly that runs twice.
- Every point of use reads `admitted_live_episode()`. Before installation — and for any build that
  reaches a live-only branch without passing through admission — that is `Standard`, never the
  ambient variable. A lane whose live claim rests on its own pinned identity rather than on a
  decision provider, the game-information lookup lane, resolves from its declaration alone.

## Consequences

`crates/harness/tests/runtime_provider_kind_admission.rs` drives the real binary and pins what the
admission grants, each case in a process of its own: an unimplemented name is refused before the
peer session, the provider bridge or the durable execution store exists; `openai-astra` keeps the
live episode it already had; `exo` is refused a live episode on `legacy` and admitted one under
`envelope`; `ollama`, `typesafe-jev` and `synthetic` are refused a live episode by capability; and a
live episode that names no kind at all is refused. Inside the binary, a declaration on its own is
also shown not to admit live behaviour, and the installed record — not the variable — is shown to
decide it.

What this record does not claim:

- The Exo lane's admitted argument vector for a live episode is undecided. `exo` is not a local
  bridge, so no `STS2_EXO_BRIDGE_ARGS_JSON` allowlist entry backs it and its argv shape remains open
  work on #145.
- Live Exo connectivity, native-host progression and any gameplay outcome remain `unverified`. This
  record admits a lane; it evidences no run.
- The durability axes of #145 are untouched: this record changes no durable record, schema or wire
  contract.
