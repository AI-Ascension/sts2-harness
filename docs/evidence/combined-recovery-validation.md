# Combined recovery validation

Date: 2026-09-07. Classification: confirmed build and synthetic test evidence.
Source under test: `a18fa66`, combining provider-result reuse (`f000cc1`)
and completed-resume (`00307e3`), with integration formatting/lint repairs.

Root independently ran these commands with a dedicated build directory:

```text
cargo check --locked --offline --workspace --all-targets --all-features
cargo fmt --all --check
cargo clippy --locked --offline --workspace --all-targets --all-features -- -D warnings
cargo test --locked --offline --workspace --all-targets --all-features
cargo run --locked --offline -p repo-policy -- --strict
```

All completed with exit status zero. The policy check reported 391 sized files,
zero warnings and zero errors. The runtime binary suite ran 81 passing tests;
the workspace command also passed the library, integration, and tooling suites.

This does not close independent review: provider payload allocation bounds and
completed-resume test isolation are under review. It does not establish live
provider reuse, host settlement, service installation, reboot recovery, or soak
completion. No game/provider/service was launched by this validation.

## Payload bound and duplex test follow-up

Root repeated workspace/all-target/all-feature tests and strict Clippy at
`63a53a2`; both commands exited zero. This includes `a1833f3`, which checks the
borrowed SQLite result length before copying to a Rust vector, and the duplex
test's explicit second-exchange drain acknowledgment. Production timeouts were
not changed. The runtime suite passed 81 tests and execution-store suite 10.
The payload bound covers Rust copying, not SQLite's internal BLOB materialization.
Child-supervisor changes through the separate completed-resume review candidate
are not included in this tested revision.
