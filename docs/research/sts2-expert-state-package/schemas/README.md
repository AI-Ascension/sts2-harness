# STS2 expert-state schemas

This directory is a research-package artifact owned by `sts2-harness`. It records the proposed
observation, action, decision, transition, and patch-validation shapes for the STS2 expert-state
research baseline. It is not a game adapter, host authority, MCP/gateway contract, or cross-
repository protocol.

## Artifacts

| File | Purpose |
| --- | --- |
| `game-observation.schema.json` | Closed, fair-play-projected policy observation. |
| `legal-action.schema.json` | Host/state-derived semantic action with typed target domain. |
| `decision-record.schema.json` | Auditable decision facts, estimates, rationale, and outcome. |
| `state-transition.schema.json` | Dispatch, settlement, postcondition, and recovery record. |
| `patch-manifest.schema.json` | Build/profile drift, validation gates, and quarantine record. |

All five schemas use JSON Schema Draft 2020-12 and have independent schema versions and `$id`
values. All record roots bind their schema version; only `PatchManifest` carries `schema_digest`.
An enclosing artifact manifest must bind the other records to their exact schema digest before
reproducible evaluation. Schema, harness, game, mod, protocol/profile, evaluator, and provider
versions remain separate. Required-field, enum,
namespace, or closed-object changes are breaking unless an explicitly reviewed compatibility
profile says otherwise.

## Boundaries and provenance

Envelope identities use namespace-qualified `{namespace, value}` pairs. Local entity references
and observation/action/decision/transition build/profile references remain bounded strings;
`PatchManifest` uses namespaced build/profile identities. These encodings require explicit mapping,
not interchangeability. Run, episode, trajectory, instance, session, observation, action, operation,
transition, trace, decision, and model-execution identities remain distinct. A successful parse is serialization
evidence only; it does not prove game behavior, target-build parity, or a settled effect.

State identities use the package's lower-snake candidate IDs. The observation schema enumerates all
131 baseline IDs; action, decision, and transition schemas enforce the same lower-snake shape, and
patch manifests use a dedicated `sts2.state` identity whenever a changed entity has `kind: "state"`.

Records carry build/profile provenance, source IDs, capture time, evidence status, epistemic status,
confidence, and independent-verification state. Missing build bytes, hashes, or live observations
remain explicit and must not be replaced with predecessor-game assumptions.

`GameObservation` is the production policy input. Its `fair_play_projection` has only the allowed
classes `VIS_DIRECT`, `VIS_ON_DEMAND`, `OBS_HISTORY`, `DERIVED_EXACT`, and `ESTIMATED`, represented
by closed, enumerated field entries. There is no generic privileged production object. The
`PRIVILEGED` class—hidden RNG, unrevealed outcomes, hidden map content, and similar values—is
blocked before serialization. The offline `privileged_offline_label` definition is deliberately
unreachable from each production schema root and belongs in a separate artifact/process; it is
never policy-visible. Schema closure is necessary but does not replace runtime taint, allowlist,
and transformed-leak tests.

Generation and freshness are explicit. Actions bind to the observation ID and generation that
produced them. `accepted` means a boundary accepted a request; it is not `settled`. Mutation-bearing
records use bounded dispatch counts and the settlement states `proposed`, `admitted`, `dispatched`,
`accepted`, `settling`, `settled`, `rejected`, `cancelled`, `unknown`, and `reconciled`. An uncertain
irreversible mutation requires read-only reconciliation before retry.

`PatchManifest.quarantine` is fail-closed: `quarantined` and `blocked` require
`autonomous_mutation_allowed: false`; `validated` requires it to be true. Public/live, beta, and
offline lanes are separately identified and cannot be silently pooled.

## Deterministic validation examples

Run from the repository root in an environment with `jq` and Python `jsonschema` installed:

```bash
for schema in docs/research/sts2-expert-state-package/schemas/*.schema.json; do
  jq empty "$schema"
done
```

Validate every document against the Draft 2020-12 meta-schema:

```bash
python3 - <<'PY'
import json
from pathlib import Path
from jsonschema import Draft202012Validator

root = Path("docs/research/sts2-expert-state-package/schemas")
for path in sorted(root.glob("*.schema.json")):
    schema = json.loads(path.read_text())
    Draft202012Validator.check_schema(schema)
    print(f"valid schema: {path}")
PY
```

The following negative check demonstrates recursive root closure. A real fixture must also satisfy
the selected schema's required fields; this check is only for the forbidden unknown property:

```bash
python3 - <<'PY'
import json
from pathlib import Path
from jsonschema import Draft202012Validator

for path in sorted(Path("docs/research/sts2-expert-state-package/schemas").glob("*.schema.json")):
    schema = json.loads(path.read_text())
    errors = list(Draft202012Validator(schema).iter_errors({"unknown_privileged_path": 1}))
    assert any(error.validator == "additionalProperties" for error in errors), path
    print(f"closed root: {path}")
PY
```

These checks validate schema syntax and declared closure only. Target-build discovery, fair-play
exposure parity, simulator parity, and runtime settlement remain separate validation gates.

## Representative schema instances

[`../fixtures/schema/README.md`](../fixtures/schema/README.md) defines valid instances and negative
mutations for all five schemas. These are separate from the 655 simplified requirements envelopes,
which do not validate against these schemas. The representative observation exercises typed potions,
relics, effects, shop items, and both standalone and array intent values. None are host captures.

The inventory's aggregate field IDs (for example, `run.resources`) are not schema field IDs
(`run.hp_current`, `run.gold`, and others). Its lowercase candidate action names likewise need an
explicit mapping to schema action enums. No inventory-to-schema projection is implemented or
validated by this package. Schema closure constrains allowed shapes; it does not enforce field-ID
specific value types, cross-record generation equality, target membership, action settlement
truth, or privileged-information provenance. Those require an independently tested interpreter.
