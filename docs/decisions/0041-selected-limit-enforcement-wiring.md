# ADR 0041: Production enforcement of the selected context-control limits

## Status

Accepted for scoped implementation of Harness #95. No consumer adoption, saved-policy store,
Console/Studio journey or feature closure is approved here.

## Context

ADR 0028 added `ContextRenderLimits` and `ContextRenderer::enabled_at_with_limits`, and ADR 0030
composed the limits a binding's admitting descriptor advertises. Both were correct in isolation but
neither was reachable from production: `ContextRenderer::enabled_at_with_limits` had no production
caller, and `max_control_events` was only range-checked when a descriptor was validated. At the
control-transition boundary the authority saturated silently — `ControlAuthority::event` returned
without recording once `self.events.len()` reached the harness maximum — so a profile that
advertised a smaller bound had no effect, and nothing observed the truncation.

## Decision

Wire the selected limits to the point of use, through the same fail-closed composition seam the
read-only projection already uses (`compose_context_owner_binding`), so what a consumer can observe
and what the harness enforces cannot diverge:

- `ManagementService::prepare_context_render` resolves the run's **current** binding and its
  admitting descriptor, then renders through `ContextOwnerEffectiveLimitsView::prepare_managed_render`
  → `ContextRenderer::enabled_at_with_limits(..., view.render_limits())`. An over-limit draft is
  refused with `context_render_limit_exceeded` naming the limit, before any provider call or
  retention. An unattached owner or an unusable binding stays explicitly unavailable rather than
  falling back to the harness maxima.
- `ManagementService::bind_context_control_authority` applies the selected bound through
  `ControlAuthority::with_max_control_events`. `ControlAuthority::event` now returns `Result` and
  refuses with `context_control_events_exhausted` instead of dropping silently; `stop` and
  `advance_boundary` propagate that refusal. `MAX_CONTROL_EVENTS` (4096) is the public outer
  ceiling, `with_max_control_events` rejects `0` or anything greater than the ceiling with
  `control_event_limit_invalid`, and `recover_bounded` refuses a journal whose retained events
  exceed the supplied bound.

Both entry points require `workflow:control` for the run and reuse the existing composition checks,
so a foreign owner, missing or disabled descriptor, non-available binding, unknown binding identity,
grant or continuity escalation, oversized descriptor or stale digest is refused before any value is
applied.

## Compatibility

Additive and behaviour-preserving for existing callers. The default authority still uses
`MAX_CONTROL_EVENTS`, so an unwired authority saturates at exactly the harness maximum as before;
the difference is that saturation is now an explicit refusal rather than a silent no-op. No route,
request/response schema, private record, SQLite table, digest, default or resource ceiling changes,
and the harness maxima are neither raised nor lowered.

## Consequences and limits

- Enforcement happens at preparation and transition time, so an over-limit draft or an exhausted
  control authority is refused before any provider call or retention, and nothing is clamped or
  silently truncated. Each transition reserves its recorded-event capacity before it mutates state,
  so a refusal at the bound leaves the plan, boundary, receipts and operation ledger unchanged.
- The saved provider-session policy store with adoption history (ADR 0027) is still
  library-only: no durable provider-session policy store exists in the harness, so
  `SessionPolicyMigrationProposal` cannot be wired to a point of use in this slice and remains
  outstanding for #95, as do the Console/Studio journeys.
- These entry points have no in-repo route or runtime caller yet: the existing control path
  (`management/workflow_ports.rs`) and the durable control store still build unbounded authorities,
  so wiring them is adoption work tracked by #95 rather than something this slice completes.
- Native/provider/deployment acceptance is not claimed: these checks are deterministic, synthetic
  and offline.
