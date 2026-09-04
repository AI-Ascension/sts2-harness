# STS2 expert-state fixture corpus

This directory owns the synthetic requirements-fixture module for the STS2 expert-state
research package. It is deliberately separate from live captures, screenshots, gameplay
traces, simulator rollouts, and privileged offline labels.

## Exact corpus contract

The corpus covers the 131 candidate atomic state IDs supplied by the research specification.
Each state occurs exactly once in each of five JSON Lines partitions:

| File | Fixture class | Records | Purpose |
| --- | --- | ---: | --- |
| `normal.jsonl` | `normal` | 131 | Stable ordinary-state baseline |
| `boundary.jsonl` | `boundary` | 131 | Empty, full, minimum, maximum, disabled, or last-choice edge |
| `adversarial.jsonl` | `adversarial` | 131 | Stale, forged, contradictory, out-of-domain, malformed, or test-only hazard |
| `recovery.jsonl` | `recovery` | 131 | Bounded re-observation and mutation reconciliation |
| `patch-regression.jsonl` | `patch_regression` | 131 | Build/schema/entity drift and quarantine |

**Intended total: 655 records (131 states x 5 classes).**

`manifest.json` contains the exact ordered state-ID registry, class/file/count contract,
source hash, generator identity, and required assertion names. A JSONL file contains one
complete JSON object per line and no header.

## Synthetic provenance

Every record is requirements evidence only. Its `provenance` object records:

- `kind: "synthetic"` and `purpose: "requirements_fixture"`;
- `live_capture: false` and `claims_live_observation: false`;
- the source attachment name and SHA-256 used to derive the 131-ID registry;
- deterministic generator name/version and the pinned research date/timezone.

The reference build label is `public-v0.107.1+steam-23811903`. It is a fixture context label,
not proof that any record was captured from that build. No screenshot, pixel signature, host
object dump, RNG state, unrevealed outcome, or gameplay result is present or implied.

## Shared record contract

Each record contains:

- `fixture_id`, `fixture_class`, `state_id`, `evidence_status`, and `build_id`;
- `normalized_observation` with an observation ID, generation, state ID, input status,
  freshness, `stale`, `unknown_state`, and an empty `privileged_fields` **fixture-only
  sentinel**. This sentinel makes a negative test explicit; it is not part of the production
  `GameObservation` schema and must be removed before projection.
- `legal_actions`, whose semantic action references carry the source observation ID and
  generation, plus an explicit target domain with `allowed_ids` and `selected_ids`;
- `execution` with an operation ID, mutation settlement status, at-most-once behavior,
  and reconciliation/retry policy;
- `patch_control` with a known reference build and an unrecognized candidate build that
  cannot authorize autonomous mutation;
- `expected_behavior` and named `assertions`.

The only actions emitted by these fixtures are conservative semantic actions such as
`reobserve`, `safe_halt`, and `reconcile_action`; raw coordinates and keypresses are
outside this module.

## Fixture classes

- **normal** exercises a stable synthetic state and a current-generation safe observation
  action.
- **boundary** exercises a conservative edge condition while keeping mutation disabled.
- **adversarial** puts hazards in a `test_only_injections` envelope. Those sentinels and
  forged references must be dropped or rejected before the production projection. They
  never belong to `normalized_observation` or the legal-action catalog.
- **recovery** marks the observation stale and the prior mutation settlement as
  `delayed_or_uncertain`. Re-observation and read-only reconciliation are required before
  any retry with the same operation identity.
- **patch_regression** keeps the reference build separate from an unknown candidate build.
  Entity/schema/action/fair-play/replay checks are required before promotion; until then,
  autonomous mutation is quarantined.

## Required safety assertions

The universal assertions are intentionally repeated in every partition so a validator can run
one base contract across all 655 records:

- `no_privileged_fields`;
- `no_transformed_privileged_leak`;
- `generation_safe_action_references`;
- `target_domain_subset`;
- `unknown_state_fail_closed`;
- `stale_observation_rejected_before_dispatch`;
- `uncertain_mutation_reconciled_before_retry`;
- `patch_mismatch_quarantines_mutation`;
- `synthetic_not_live_observation`.

The manifest also declares class-specific required assertions. For adversarial records,
`test_only_hazards_are_not_projected` and `test_only_injections` check transformed-leak resistance,
generation mismatch rejection, and out-of-domain target rejection. For recovery records,
`read_only_reconciliation_precedes_retry` makes uncertain mutation handling explicit. For
patch-regression records, `candidate_build_quarantined_before_autonomous_mutation` makes
the release gate explicit.

