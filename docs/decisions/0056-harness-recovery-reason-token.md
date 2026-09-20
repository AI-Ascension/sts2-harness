# ADR 0056: Carry and name the host recovery reason token

Status: accepted for the harness adapter and the episode failure surface;
native and end-to-end verification pending.

## Context

A runtime-v3 recovery state is **not self-describing**. The host answers
`{"state":"recovery","code":...}` and the state alone says only that policy may
not act. The reason is carried in the sibling `code`, which is a bounded wire
identity rather than free text, and the host composes it from one vocabulary:

- `host_not_configured` when a lane never declared a launch contract, and the
  recorded refusal code `launch_contract_refused_<reason>` when the contract was
  refused (gateway to host: `TryCurrent` yields
  `LaunchContractRefusal.UnconfiguredCode`, which is the never-declared code or
  the recorded refusal, and `RecoveryState` copies it into the observation's
  single state value);
- `host_observation_unavailable` when no host observation is current; and
- `recovery` or `unknown_state` when the host degenerate-composes a code from an
  empty or unrecognized state value.

Before this record the harness parser read only `observation.state.state` and
mapped `"recovery"` to `EpisodeStage::Recovery`, so the sibling `code` was
dropped at parse time and every one of those conditions reached the operator as
the same sentence, `episode requires recovery before policy can continue`. The
diagnostic that names the two compared directories stays in `game.log` by
design (the host records it as operator evidence and does not put it on the
wire), so the episode failure is the only place an automated acceptance run can
distinguish a refused launch contract from an unconfigured host or an
unavailable observation.

## Decision

1. **The reason travels with the observation.** The parsed recovery code is
   bound to the observation rather than passed beside it, so a stage and its
   reason cannot be separated by later plumbing. Only a recovery or unknown
   observation may carry one; binding a code to any other stage is refused, so a
   normal observation cannot acquire a reason that would misdescribe it.

   Composition is later plumbing. An expert runtime profile replaces the parsed
   runtime-v3 read with a projection that names its own `recovery` state and
   carries no `code`, so a site that rebuilds an observation from a baseline
   re-applies the baseline's code rather than re-deriving one from the
   projection. Deciding the stage again is not the same as keeping the reason.
2. **The code is held to the identity rule already in force.** One predicate
   governs both `state_id` and the recovery code -- non-empty, at most 512 bytes,
   ASCII alphanumerics plus `.`, `:`, `/`, `-`, `_` -- which is the shape the
   runtime-v3 observation schema requires of an identity field. The code is
   therefore never wider than a field the host already validates, and a value the
   host's own schema would reject is refused rather than reported.
3. **A malformed code fails closed and does not degrade to an anonymous
   recovery.** A recovery state whose `code` is absent, null, non-string, empty,
   oversized or outside that charset is refused at parse time. Reporting it as
   "no reason" would restore exactly the defect this record removes, and would
   pass host text into a failure message unchecked.
4. **The failure names the reason when the host named one, and invents none when
   it did not.** `EpisodeRunnerError::RecoveryRequired` carries an optional code;
   when present the sentence appends it, and when absent the established sentence
   is unchanged. No path, save, credential, directory or free-form host prose is
   reachable through the code, because the type admits only the bounded token.

## Consequences

- **The error contract changes.** `EpisodeRunnerError::RecoveryRequired` is a
  struct variant with an optional `code` in place of the former unit variant, so
  an exhaustive match on it does not compile unchanged. That is deliberate: a
  silent widening of the existing variant would let a caller keep reporting a
  bare recovery while the reason it now has goes unread.
- **The parse boundary is stricter than it was.** An envelope that previously
  produced an anonymous recovery with an unreadable code now fails to parse. This
  is a refusal of a shape the schema already disallows, not a new requirement on a
  conforming host.
- **The host vocabulary is not mirrored as a list.** The adapter admits the
  identity shape rather than today's reason strings, so a reason the host names
  later needs no second harness change, while a string the schema cannot carry
  still fails closed.
- **The `game.log` diagnostic is not imported.** The directory line remains in
  the game log and is not read, parsed or relayed by the harness.

## Verification

`crates/harness/src/bin/runtime_support/runtime_v3_parse_test.rs` covers a
recovery read that carries each vocabulary token into the observation, the
malformed and missing-code refusals, and that a playable observation binds no
code. `crates/harness/tests/episode_runner/recovery_reason.rs` covers the runner
failure naming a refused-contract reason, the unchanged sentence when the host
named none, the two binding refusals, and every vocabulary token the host
composes.

`crates/harness/src/bin/runtime_support/runtime_v4_expert_port_recovery_tests.rs`
covers the composition boundary: each of the three sites that rebuilds the
observation keeps the baseline's code, a composition whose baseline named none
stays anonymous instead of lifting one out of the projection, and a code offered
outside a recovery stage is refused.

These establish component behaviour. They do **not** prove a native recovery
transition, a refused launch contract observed end to end, or that any campaign
ran; those remain separate gates. The producer-side rule is read from the merged
`sts2-game-mod` source and is not executed here.
