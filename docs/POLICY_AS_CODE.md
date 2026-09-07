# Policy as Code

## Purpose

Mandatory requirements remain mandatory whether checked automatically or through owner review.
Only explicitly designated guidance is advisory. This repository keeps its objective foundation rules in `policy.toml` and checks them with the Rust `repo-policy`
workspace tool. Review is still required for architecture, provenance, privacy, and evidence claims.

## Local entrypoint

```bash
cargo run --locked --package repo-policy -- --strict
```

The command is read-only. This target uses policy version 2: strict mode keeps preferred-size
`SIZE001` guidance advisory and returns nonzero for mandatory findings or hard limits. Version 1 is
still parsed for legacy callers and its strict mode turns every warning into a failure.

## Rule families

| Rule | Enforcement |
|---|---|
| `CFG001` | Policy exists, parses, and uses its supported version |
| `DOC001` | Required foundation files exist |
| `DOC002` | Local Markdown link targets exist |
| `SIZE001` | Rust, workflow, and Markdown files stay within budgets |
| `EXC001` | Exemptions are exact existing paths with meaningful reasons |
| `WF001` | Workflows declare top-level permissions |
| `WF002` | `pull_request_target` is prohibited |
| `WF003` | `continue-on-error: true` is prohibited |
| `WF004` | External actions use full immutable commit SHAs |
| `WF005` | Workflow commands cannot hide failure with `|| true` |
| `RUST001` | Cargo workspace has matching toolchain, lockfile, and lint policy |
| `LANG001` | Python source and package metadata are prohibited |
| `LIC001-003` | MIT license and Rust source headers are present |

The checker is deliberately bounded: it does not prove runtime behavior, schema semantics, provider
correctness, game compatibility, artifact confidentiality, or that an implementation respects every
architectural invariant. Those require tests, review, and controlled evidence.

## Configuration and exemptions

`policy.toml` lists required files, ignored build/editor/vendor directories, limits, and exact-path
exemptions. An exemption must explain provenance or regeneration in at least twenty characters. Do not
use wildcards, broad prefixes, or an exemption to preserve copied implementation source.

Rust `src/bin` production source is traversed despite the generic generated-output `bin` ignore.
The traversal regression proves file collection and size/language findings while preserving ignored
generated managed `bin` output. No runtime-source size or license exemption is granted.

## CI and change control

`policy.yml` runs the same checker on pull requests and pushes to `main` with read-only contents
permission, a timeout, and immutable action pins. Changes to policy are themselves reviewed changes:
explain the rule, enforcement effect, migration, and exact local results. Never weaken policy merely to
make unrelated work pass.

## Known limits

The current checker validates the target-owned harness package structurally, including required
headers and bounded source/test files, as well as tooling and documentation. It does not prove the
package's semantic invariants or runtime boundaries. Future integrations must add dependency-
direction, contract/conformance, artifact, privacy, and release checks when enforceable structures
exist; planned checks are not current evidence.

## Compiler and minimum version

The development compiler remains exactly 1.97.1 and the declared Rust MSRV remains
1.97.1. This repository retains its existing requirement that those values match;
`rust-version` is a minimum supported version in Cargo, not a generic exact compiler
selector. Any deliberate separation of these values needs the owner policy change
and its compatibility matrix, rather than deletion of the equality check.

## Production lint scope

The production Clippy lane selects workspace libraries and binaries and forbids
unwrap, expect, panic, todo and unimplemented on the compiler command line.
A source-level allowance cannot override that lane. The existing all-target lane
still checks tests with their scoped allowances. `production_lints` runs real
compiler fixtures for forbidden constructs, an attempted blanket allowance,
and valid comments/test-only code. Missing Clippy or an unrelated compiler
failure cannot satisfy a negative case: its diagnostic must identify the rule.
