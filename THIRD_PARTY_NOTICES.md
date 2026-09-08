# Third-Party Notices

The isolated Linux worker-transport candidate adds Linux-targeted Tokio `net`
and rustix 1.1.4 `event`, `fs`, `net`, and `process` features. Rustix offers
Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT and originates at
https://github.com/bytecodealliance/rustix. Tokio's newly locked socket2 0.6.5
dependency is MIT OR Apache-2.0 (https://github.com/rust-lang/socket2).
Secret comparison uses subtle 2.6.1, BSD-3-Clause
(https://github.com/dalek-cryptography/subtle), with defaults disabled;
secret buffer cleanup uses zeroize 1.9.0, Apache-2.0 OR MIT
(https://github.com/RustCrypto/utils), with defaults disabled and `alloc` enabled.
Locked offline Cargo metadata confirmed these declarations on 2026-09-08.
Cargo.lock retains crates.io sources and integrity checksums; no source is
vendored. The earlier advisory result below does not cover this changed lockfile.
Transport review, a refreshed advisory check, and native validation remain gates.

Operation identity creation uses `uuid` exactly `1.23.5` (Apache-2.0 OR MIT;
https://github.com/uuid-rs/uuid), with UUIDv4 generation backed by `getrandom`.
The dependency change locks `getrandom` 0.4.3 (MIT OR Apache-2.0), `r-efi` 6.0.0
(MIT OR Apache-2.0 OR LGPL-2.1-or-later), `futures-core`, `futures-task`, and
`futures-util` 0.3.34 (MIT OR Apache-2.0), and `slab` 0.4.12 (MIT). Target-specific
lock entries do not imply those packages execute on every supported platform.
These declarations were checked with locked, offline Cargo metadata on 2026-09-07;
exact sources and integrity checksums remain in Cargo.lock. No source was vendored.
The initially unavailable advisory tool was subsequently built in an isolated
tooling directory. cargo-audit 0.22.2 checked the 126-dependency lockfile against
RustSec revision `8a1eb4f933fb5821add5b4e98601ebd90b8b3538` with `--deny warnings`,
no ignored advisories and no target filters: exit 0, zero vulnerabilities and
empty warnings. Audited Cargo.lock SHA-256:
`d19e690f50502932fe3872c629018127ec566e3b36eac9694d56853351d088e5`.
This bounded snapshot check must be refreshed for changed dependencies or release.

Runtime-v3 contract tests use `jsonschema` 0.52.1 (MIT), with default features disabled to avoid
HTTP/file reference retrieval. It is a dev-only schema validator, not a second protocol owner.
The copied Runtime-v3 schemas/goldens are MIT-licensed `sts2-protocol` artifacts with exact
commit, checksum inventory, mapping, and sanitization provenance in their adjacent README.

This foundation contains no game binaries, host assemblies, model weights, datasets, provider SDKs,
or copied implementation source.

The target-local repository policy tool uses the Cargo dependency `toml`; the deterministic POC uses
`serde`, `serde_json`, and `sha2`, all declared in the workspace manifest and locked in `Cargo.lock`.
Their upstream licenses and transitive notices are resolved from the pinned Cargo package metadata
and must be rechecked by release tooling before distribution. Dependencies are not vendored into
this repository.

The Runtime-v1 MCP process adapter uses MIT-licensed Tokio exactly1.53.1 with default features
disabled and only rt/process/io-util/time/macros enabled. Its locked process/I/O dependencies are
not provider SDKs or copied source; their declared licenses remain independent. The rationale and
bounded lifecycle are documented in ADR0005; release-time advisory/license review remains required.

The research-package tool uses `jsonschema` exactly `0.52.1` as a test-only dependency (MIT;
upstream `https://github.com/Stranger6667/jsonschema`). Default features are disabled, including
HTTP and file reference retrieval. It validates synthetic research-schema instances, not game
behavior. Its transitive dependency versions are pinned in `Cargo.lock`; their own declared
licenses apply independently and must be included in any distribution review.
The inspected Linux test dependency metadata also includes Apache-2.0, MIT-0, Zlib,
Unicode-3.0, and dual-license choices; this notice does not relicense those packages as MIT.

The Exo process adapter uses Tokio 1.53.1 (MIT, crates.io, upstream
https://github.com/tokio-rs/tokio), with only `rt`, `process`, `io-util`, `time`, and `macros`
features requested. Newly locked dependencies are bytes 1.12.1 (MIT), mio 1.2.3 (MIT),
tokio-macros 2.7.2 (MIT), errno 0.3.14, signal-hook-registry 1.4.8, pin-project-lite 0.2.17,
windows-link 0.2.1, and windows-sys 0.61.2 (each MIT or Apache-2.0), plus
wasi 0.11.1+wasi-snapshot-preview1 (MIT or Apache-2.0, also offers LLVM-exception terms).
Sources and exact integrity checksums are in Cargo.lock; these are dependencies, not imported
fixtures or retained provider data. Cargo metadata verified these license declarations on
2026-09-04. A release must still check current advisories and redistribution notices.

Manual advisory review used RustSec database commit
`5a0ebedfe8bdd2e295b171f4162f8c977bcad9a5`: five Tokio, two mio, and one bytes
advisories matched newly added package names; the locked versions satisfy their patched ranges.
No records matched the other newly added package names at that snapshot. This is a bounded
dependency-change review, not a full-lockfile `cargo audit` result (that tool was unavailable).

Future dependencies, imported fixtures, generated schemas, provider adapters, and model artifacts
must record their source, exact version or digest, license, redistribution permission, and retention
status before they become release inputs. Unknown or incompatible provenance is a release blocker.

The patch-diff workspace tool uses the existing pinned `serde` and `serde_json` packages for
structural JSON parsing. Its full-schema tests use `jsonschema` 0.52.1 (MIT, crates.io, upstream
https://github.com/Stranger6667/jsonschema) with default features disabled. This test-only dependency
does not enable external HTTP/file schema resolution and is not linked into the utility binary.
Exact transitive versions and integrity checksums are recorded in Cargo.lock. The validator checks
schema structure, not the truth of runtime evidence or permission to promote a quarantined build.
