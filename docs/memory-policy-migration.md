# Durable memory-policy migration

This is the memory-only component increment for Harness #95. The Harness owns policy bytes,
execution validation and activation. Console/Studio consumer operations, a composed browser service,
session-policy migration and final input-budget authoring remain separate work.

## Operations

Construct `MemoryPolicyOwner` with an explicit database path, nonzero authenticated-encryption key,
retention consent and `MemoryPolicyAuthority`. The latter owns the actual corpus, trusted selected
capabilities/pins, phase2 revision/epochs and exact-scope grant registry, plus an authenticator and
trusted clock. Construction and `authority.update` are trusted composition operations, never
request-data adapters.

| Operation | Permission and behavior |
| --- | --- |
| `Import` | Write: retain exact schema-valid source bytes without activation |
| `ProposeMigration` | Write: genuine over-limit source, independently authored newer target, full corpus/profile validation |
| `ProposeRevalidation` | Write: proven active prior adoption, identical bytes/version or newer target, no fake violation |
| `Approve` | Approve: one exact review digest, authenticated subject/grant, no activation |
| `Adopt` | Adopt and still-valid approval permission: same subject/grant/epoch, full fence recheck, atomic binding/history/receipt |
| `inspect_policy` | Content-read: exact stored raw bytes and separate raw/execution hashes |
| `inspect_review`, `active_binding`, `lookup_receipt` | Metadata-read: bounded identity/digest projections |
| `prepare_active` | Select plus continued approval-grant validity: load durable policy and realize actual local selection |

The store is bound to project/run/episode/agent scope; trusted maintenance cannot replace that scope.
It retains policy ID/version separately from
review, approval, binding and operation identity. Active binding version is an adoption counter,
not the target policy version. Revalidation after restart may create binding version 2 pointing at
unchanged policy version 2.

No operation clamps values, generates summaries, invokes a provider or resumes gameplay. A
schema-valid but profile-inadmissible source remains inspectable and inactive. Target validation
uses `MemoryPolicy::validate_against_capabilities` with the actual corpus, then requires the exact
current revision and corpus generation. Missing/revoked catalog entries are not hidden by scalar
ceiling checks. Byte counts do not become token estimates.

## Fences and recovery

Reviews bind source/target identity and raw hashes, target execution hash, selected descriptor digest,
owner/store epoch, phase2 revision/control/plan epoch, corpus generation/revocation epoch, grant
identity/epoch and expected active-binding version. Approval binds the reviewed digest and
authenticated subject. Command bodies cannot supply authority state or an authenticated principal.

The authority lease excludes relevant mutation until SQLite commit. The final trusted-clock check
occurs immediately before commit; wall time can advance while the lease is held. Revocation ordered
before admission prevents activation; revocation ordered after commit preserves history but fences
subsequent preparation. Preparation checks both selector and original approval grants.

Every successfully published trusted maintenance update advances the authority-owned owner epoch,
including a no-op. An explicit larger epoch is preserved; rollback, overflow and failed updates
publish nothing. This prevents corpus reconstruction, disable/reset or descriptor/revision
A-to-B-to-A changes from restoring an old approval. Corpus disable may still reset its own counters.
After any successful maintenance, old reviews and active bindings require explicit revalidation,
approval and adoption; do not call `update` as a read-only health check.

Accepted effects and receipts share one transaction. A before-commit failure leaves neither;
after-commit lost reply leaves both. Recover by original subject/idempotency key. Reusing that key
with changed content conflicts. Reopening authenticates history before incrementing the persistent
owner epoch. Wrong-key/corrupt opens do not fence the healthy owner; a legitimate replacement does.
Old handles cannot reclaim ownership. Restart requires explicit revalidation/approval/adoption
before preparation and never silently resumes execution.

## Fixed storage bounds

| Resource | Bound |
| --- | ---: |
| Each exact source/target encoding | `min(MEMORY_POLICY_MIGRATION_MAX_BYTES, 65_536)` bytes |
| Saved policy versions | 128 |
| Reviews / approvals / adoptions | 64 each |
| Grants | 64 |
| Receipts | 512 |
| Serialized encrypted-journal plaintext | 16 MiB |
| Ciphertext envelope | 16 MiB + 40 bytes |
| SQLite main database | 32 MiB, 4-KiB pages, at most 8,192 pages |
| SQLite busy wait | 1 second |

All retained serialized bytes count toward the journal ceiling. There is one journal row rather
than an unbounded SQL collection. Before decoding, SQLite reports its BLOB length; typed sequence
readers stop at their record count limits. Source encodings are independently bounded and strict.
The store uses rollback-journal mode rather than WAL; transaction journal overhead is not zero and
is bounded by the database being changed. There is no automatic history deletion, cleanup or
resource-ceiling increase. Capacity errors roll back attempted changes.

Raw bytes and credentials are not included in Debug output or receipts. Policy source/target bytes
are encrypted before SQLite writes. Metadata-only reads expose identities and hashes, not raw
policy encodings. Synthetic test authorization does not establish permission to retain private data.

## Verification and evidence

Focused test entrypoint:

```sh
cargo test --locked -p sts2-harness \
  --test context_memory_policy_owner \
  --test context_memory_policy_store \
  --test context_memory_policy_races \
  --test context_memory_policy_limits \
  --test context_memory_policy_maintenance \
  --test context_memory_policy_history
cargo test --locked -p sts2-harness --lib \
  context_memory::policy_owner
```

Tests exercise actual corpus validation/selection and encrypted SQLite, with synthetic credentials,
policies and clocks. Cases include exact-byte preservation, distinct raw/typed hashes, schema/profile
rejection, missing catalog entries, target-bound approval, restart/revalidation, no downgrade,
idempotency, lost reply, wrong key/corruption, quota/physical limits and serialized revocation.
These are component tests, not a provider/native/browser-production acceptance claim.

Run repository policy, locked format/clippy/build/workspace gates before review. The approved
[ADR](decisions/0019-memory-policy-owner-store.md) defines the internal compatibility boundary.
Future consumer migration operations and a standalone synthetic Console/Harness/browser fixture
require separately reviewed capabilities, schemas/authentication mapping and exact dependency pins.
Harness core must not depend on Console implementation or invent an effective-record endpoint.
