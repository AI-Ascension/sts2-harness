# Changelog archive: 2026-09-24

This file preserves completed `## Unreleased` history that was moved out of
[`CHANGELOG.md`](../CHANGELOG.md) when the active changelog reached its preferred Markdown size
budget. Entries are unchanged from the revision that introduced them apart from relative link paths,
which are corrected so they resolve from this directory; this file is a verbatim record, not a
supported release or a second normative changelog.

### Archived from CHANGELOG.md

- Derive the **presented option set from the state** instead of offering a provider the whole legal
  catalog. `context_control::OptionSelection` folds catalog entries that are identical under the
  admitted action vocabulary — the same kind aimed at the same target, differing only in which copy
  of a card in hand it names — into one presented option, and records every fold with the option it
  folded into, so a replay can show exactly what the provider was and was not offered. It reports
  `forced` when one action is legal and no question is needed, `single` when the presented options
  fit one question, and `two_stage` above a declared bound, where a kind is asked before an action
  within it. Presented and withheld entries partition the catalog; presented order is catalog order
  and nothing here ranks, scores, or prefers an action. A selection that would leave fewer than two
  options presents the catalog unchanged, because one option is not a question. Affordability is
  deliberately not a withholding rule: the host lists an action only when it is legal, so filtering
  on cost could only ever overrule that authority, and affordability stays in the derived-exact facts
  beside the state. Compatibility: additive; one new module and its re-exports, no change to an
  existing record, route, or digest. Refs #290.

- Admit a **`typesafe-jev` local bridge provider kind**, fail-closed. The kind joins `ollama` and
  `openai-astra` on the legacy local-bridge lane and keeps every guard that lane applies: the
  SHA-256 digest computed from the bytes at `STS2_EXO_BRIDGE_BINARY`, the explicit combat-demo or
  live-episode requirement, and the per-kind argument allowlist. It is not promoted to live-episode
  mode, which stays Astra-only, and not to the reviewed envelope, whose route axes bind one provider
  and host by design. Its admitted argument form is exactly
  `["--model", MODEL, "--transport", PATH]` with an absolute transport path, re-parsed with the same
  parser the bridge executable uses so admission and the executable cannot disagree about what a
  valid invocation is. Argument admission moves out of `runtime_v3_settings.rs`, which was at its
  300-line preferred budget, into `runtime_v3_settings_local_bridge.rs` with the existing Ollama
  shape and its tests. The provider credential needs no code: the bridge process is spawned with a
  cleared environment and only the names in the operator's `STS2_EXO_INHERITED_ENV_JSON` pass
  through, so `TYPESAFE_API_KEY` reaches it by name and never as an argument or a record.
  Compatibility: additive; one new accepted value, no change to an existing shape, record, or digest.
  With the kind admitted and no bridge executable present, the runtime still fails closed at digest
  verification. Refs #285.

- Build a **System One provider request** from a bridge decision request.
  `context_control::build_system_one_request` turns a rendered observation and a presented action
  catalog into the body of one typed question: a `choice` whose option identifiers are exactly the
  catalog, with the request's objective and hard constraints carried in its instruction. It sits
  beside the existing provider projection and is pure — no socket, no environment, no credential —
  so every refusal happens before any of those exist. It refuses an empty, oversized, duplicated, or
  non-printable option set, a malformed model identifier, an empty state, and a state that would
  exceed a conservative byte ceiling for the published 32k-token state-and-question budget, and it
  never truncates a state to make it fit. Serialization is byte-stable, so
  `system_one_questions_digest` gives a run record the honest analogue of the reviewed envelope's
  `prompt_digest`: a question set is data, so its digest states exactly what was asked. Exactly one
  question is asked; the provider evaluates many per call in parallel, but an unconsumed question
  would spend tokens producing a number no code reads. Compatibility: additive; one new module and
  its re-exports, no change to an existing record, route, or digest. Refs #283.

- Fix the **option-selection fold key**, which folded distinct host-listed actions. The key was built
  from a fixed list of seven identity fields, so any action whose identity lived outside that list
  collapsed into a neighbour and was recorded as an intentional duplicate: two different cards aimed
  at one enemy differed only in `card_id`, and `use_potion`, `rest_option`, `select_card`,
  `confirm_selection` and `cancel_selection` carry `potion_id`, `rest_option_id` and `selection_id`,
  which the model-view vocabulary does not declare at all. The key is now built from the whole action
  the host emitted, so it is injective on whatever the host carries, declared here or not. Exactly
  one substitution remains and it is the only thing that folds anything: a `card_id` that resolves to
  a card in hand is replaced by that card's identity — name, cost, upgraded — so two copies of one
  card aimed at the same target still fold, while two different cards, two costs, an upgrade, or a
  card that does not resolve never do. Reported against the merged #295. Compatibility: fewer options
  are withheld, and no option that the host listed can now be hidden behind an unrelated one. Refs
  #302.

- Let an operator **set the System One confidence gate** per invocation. `sts2-jev-bridge` gains
  `--gate PERCENT`, an integer percentage so an argument vector carries no locale-dependent
  separator and admission can compare it exactly; absent, the bridge's own default still applies.
  The admitted argument form for `typesafe-jev` accordingly accepts either the four-element model and
  transport pair or that pair followed by `--gate PERCENT`, and nothing else. This exists because a
  gate is a measurement rather than a taste: the first two recorded live answers came back at `0.44`
  and `0.42` against a `0.55` default, so a lane left at the default would return `reobserve` on
  states like those and never act. Changing it through the recorded argument vector keeps that
  visible in a run's identity instead of hidden in a rebuild. `--describe` reports the gate the
  invocation would use. Compatibility: additive; the existing four-element form and the default gate
  are unchanged. Refs #308.

  The HTTPS exchange is performed by an operator-owned transport executable named by `--transport`,
  following the precedent `sts2-astra-bridge` set; request construction, bounds, catalog membership,
  the confidence gate and the decision shape stay inside the digest-pinned binary, and the credential
  never reaches this process. Standard input and output are serviced on their own threads and the
  transport is killed at a deadline, so neither side can deadlock on a full pipe.
  `systemone_decision` maps the answer: an in-catalog choice at or above the confidence gate becomes
  an `action` carrying the confidence as the percentage the decision contract already accepts, and
  one below it becomes `reobserve` rather than a guess. The `rationale` is composed from the returned
  distribution and labelled bridge-authored, because this provider generates no text and a fluent
  sentence presented as model reasoning would be a fabricated record.
  Compatibility: additive; one new binary, one new support module, one new document.
  `confirmed` only for the offline suite; a live call, the TLS path, decision quality, and any
  gameplay outcome are `unverified`. Refs #284, #288.
