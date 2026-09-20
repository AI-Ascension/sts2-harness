# Context-control catalog limits

The authenticated management `GET /v1/context-bindings` returns existing
`ascension.context-control.owner-catalog.v1` metadata containing closed
`ascension.context-control.owner-binding.v1` descriptors. Management checks `workflow:read`,
calls the owner catalog port and validates the returned catalog. These are distinct from the
Console Phase 2 control capabilities and memory/session v3 capabilities.

`management/context_owner_support.rs::validate_limits` validates the following descriptor
ceilings. They are not minimum draft content counts.

| Field | Minimum | Maximum/default | Current source enforcement |
|---|---:|---:|---|
| `max_items` | 1 | 64 | Renderer uses fixed maximum 64 selected references. |
| `max_notes` | 0 | 16 | Renderer uses fixed maximum 16 notes. |
| `max_context_bytes` | 1 | 131072 | Renderer bounds each input/schema/configuration buffer separately. |
| `max_objective_bytes` | 1 | 512 | Renderer bounds explicit draft objective bytes. |
| `max_control_events` | 1 | 4096 | Journal recovery bound and event recording saturation. |
| `output_reserve_bytes` | absent | absent | Optional; when present it is 1..=8192 and makes `max_context_bytes` the combined bound. |

The first four rows are always present. `output_reserve_bytes` is optional and skip-serialized: absent
is the pre-existing contract, where `max_context_bytes` bounds the input bytes alone and response
capacity stays bounded independently by the provider configuration. When it is present,
[ADR 0058](decisions/0058-served-whole-input-output-reserve.md) makes `max_context_bytes` the combined
whole-input bound, admits the assembled provider bytes against it before any dispatch, and refuses an
unusable advertised reserve rather than treating it as unlimited.

The source accepts restricted descriptor values, and
[ADR 0030](decisions/0030-context-owner-effective-limits-composition.md) now composes those selected
values with the owner's current run binding and publishes them over
`GET /v1/workflow-runs/{run_id}/context-owner-effective-limits` through the same fail-closed seam
that live admission uses. Reading them does not pass them into the renderer or the event journal, so
catalog validation and this projection together still do not establish selected-limit enforcement.
Default values match current constant guards; no capacity was raised. Self-digests bind complete
descriptor/catalog content but cannot authenticate a supplied owner. Disabled metadata is retained
and cannot be selected by `descriptor_for`.

[Producer fixtures](../fixtures/context-control/README.md) exercise all five numeric descriptor
minima/maxima and restricted values, valid default/restricted/zero-note/disabled catalogs, and
digest consistency. The generator invokes actual producer types and methods. Its separate
generation evidence records the candidate source identity independently from the fixture's
historical contract-origin revision; that pinned fixture predates `output_reserve_bytes`, which is
exercised by the library and served-boundary tests in this repository rather than by a regenerated
fixture. Consumer parity and pin adoption remain separate work.

Other limits must not be collapsed into these fields:

- `context_control/types.rs` bounds individual note bytes at 4096. This is absent from the
  catalog and is not newly advertised here.
- Context-control draft schema bounds selected/pinned references at 64 and notes at 16.
  Patch schema bounds operations at 32 and note/objective text at 4096/512 characters.
  JSON Schema string lengths and executable UTF-8 byte lengths differ.
- Pins must be selected. Reference integrity, expiry, protected content and UTF-8 validity
  are independent checks. The only relation introduced between catalog fields is the one
  `output_reserve_bytes` states: it is subtracted from `max_context_bytes` when present. No
  relation between the objective ceiling and the whole-context ceiling is introduced, and concrete
  final serialization must still fit on its own.
- Console owns its additional total-note and facade transport bounds. These are not Harness
  prepared-input capacity and cannot be inferred from this catalog.
- Binding catalog entries (128) and descriptor sources/operations/node-kind counts (16)
  are metadata resource guards rather than authorable content budgets.

Compatibility: this increment adds only a catalog sealing convenience method and offline
fixtures/tests, and ADR 0030 adds one versioned read-only route plus a shared composition helper;
existing validation, public fields, schema versions and endpoints remain unchanged. ADR 0058 adds
one optional field under the review this sentence requires: it defaults to absent, is
skip-serialized, and raises no ceiling. Future publication of additional limits/provenance
requires an explicitly reviewed versioned contract. Selected-limit rendering/event behavior,
saved-policy migration, input token budgets and composed browser-to-owner execution remain
unverified by these tests.
