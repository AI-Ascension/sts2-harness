# Per-state requirements records

This directory contains one generated Markdown record for each of the 131 candidate state IDs in
`../data/states.json`. Each record repeats identity, entry/exit, graph, policy-facing legitimate observation,
expert-use, semantic action, transition verification, recovery, memory, and importance-validation
requirements. The records are proposed and target-build validation is required.

Run the generator from the repository root to regenerate them:

```sh
cargo run --locked --package sts2-expert-state-package -- docs/research/sts2-expert-state-package
```

The generated files are not screenshots, live traces, or evidence that a target executable exposes
the listed state. The exhaustive row-level state/field join is in `../data/state-field-matrix.csv`;
these pages render the state policy view rather than duplicating every normalized field row. The
authoritative counts and provenance are in the package root and `data/`.
