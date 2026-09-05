# Licensing and Provenance

## License

This repository is licensed under the MIT License in [`LICENSE`](../LICENSE). Contributions must be
provided by someone entitled to license them under those terms. No separate contributor license
agreement or Developer Certificate of Origin sign-off is currently required.

## Source policy

The harness is an original greenfield target. Do not copy, vendor, transliterate, or use another
harness's implementation source, private paths, generated outputs, decompiled bulk, or symbols as a
product plan. The planning and rust exemplar repositories are standards/evidence inputs only.

No proprietary STS2 game assemblies, game assets, saves, profiles, credentials, provider responses,
model weights, private datasets, or personal identifiers may be committed. A local host installation
may be used only in an explicitly authorized disposable validation lane and remains outside this tree.

## Dependencies

Cargo dependencies are declared in manifests, locked in `Cargo.lock`, and reviewed for source,
license, advisories, feature footprint, platform impact, and redistribution terms. The current
foundation uses `toml` for the target-local policy checker and `serde`, `serde_json`, and `sha2` for
typed POC wire validation and copied-artifact checksums; none are vendored.
The Exo process adapter additionally uses pinned MIT-licensed Tokio for cancellable process I/O.
Its narrow features and lifecycle rationale are recorded in ADR 0006. The newly locked transitive
packages were checked using Cargo metadata; a full automated advisory audit remains a release gate.
Third-party information is summarized in [`THIRD_PARTY_NOTICES.md`](../THIRD_PARTY_NOTICES.md).

## Fixtures, schemas, and artifacts

Every imported or generated fixture, schema, snapshot, model, dataset, or package records:

- source and exact version, commit, or digest;
- license and redistribution permission;
- generator and generator version, when applicable;
- input identity and reproducible generation command;
- sanitization, retention, and deletion status; and
- whether it is normative, test-only, evidence-only, or unverified.

Generated files and large snapshots need an exact policy exemption and nearby provenance record.
Reference implementation source is never eligible for an exemption. Unknown provenance blocks
publication.

## Review obligations

Reviewers must check license headers, dependency lock changes, generated-source provenance, artifact
allowlists, and absence of private/proprietary material. A successful build does not establish the
right to redistribute its inputs.
