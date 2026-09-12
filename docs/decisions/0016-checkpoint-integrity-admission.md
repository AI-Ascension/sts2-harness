# Checkpoint artifact verification and admission

Status: accepted for local component implementation; native restoration remains unverified.

The harness owns private artifact verification and session admission. An independent adversarial
review showed that missing payload dependencies could receive integrity evidence and that source
and destination compatibility contracts could disagree while a session opened.

Verification now validates the copied frozen checkpoint-manifest schema, including its nonempty
restore closure and closed descriptors. It checks each stored blob digest and declared byte length,
and recomputes the domain-separated state identity from the payload bytes. The existing locked
`jsonschema` dependency moves from test-only to production use; no new version or network resolver
is enabled. Absent fields and unsupported schema versions fail closed. This intentionally rejects
earlier incomplete synthetic manifests; it does not migrate or delete their bytes.

Payload validation additionally checks the frozen generic envelope schema, restricted canonical
bytes, boundary equality, the canonical compatibility-object digest and coverage-contract binding.
ASCII object keys and safe integer values are required before comparing sorted serialization to
the supplied bytes; this comparison rejects duplicate keys and noncanonical lexical encodings.
Fixtures now use the protocol's synthetic canonical payload rather than arbitrary placeholder text.

Session admission compares the verified source compatibility and coverage contracts to the
destination receipt before the restore gate changes epoch. A separately configured destination
gate cannot establish compatibility with a different source. The oracle exercises a valid source
with a separately internally-consistent but incompatible receipt/gate and requires no admission.

Artifact reads and deduplication reads enforce the same 16 MiB bound as writes, including a
bounded read when a file grows after metadata inspection. The tests use a sparse oversized file.

Writes synchronize temporary file contents before atomic publication, then synchronize the
containing directory and its ancestors to persist newly created directory entries. Unsupported
directory synchronization fails as a storage error. Unix temporary files use mode 0600. These
source guarantees do not substitute for a power-loss test on every supported filesystem.

These are integrity and consistency checks, not authentication of the producer or proof of
native coverage, per-phase gameplay semantics, complete native codecs, or a live restore. The
producer/profile admission and phase-specific payload validation remain separate integration
requirements. Public observation and existing replay identity semantics are unchanged.

Validation: `checkpoint_verify`, `checkpoint_session`, `exact_checkpoint_cli`, and
`exact_checkpoint_store` regression suites, followed by mandatory workspace checks.
