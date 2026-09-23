# ADR 0062: Ask a two-stage option question above the presented bound

Status: accepted for the System One bridge production path in `crates/harness`. It changes what the
bridge asks, not whether a provider is called: the ask only splits in two when the presented option
set is larger than the single-question bound. It does not authorize a provider, native-host, game or
deployment lane, and it does not claim any run has exercised the two-stage shape against the real
endpoint.

## Context

`OptionSelection::from_observation` computes `SelectionMode::TwoStage` when the presented option set
exceeds `MAX_PRESENTED_OPTIONS` (24), and `SelectionMode::Forced` when exactly one action is legal.
Until this record the production consumer (`crates/harness/src/bin/support/jev_record.rs`) only
special-cased `Forced`; otherwise it built exactly one `action` question through
`build_described_system_one_request`. `TwoStage` was therefore computed and never asked: the variant's
documented behaviour ("ask for a kind first, then an action within it") did not exist as behaviour.

Two designs were defensible (tracked in issue #408):

1. **Two round trips** — one `kind` question, then one `action` question restricted to the chosen
   kind. Each question stays small, which is the reason the bound exists.
2. **Both questions in one call** — the provider evaluates many questions in parallel, but the
   `action` question would have to carry options from every kind to be answerable, so the `kind`
   answer would be read by nothing. The module documentation rejects exactly that: an unconsumed
   question would spend tokens to produce a number no code reads.

## Decision

Design 1. When the presented set exceeds the bound and the evaluation (`--tactical`) profile is not in
use, the bridge asks two questions in sequence:

- a `kind` question whose criteria are the distinct kinds of the presented options, built by
  `build_class_system_one_request`; then
- an `action` question whose criteria are exactly the presented options of the chosen kind, built by
  the existing `build_described_system_one_request`.

The chosen kind is re-checked against the presented classes, and the chosen action against the
chosen kind's presented options, by the same containment check the single question already applied; a
kind outside the presented classes fails closed before the second stage is asked.

The record keeps `provider_request`, `provider_response`, and `decision` as the **action** stage, so a
published evidence file reads the same decision-from-response pair it did before. The kind stage is
carried beside it under `class_question`, and every record now carries `selection_mode`
(`forced` / `single` / `two_stage`). `selection_mode` is a strictly additive field: records are
validated by named-field presence, so an existing reader ignores it.

The evaluation profile owns a separate question-count contract (`1 + 7 * targets`) and is not part of
this change; when `--tactical` is set the ask stays a single question regardless of the computed mode.

## Consequences

- A presented set in `(24, 64]` costs two provider calls per decision instead of one. Above 64 the
  builder refuses first, as before.
- The two stages are each independently tested at the builder boundary and end to end through the
  bridge's offline fake exchange; the record states the mode so a two-stage ask is observable.
- No vendor evidence exists for this shape: ADR 0053 records that no run in this repository has called
  the System One endpoint. The two-stage shape is therefore `source-derived`, not `confirmed`, and the
  first real run exercised against the endpoint is what would retire that caveat.

Refs #408. Updates the disposition of #290 acceptance criterion 6.
