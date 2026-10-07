# ADR 0083: Ordinary Exo provider profile descriptor

Status: accepted source contract; runtime and broker admission remain separate gates.

## Decision

Describe the ordinary Exo one-shot decision route with a versioned
`NativeCapabilities` descriptor derived from a complete `ExoIdentity`. The
builder validates the identity through the public source-review descriptor,
requires the reviewed Exo source revision, and applies the existing provider
route and Responses-model predicates. It does not hardcode the fixture model
as the production allowlist.

The descriptor digest binds the complete inspected identity, provider-session
capability schema, profile and runtime selector versions, `turn/start` method,
Exo contract, outer wire and decision schemas, guarded-config and executor
message schema identifiers, and the SHA-256 of the checked-in Exo bridge
schema bytes. The ordinary descriptor proposes only `turn/start`; it does not
claim that every provider-session broker operation is implemented.

The runtime dispatch keeps its existing admission validation, then requires
the trusted identity to equal the independently inspected identity in full.
It also checks the configured source revision and runtime instance and
requires the supplied descriptor to equal the descriptor rebuilt from that
inspected identity. A descriptor built from a caller-selected identity alone
is not proof of inspection.

The Linux bridge's `--provider-capabilities CONFIG INSTANCE` command is a
bounded read-only description path. It requires the guarded private-state
configuration, validates the production provider route, inspects the loaded
files against the current bridge executable, serializes the descriptor, and
caps output at 8 KiB. It does not read request input, launch the executor,
contact the provider, or write state.

## Boundaries

This schema-only descriptor is not broker admission, provider execution, a
full Exo lifecycle profile, durable recovery, or native-game evidence. It does
not promote the lifecycle-v2 capabilities or alter the provider-session
schema. The broker's ordinary method-to-executor mapping still requires its
own implementation and review. Synthetic strict-envelope admission and the
R3 process witness also remain separate work; this decision does not add a
synthetic route or relax production route validation.
