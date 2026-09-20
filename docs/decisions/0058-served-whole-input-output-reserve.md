# ADR 0058: A served whole-input bound with an advertised output reserve

Status: accepted for the context-owner descriptor and the served managed render boundary. This
record does not authorize a provider, native-host, game, deployment or paid-call lane, and it does
not raise any capacity: the default advertises no reserve and every existing bound is unchanged.

## Context

`sts2-harness#107` asks for final prepared-input budgets enforced with qualified model/token
measurements. The measurement, eviction and reservation arithmetic for a *request* was already
merged as a library (`PreparedInputRequest`, `ContextRenderLimits::max_context_bytes`), but nothing
in a served path admitted the bytes a decision actually sends. `ContextRenderLimits.max_context_bytes`
bounds `PreparedContext.input.len()` — the dispatcher's whole request — so the existing check already
bounds the input bytes under the common reading of that field. What did not exist was any published
statement of how much response capacity the same bound has to leave room for, and no served refusal
for an input that fits the bound on its own but not beside that reserve.

The hazard is a bound that reads as covering the whole exchange while covering only half of it: a
descriptor could publish `max_context_bytes` and a provider configuration could independently reserve
output capacity, so the composed exchange exceeded the number an operator read from the catalog with
nothing refusing it.

## Decision

An owner may advertise an output reserve beside the whole input.

- `ContextEffectiveLimits` (the closed `ascension.context-control.owner-binding.v1` descriptor) gains
  an optional `output_reserve_bytes`. `ContextRenderLimits` gains the same optional field, and
  `harness_maxima()` advertises `None`.
- `None` is exactly the pre-existing contract, not a claim of unlimited response capacity:
  `max_context_bytes` bounds the input bytes alone and response capacity stays bounded independently
  by the provider configuration (`ExoConfig.max_response_bytes`). Because the field is optional and
  skip-serialized, a descriptor that does not advertise it serializes byte-identically to
  pre-adoption producer output, and the committed catalog conformance fixture is unchanged.
- When a reserve is advertised, `max_context_bytes` is the *combined* whole-input bound: the admitted
  input is `max_context_bytes - output_reserve_bytes`. `AssembledInputBound` owns that subtraction and
  the assembled refusal, and `PreparedInputRequest::prepare` reuses the same helper rather than
  repeating the arithmetic, so the request and assembled shapes cannot drift apart.
- The served managed decision admits the already-assembled provider bytes against that bound
  immediately after `prepare_managed_context` and before `assert_render_source_current` or
  `decide_prepared_for`, so an over-bound input is refused before any provider exchange and without
  writing anything. The refusal reuses the merged `CombinedWindowOverflow` shape and reports the
  management codes `context_whole_input_budget_exceeded` (the input does not fit) and
  `context_whole_input_budget_invalid` (the advertised reserve is zero or above the harness guard,
  so the bound is unusable rather than permissive).
- Nothing is evicted at this boundary. Optional content was already selected, and the bytes exist;
  eviction remains the request path's job. `validate_limits` rejects an advertised reserve of zero and
  one above `MAX_PREPARED_OUTPUT_RESERVE_BYTES` (8 KiB), the same guard the memory surface applies.

## Consequences

AC1 of `#107` is claimable only at this boundary and only when a reserve is advertised: a descriptor
that publishes `None` inherits exactly the previous behaviour, and this record does not claim that
the input-plus-response composition was bounded before. The refinement is additive: one optional
field on two types, no new wire variant, no removed public variant, no route, digest or default
change, and no capacity raised. Token measurement, saved-policy migration, native execution and
composed browser-to-owner execution remain unverified by this change, and no provider was called
while validating it.
