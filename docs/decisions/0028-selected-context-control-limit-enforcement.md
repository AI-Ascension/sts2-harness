# ADR 0028: Selected context-control limit enforcement

## Status

Accepted for scoped implementation of Harness #95. No consumer adoption, production owner
composition or feature closure is approved here.

## Context

A context-binding descriptor advertises `ContextEffectiveLimits` (`max_items`, `max_notes`,
`max_context_bytes`, `max_objective_bytes`, `max_control_events`). Those values were only
*validated* — `context_owner_support.rs` requires each one to be non-zero and no greater than the
corresponding harness maximum — and never *enforced* at the point of use. A descriptor could
therefore advertise, say, `max_notes: 4` while the renderer happily prepared a draft with the
harness maximum of notes, so the runtime accepted input the selected owner had said it would not.

Issue #95 requires the opposite: a value that is valid against the portable contract but not
executable for the selected owner/profile must be refused with a precise error **before** inference
or retention, rather than failing late.

## Decision

Enforce the selected limits in the render path:

- `ContextRenderLimits { max_items, max_notes, max_context_bytes, max_objective_bytes }` carries the
  selected limits into the renderer. `ContextRenderLimits::harness_maxima()` is the outer bound.
- `ContextEffectiveLimits::render_limits()` converts an advertised descriptor into render limits. It
  can only narrow: the advertised values were already checked against the harness maxima when the
  descriptor was validated.
- `ContextRenderer::enabled_at_with_limits(..., limits)` runs the existing harness-maxima checks
  first (which still report `ContextRenderError::TooLarge`, so current behaviour is unchanged) and
  then enforces each selected limit with a new, precise
  `ContextRenderError::ExceedsSelectedLimit("<limit>")` naming the limit that was exceeded.
- `ContextRenderer::enabled_at` keeps its signature and now delegates with
  `ContextRenderLimits::harness_maxima()`, so every existing caller and test behaves exactly as
  before.

## Compatibility

Additive. `enabled`, `enabled_at` and `legacy` keep their signatures and behaviour; the new entry
point and type are additional. No default, schema, digest, range or resource bound changes, and the
harness maxima are neither raised nor lowered.

## Consequences and limits

- Enforcement happens at preparation time, so an over-limit draft is refused before any provider
  call or retention; nothing is clamped or silently truncated.
- `max_control_events` applies to recorded control transitions rather than to rendering, so
  enforcing it belongs to the control-transition path and remains outstanding, along with
  authenticated owner/consumer composition and the Console/Studio journeys. #95 stays open.