Unknown states expose only fail-closed behavior: re-observe, wait, reconcile where applicable,
or safe-halt. They must not acquire a generic play, buy, confirm, or map-selection action.

## Validation

The committed Rust checker and negative regression tests run in the normal workspace CI:

```sh
cargo test --locked --package sts2-expert-state-package
```

The validator below is an optional equivalent diagnostic for the envelope contract, not a
runtime execution test. Assertion strings are requirements declarations; their presence does
not prove the named firewall or recovery behavior was exercised. These are five repeated
nonmutating templates, not 131 independent state-specific behavior tests. Their flags describe
synthetic scenarios and can intentionally differ from the proposed state's normal actionability.
Schema-instance examples are separate from this envelope corpus.

From the repository root, validate the corpus with:

~~~sh
python3 - <<'PY'
import json
from pathlib import Path

root = Path("docs/research/sts2-expert-state-package/fixtures")
manifest = json.loads((root / "manifest.json").read_text())
expected_states = manifest["candidate_state_ids"]
assert len(expected_states) == len(set(expected_states)) == 131
assert len(expected_states) == manifest["source"]["candidate_state_count"]
classes = {item["name"]: item for item in manifest["fixture_classes"]}
records = []
for name, spec in classes.items():
    rows = [json.loads(line) for line in (root / spec["path"]).read_text().splitlines()]
    assert len(rows) == spec["records"] == 131
    assert all(row["fixture_class"] == name for row in rows)
    records.extend(rows)
assert len(records) == manifest["expected_records"] == 655
assert len({row["fixture_id"] for row in records}) == 655
assert {(row["state_id"], row["fixture_class"]) for row in records} == {
    (state, name) for state in expected_states for name in classes
}
required = set(manifest["required_assertions"])
class_required = manifest["class_required_assertions"]
for row in records:
    assert row["evidence_status"] == "proposed"
    assert row["provenance"]["kind"] == "synthetic"
    assert row["provenance"]["purpose"] == "requirements_fixture"
    assert row["provenance"]["live_capture"] is False
    observation = row["normalized_observation"]
    assert observation["state_id"] == row["state_id"]
    assert observation["privileged_fields"] == []
    assert observation["generation"] > 0
    for action in row["legal_actions"]:
        assert action["generated_from_observation_id"] == observation["observation_id"]
        assert action["generated_from_generation"] == observation["generation"]
        assert action["observation_generation"] == observation["generation"]
        domain = action["target_domain"]
        assert set(domain["selected_ids"]) <= set(domain["allowed_ids"])
        assert action["mutates_state"] is False
    assert required <= set(row["assertions"])
    assert set(class_required[row["fixture_class"]]) <= set(row["assertions"])
    assert row["patch_control"]["autonomous_mutation_allowed"] is False
    assert row["expected_behavior"]["unknown_state"].startswith("fail_closed")
    assert row["expected_behavior"]["stale_observation"] == "reject_action_and_reobserve"
    assert row["expected_behavior"]["delayed_or_uncertain_mutation"] == "reconcile_read_only_before_any_retry"
    assert row["expected_behavior"]["build_mismatch"] == "patch_quarantine_before_autonomous_mutation"
    if row["fixture_class"] == "adversarial":
        assert row["normalized_observation"]["stale"] is True
        assert "test_only_injections" in row
    if row["fixture_class"] == "recovery":
        assert row["normalized_observation"]["stale"] is True
        assert row["execution"]["mutation_status"] == "unknown"
        assert row["execution"]["settlement"] == "delayed_or_uncertain"
    if row["fixture_class"] == "patch_regression":
        assert row["patch_control"]["migration_status"] == "quarantined"
        assert row["patch_regression"]["candidate_fixture_is_not_promoted"] is True
print(f"validated {len(records)} records across {len(expected_states)} states and {len(classes)} classes")
PY
~~~

This validates parseability, exact counts, fixture-ID uniqueness, state/class coverage,
synthetic provenance, privilege projection, generation-safe action references, target-domain
subsets, and patch quarantine. The corpus remains a requirements baseline until target-build
observation and independent parity experiments are performed. The committed manifest records the
upstream attachment's SHA-256 for provenance; validation intentionally does not require that
external attachment to be present.
