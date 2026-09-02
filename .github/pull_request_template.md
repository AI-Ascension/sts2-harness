## Outcome and reason

Describe the engineering outcome and why this harness change is needed.

## Boundary and compatibility

- Harness responsibility or contract affected:
- Runtime communication boundary affected (MCP, gateway, provider, or artifact store):
- Compatibility classification:
- Intentional divergence or migration:

## Evidence

- Exact local checks and results:
- Deterministic fake-provider/gateway evidence, if applicable:
- Checks not run and why:
- Runtime or game evidence (if any; the harness cannot create it by itself):

## Review checklist

- [ ] The change has one cohesive responsibility and respects the ownership/dependency ADR.
- [ ] Files, functions, and workflows stay within policy budgets.
- [ ] No direct game-process, host-object, loader, or game-mod access was added.
- [ ] No product behavior was implied by a schema, fixture, replay, or model response alone.
- [ ] Identifier namespaces, versions, ordering, cancellation, and artifact lineage are explicit.
- [ ] Prompts, model outputs, credentials, saves, personal paths, and private multiplayer data are redacted.
- [ ] No proprietary host files or copied/reference implementation source is included.
- [ ] Documentation and `CHANGELOG.md` are current for public or operational changes.
- [ ] Remaining risks and unverified claims are explicit.
