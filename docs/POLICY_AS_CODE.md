# Policy as Code

## Purpose

Written guidance is advisory until an enforceable check makes objective parts visible. This repository
keeps its objective foundation rules in `policy.toml` and checks them with the Rust `repo-policy`
workspace tool. Review is still required for architecture, provenance, privacy, and evidence claims.

## Local entrypoint

```bash
cargo run --locked --package repo-policy -- --strict
```

The command is read-only. Strict mode turns preferred-size warnings into failures and returns nonzero
when a mandatory rule or warning fails.

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
