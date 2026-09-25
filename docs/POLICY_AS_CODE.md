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
| `ADR001` | ADR decision records use unique four-digit numbers |
| `SIZE001` | Rust, workflow, and Markdown files stay within budgets |
| `EXC001` | Exemptions are exact existing paths with meaningful reasons |
| `WF001` | Workflows declare top-level permissions |
| `WF002` | `pull_request_target` is prohibited |
| `WF003` | `continue-on-error: true` is prohibited |
| `WF004` | External actions use full immutable commit SHAs |
| `WF005` | Workflow commands cannot hide failure with `|| true` |
| `RUST001` | Cargo workspace has matching toolchain, lockfile, and lint policy |
| `RUST002` | Every tracked Rust source is reachable from a crate root through `mod`, `#[path]`, or `include!` |
| `LANG001` | Python source and package metadata are prohibited |
| `LIC001-003` | MIT license and Rust source headers are present |

The checker is deliberately bounded: it does not prove runtime behavior, schema semantics, provider
correctness, game compatibility, artifact confidentiality, or that an implementation respects every
architectural invariant. Those require tests, review, and controlled evidence.

`RUST002` follows rustc's own name resolution for the declaration forms it recognises, so it must
never report a file rustc compiles. That holds for raw identifiers — `mod r#move;` resolves to
`move.rs` and an inline `mod r#type { }` owns `type/`, not `r#move.rs` or `r#type/`, because the
name is the stem rustc looks up for whether or not the `r#` form was needed to write it — and for
`include!`, whose target is compiled in place, so a `mod child;` inside the fragment resolves beside
the fragment rather than beside the file that included it.

## Configuration and exemptions

`policy.toml` lists required files, ignored build/editor/vendor directories, limits, and exact-path
exemptions. An exemption must explain provenance or regeneration in at least twenty characters. Do not
use wildcards, broad prefixes, or an exemption to preserve copied implementation source.

Rust `src/bin` production source is traversed despite the generic generated-output `bin` ignore.
The traversal regression proves file collection and size/language findings while preserving ignored
generated managed `bin` output. No runtime-source size or license exemption is granted.

`CHANGELOG.md` is not exempt from the Markdown budget. When the active file would exceed
`markdown_preferred`, move the oldest completed entries verbatim into
`docs/CHANGELOG-ARCHIVE.md` instead of shortening them or adding an exemption. The archive keeps the
original text and only rebases relative links for its new location; `CHANGELOG.md` keeps a link to it,
so a removed or renamed archive fails `DOC002`. Trimming entry text to fit the budget loses the
operational detail the changelog exists to preserve.

The archive is not exempt either. When the archive would itself exceed `markdown_preferred`, move its
oldest closed wave verbatim into a dated archive file beside it — currently
[`docs/CHANGELOG-ARCHIVE-2026-09-10.md`](CHANGELOG-ARCHIVE-2026-09-10.md) — and keep a link to that
file from the archive, so the archive can keep receiving evictions without shortening entry text,
raising a limit, or adding an exemption. A dated archive file is a verbatim record under these same
rules: it is never edited in place, and an entry is filed there exactly once.

Evicted entries are appended in move order at the end of the archive under its
`### Archived from CHANGELOG.md` heading, because they come from the flat `## Unreleased` list and
carry no section of their own. Filing an entry is idempotent: the archive holds exactly one copy of
each evicted entry, and an entry already filed there is never appended a second time. Appending to
EOF without that heading files feature entries under whatever section the file happens to end with.

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
