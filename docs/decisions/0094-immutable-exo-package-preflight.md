# ADR 0094: Immutable Exo installation and non-inferencing preflight

## Scope and ownership

This is the repository-owned packaging increment of Harness #147. The new `sts2-exo-package`
binary uses the existing one-shot v2 configuration and source verifier. It adds no Exo dependency,
provider implementation, scheduler, service manager, gateway/MCP authority or model credential
provisioning. It neither fetches external source nor publishes a package or release.

The supported implementation row is Linux x86_64. Other platforms refuse the command explicitly.
The package profile is standard/fresh with the existing empty model-tool catalog; lookup, managed
context, continuity and other extension variants are outside this installation format.

## Closed package and configuration records

`sts2.exo-install-manifest-v1` is canonical pretty JSON with a final newline. It records the
declared 40-hex Harness source revision, reviewed Exo revision, Rust/Node/pnpm pins, bridge contract,
tool-catalog digest, and a sorted closed inventory of fifteen components. Each component carries
its filename, actual nonzero byte length and SHA-256. Paths, credentials and timestamps are absent.
The manifest is deterministic for the same supplied bytes and declaration. It does not attest how
a binary was built: `build_provenance` is always `operator_review_required`.

The inventory consists of the bridge, executor, packaging tool, Node, the owned one-shot extension,
the three Cargo locks, Exo's pnpm lock, extension package metadata, Rust toolchain file, the Harness
notice file, and the Harness/Exo/Node license files. Source-owned bytes must also equal those
embedded in the packaging tool. Extra entries, nested modules, symbolic/hard links, nonregular
files, missing execute permission, loose write permissions and bounded-file overflows are refused.
License bytes are recorded for review; hashing them does not establish redistribution permission.

The private `sts2.exo-install-settings-v1` record selects an existing absolute Exo source root,
explicit `synthetic` or `openai` mode, model/route, quota and retention. The shared bridge route and
private-state validators retain authority. Synthetic mode cannot name an external endpoint.
Installation requires independently supplied SHA-256 values for both the exact manifest and the
exact private settings bytes. A digest is an integrity pin, not an authorization or signature.

The installation ID is SHA-256 of the domain-separated pair of these two digests. Changing the
model, endpoint, source locator, quota, retention or any component produces a distinct installation.
The generated `sts2.exo-one-shot-config-v2` names the copied artifacts and three disjoint private
roots beneath that installation. The existing configuration schema and runtime admission rules
are unchanged.

## Filesystem and interrupted installation behavior

The installer opens every directory component without following symlinks. Ancestors must be owned
by root or the service UID and must not be writable by other users, except for a root-owned sticky
temporary directory. Installation directories are service-owned `0700`; copied executables are
`0500`, other sealed files `0400`. An existing nonempty foreign root is refused. An owned root has
a fixed format marker and a private nonblocking installer lock. The root contains at most 64
installations; there is no automatic pruning.

Each destination is created exclusively under its content-addressed ID. Files are copied through
bounded streams, hashed again while writing, synced and sealed. The receipt is written last and
binds the manifest, settings and configuration digests. A crash leaves an incomplete destination
that fails inspection. Retrying cannot overwrite it, and the tool does not infer that deleting it
or changing any adjacent state is safe. Existing installations and unrelated agents remain intact.

These are service-UID and ordinary filesystem protections. They do not defend against a hostile
administrator or a process with the same UID rewriting trusted files during inspection. The tool
does not supply an OS sandbox, kernel storage quota or deployment containment.

## Preflight and rollback

Preflight rechecks the closed installation inventory and receipt, hashes every actual component,
reconstructs the exact configuration, and observes private-root permissions, entry/depth bounds and
total retained bytes without deleting or rewriting any private data. Quota observation is not a
reservation; the existing guarded bridge continues to enforce its own runtime checks.

It compares the actual Exo source Cargo/pnpm locks with the package, then calls the shared bridge
loader to verify the source revision, tracked cleanliness, executable/module hashes, model/route
and guarded configuration. Only after those checks does it run the supplied Node with `--version`
and the supplied bridge with `--describe CONFIG`. Both use an empty inherited environment, null
stdin/stderr, bounded stdout, a ten-second deadline and bounded direct-child cleanup. The observed
version and entire canonical description must match the expected package. The installer lock is
not held over these subprocess boundaries.

The result distinguishes package preflight from model authentication, executor loadability,
synthetic smoke, downstream readiness, native gameplay and full runtime admission. Those remain
unverified. No credential is requested or read, and neither executor nor provider is invoked.

There is deliberately no mutable `current` link, overwrite, service activation, journal migration
or operation retry command. A candidate upgrade is another immutable installation. `rollback-check`
preflights the retained source and target installations and returns `held_for_owner_readmission`,
with `changes_applied: false`. Existing active/unknown operations are not inspected or modified.
The existing lifecycle journal cannot be reopened as a read-only quiescence check: reopening
advances its owner epoch. The packaging tool therefore does not manufacture a quiescence claim.
The owning scheduler must hold new admissions, reconcile original operations under their original
pins, and authorize a new admission before selecting a different configuration. This remaining
service cutover contract stays under #145/#147.

## Compatibility and validation

This is an additive CLI and private installation format. Old bridge configurations, admission,
wire schemas and journals are unchanged. Unknown record versions or platforms fail closed. Retain
the matching packaging binary with each installation; a future changed format or owned source pin
requires its corresponding reader, and unsupported rollback remains held.

Production-path tests cover deterministic manifest generation, exact copying and permissions,
changed inputs/configuration, closed-shape/duplicate-key rejection, unsafe paths and links,
quota refusal without deletion, side-by-side configuration identity, preserved opaque unknown
records, partial installation refusal and lock contention. They use explicitly synthetic package
inputs and establish filesystem/CLI behavior only. An absent reviewed Exo source prevents preflight
before any probe executable runs. Real packaged Exo smoke, live service cutover and actual provider
or native-host acceptance require their separate evidence and do not follow from these tests.

The operator recipe and exact remaining gates are in the
[package README](../../experiments/exo-agent/package/README.md).
