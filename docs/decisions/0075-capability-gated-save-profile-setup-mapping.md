# ADR 0075: Map save-profile setup through a capability-gated operation contract

Status: accepted for the harness-owned, source-only slice of issue
[#102](https://github.com/AI-Ascension/sts2-harness/issues/102) — the typed mapping of authored
save-profile discovery, selection and disposable provisioning onto the accepted MCP tools and fixed
gateway routes. It calls no gateway, reads no save and creates no profile; the durable operation
adapter and the boundary validation are separate slices, and any real profile mutation stays gated
by the separately authorized gateway/game-mod lanes. It is ratified when the change carrying it
merges.

Revision, 2026-09-24: the baseline-fence rule was corrected to the owner's contract. The first
merged revision required the fence to name the *same* profile as the selection, which the gateway
does not require and which would have refused the owner's own accepted select fixture. See the
Decision bullet on effect-free admission.

## Context

Issue #102 requires the harness to drive save-profile setup through owner descriptors and the
accepted MCP routes without acquiring game, filesystem or process authority. The accepted
`sts2-mcp-server` contract (revision `save-profile-v1-mcp`, contract `gateway-save-profile-v1`)
already fixes five tools and their routes, but harness owned no artifact that says which authored
operation maps to which tool, which permission it needs, and which preconditions must hold before a
mutation is attempted. Without that, a workflow could not be refused before an effect.

Four failure modes had to be excluded by construction:

1. discovery that names a profile or a baseline fence, which is a selection in disguise;
2. a disposable provision fenced against a baseline that does not exist until the gateway
   allocates it, which would make provisioning impossible or force a fabricated fence;
3. a read-only deployment selecting or provisioning, or a selection-capable deployment allocating
   a profile it was never granted;
4. a mutation progressing on a readback that names a different profile, or on none at all.

## Decision

A new module `crates/harness/src/management/save_profile_setup/` owns the contract, split so each
file stays inside the production size budget:

- **Fixed vocabulary (`operation.rs`).** `ProfileSetupOperation` maps one-to-one onto the accepted
  tool names, and each operation resolves to a fixed route suffix; only an already-validated
  instance and operation identity are interpolated. `ProfileGrant` separates discovery, selection
  and provisioning. `PROFILE_ROUTE_REVISION` / `PROFILE_ROUTE_CONTRACT` pin the accepted consumer
  revision so drift is visible.
- **Authored shape (`request.rs`).** `ProfileSetupRequest` is a closed schema
  (`deny_unknown_fields`, pinned `ascension.save-profile-setup/v1`). Profile, instance and operation
  identities must be bounded and portable: a path traversal, a URL and a host path are refused, so
  no authored value can become a filesystem or network reference. `ProfileSetupGrants` is supplied
  by the deployment, never by the request, so an authored workflow cannot widen its own permission.
- **Effect-free admission (`setup.rs`, `error.rs`).** `admit_profile_setup` checks in a fixed
  order — schema and identity shape, permission, effect-free discovery, required profile, baseline
  fence presence, retained operation identity, active-run conflict — and refuses the first violated
  property. Only a selection may carry a fence, and it must carry one; the fence mirrors the
  owner's `ProfileBaseline` (`{ identity, digest }`), whose identity is the *baseline's* own
  user-data identity and is independent of the selected slot. The harness imposes no equality
  between the two, because the owner applies none: the gateway's own select fixture pairs
  `profile_id:"slot-1"` with `baseline.identity:"baseline-1"` and is accepted, and its `Select`
  validator shape-checks the two independently. Requiring equality would force a fabricated
  baseline identity and refuse a valid selection. A mutation admitted under an active run is
  refused. `AdmittedProfileSetup` exposes the exact tool and route and performs no call.
- **Readback verification (`setup.rs`).** A mutation is not usable on admission. A readback is
  verified separately: the identity must match the admitted one, the baseline must be a lowercase
  SHA-256 digest that matches the admitted fence, and the owner must report the profile available.
  Discovery names no profile, so no readback can verify against it — discovery cannot advance a
  downstream setup step.

## Consequences

- A workflow that lacks a permission, that smuggles a selection into discovery, or that fences a
  provision is refused before any call, and the refusal distinguishes the four cases.
- Save profiles stay separate from workflow, execution and inference profiles: this module carries
  only save-profile identity and its fence.
- Downstream setup may consume only a `VerifiedProfileReadback`, so an unverified or substituted
  profile cannot be handed on.
- This slice calls nothing and persists nothing. The durable operation adapter (issue #102 T2), the
  boundary validation matrix (T3), the Studio descriptor exposure and every real profile mutation
  remain open and are not claimed satisfied here.
