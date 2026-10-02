# Compatibility: inference-profile resolution

This file holds the compatibility rows for inference-profile resolution at validation and
publication. It is part of the [compatibility policy and matrix](COMPATIBILITY.md); the
classification vocabulary, evidence rules and the remaining rows live there.

## Owner-authoritative resolution at validation and publication

`POST /v1/workflow-definitions/validate` and `POST /v1/studio/drafts/{draft_id}/publish` now resolve
every `decide` / `adaptive_region` profile reference through the owner's served catalog using the
same per-node fences live submission admission already used, and publish that decision — so the
owner's verdict is the single authority a consumer reads, rather than a rule a browser re-derives.

Each entry carries `profile_ref`, the reference exactly as authored, beside `resolved_pin`, the
`profile_id:version:digest` identity it resolved to, plus its graph, node, kind and JSON path. A
consumer wanting an immutable binding records `resolved_pin`; `profile_ref` alone is not immutable,
because a later catalog revision can resolve the same floating id to a different descriptor. Both
surfaces fail closed: a floating id the catalog does not advertise is refused with the catalog's
own `inference_profile_unknown` vocabulary, and publication refuses before creating a definition.

`inference_profiles` is `null` when the owner serves no catalog (no authority exercised — not
"admissible") and an empty list when a catalog was served and the document carries no reference, so
an owner with no catalog is unchanged. This is **breaking for a strict decoder that rejected unknown
response fields**: it must add `inference_profiles`. Evidence is synthetic/component only — in-memory
owner doubles and an in-memory authoring store. No provider, model, credential, native host or
live-owner browser run is claimed.
