# ADR 0027: Bounded migration records for saved provider-session policies

## Status

Accepted for scoped implementation of Harness #95. No consumer adoption, production owner
composition or feature closure is approved here.

## Context

ADR 0026 made the harness able to classify a saved provider-session policy precisely: portable
schema validity is separate from whether the **selected** profile can execute it, with
`effective_limit_exceeded` and friends instead of a generic invalid-policy failure.

That left the operator half of the requirement undefined. Issue #95 requires that a saved policy
above a new effective limit be retained for inspection, blocked at admission, and offered a
**bounded migration proposal that requires explicit approval** — never silently clamped.
The context-memory surface has such a flow (`PolicyMigrationProposal`); provider sessions had none.

## Decision

Add a provider-session migration record that records the difference instead of resolving it:

- `ProviderSessionPolicy::capability_limit_violations(&NativeCapabilities)` reports each limit the
  saved policy asks for above the profile's executable ceiling. Absent rows are not violations; they
  are reported separately by `admit_for_profile` as not advertised.
- `SessionPolicyMigrationProposal::new_from_bytes` retains the **exact** saved policy bytes
  (`original_policy_bytes`, `source_policy_sha256` over those bytes, the capability descriptor digest
  it was raised against) and the violated limits, starting in state `Proposed`. The source policy must
  be portable-schema valid and at least one limit must be violated, so a proposal cannot be raised for
  a contract violation or for an already-executable policy.
- `approve(approval_ref)` records explicit operator approval. It changes no bytes and activates
  nothing.
- `adopt(target, capabilities, approval_ref)` requires state `Approved`, the matching approval
  reference, and the matching capability descriptor. The **caller supplies** the target policy; this
  API never derives, clamps or partially applies one, and it refuses any target that still violates an
  executable limit with `effective_limit_exceeded`. A successful adoption records the adopted digest
  and moves to `Adopted` while the original bytes remain retained.
- `SessionPolicyMigrationError` distinguishes `InvalidProposal`, `PermissionDenied`,
  `InvalidCapabilities` and `CapabilityLimitExceeded` with stable `code()` strings.

## Compatibility

Additive and library-only. No policy field, schema, digest, range or resource bound changes;
`validate`, `validate_schema` and `admit_for_profile` are unchanged. The portable schema ceilings stay
intentionally broader than the executable ceilings, and this record classifies the difference rather
than narrowing either bound.

## Consequences and limits

- Producing or approving a proposal performs no persistence, no activation and no provider call. A
  saved-policy store with adoption history, and the approval authority behind `approval_ref`, remain
  outstanding, as do selected context-control limit enforcement and owner/consumer composition; #95
  stays open.
- Because adoption takes a caller-supplied target, a target that satisfies the executable ceilings by
  changing semantics is the caller's explicit decision and is recorded as such — it is never inferred
  by the harness.
