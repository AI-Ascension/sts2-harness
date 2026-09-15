# ADR 0026: Saved provider-session policy admission

## Status

Accepted for scoped implementation of Harness #95. No consumer adoption, production owner
composition or feature closure is approved here.

## Context

The provider-session policy contract has two distinct range checks that were previously reachable
only as one generic failure:

- the portable schema ceiling, enforced by `ProviderSessionPolicy::validate_schema`
  (`SESSION_POLICY_SCHEMA_MAX_COMPLETED_TURNS = 1_024`,
  `SESSION_POLICY_SCHEMA_MAX_HISTORY_TTL_SECONDS = 604_800`); and
- the executable ceiling of the **selected** adapter profile
  (`MAX_COMPLETED_TURNS = 128`, `MAX_HISTORY_TTL_SECONDS = 86_400`), which is published only through
  the profile's effective-limit record (`NativeCapabilities::effective_limit_record`,
  `admit_policy_value`).

`ProviderSessionPolicy::validate` combined both checks and returned
`SessionError::InvalidPolicy` for either. A saved policy that was perfectly valid against the
portable contract but above the selected profile's executable ceiling therefore surfaced as a
generic invalid-policy failure. That is the failure mode issue #95 forbids: a value must never be
silently clamped, and "this profile cannot execute it" must not be reported as "this policy is
invalid".

## Decision

Add an explicit, precise classification for a saved policy against a selected profile:

- `ProviderSessionPolicy::admit_for_profile(&NativeCapabilities) -> Result<(), PolicyAdmissionError>`
  checks portable schema validity first, then admits each policy limit through the capability
  descriptor's effective-limit record.
- `PolicyAdmissionError` keeps the outcomes distinguishable:
  `Schema(SessionError)` for a portable-contract failure (code
  `provider_session_policy_schema_invalid`) and
  `Profile(UnavailableReason)` for a schema-valid policy the selected profile cannot execute.
  The profile codes are the existing effective-limit reasons (`effective_limit_exceeded`,
  `disabled`, `field_not_advertised`, `profile_mismatch`, ...).

No value is clamped, rewritten or partially applied. A refused policy keeps the exact saved values,
so a caller can retain it for inspection and propose a bounded migration that requires explicit
approval. `validate` and `validate_schema` keep their existing behavior and are unchanged.

## Compatibility

Additive and read-only. No policy field, schema, digest, range, resource bound or existing error
changes; `validate`/`validate_schema` remain as they were and the new method is the only addition.
The portable schema ceilings are intentionally broader than the executable ceilings and remain so:
this ADR classifies the difference rather than narrowing either bound.

## Consequences and limits

- The classification is only as good as the selected descriptor: an unadvertised field is reported
  as `field_not_advertised` rather than treated as unlimited, and a disabled surface reports
  `disabled` rather than "no policy".
- A full saved-policy store with adoption history and approval gating for provider sessions remains
  outstanding, as does the consumer composition around it; #95 stays open.
- This does not assert provider execution, native game state or any live adapter behavior.
