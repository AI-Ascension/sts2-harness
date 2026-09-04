# Representative schema regression instances

These five hand-authored, original MIT-licensed synthetic examples exercise the five research
schemas. They are not target-build observations, gameplay traces, provider output, or additional
members of the 655-envelope requirements corpus. Zero-valued digests are deliberate test sentinels,
not measured artifact hashes. The example date identifies this regression set, not a host capture.

`cases.json` lists each schema, baseline fixture, expected validity, and ordered changes. A change
adds or replaces one object member or existing array member at its JSON pointer; its parent must
already exist. Each case starts from a fresh baseline. The committed Rust test applies these
changes and validates the complete instance against JSON Schema Draft 2020-12 with format
assertions enabled. The pinned test-only validator has HTTP/file reference resolution disabled.

| Coverage | Cases |
| --- | ---: |
| One valid full instance of each schema | 5 |
| Unknown root, privileged root, and privileged provenance member, for each schema | 15 |
| Unknown nested member in potion, relic, effect, shop item, standalone intent, and array intent | 6 |
| Denied observation field ID | 1 |
| Action ID in wrong namespace | 1 |
| Unknown transition without reconciliation | 1 |
| Quarantined patch authorizing mutation | 1 |

The observation combines several typed field shapes for serialization coverage, including two
representations of visible intent. It is not a claim that this exact combination occurs in-game.
The original observation schema rejected the defined potion, relic, effect, and shop-item objects
because its value union did not reference them; standalone and array intent values were also absent.

From the repository root:

```sh
cargo test --locked --package sts2-expert-state-package schema_tests
```

The ordinary workspace test command runs the same test in CI. This proves only the listed
serialization/closure constraints. It does not test a policy projector, generation correlation,
taint tracking, operation execution, quarantine promotion policy, or actual host settlement.
The schemas remain proposed research contracts and are not Runtime-v3 wire messages.
