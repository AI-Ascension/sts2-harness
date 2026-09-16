# ADR 0039: scoped game-information lookup consumer

Status: accepted for the harness-owned consumer and deterministic tool-loop contract.
Owner: sts2-harness. Related issue: [#127](https://github.com/AI-Ascension/sts2-harness/issues/127).

The consumer admits `game-information-query-v1` at protocol commit
`34f68b182c09472c3a0573ff478e17e6ed53c91f`, schema SHA-256
`376845b0c86b4afcd2c79ffba753eb7e7e416f5410da26b4dae970cfee2221d9`.
The exact inert schema is imported with provenance and checksum inventory. The existing
Rust protocol dependency and closed expert/Exo profiles are unchanged.

`LookupSession::query_port` obtains correlation from the existing MCP port, validates the
request against an approved `MemoryPolicy` and owner `LookupBinding`, dispatches one fixed
read, validates the entire source before projection, and retains accepted raw bytes in
`MemoryCorpus`. `RuntimeV3Port` implements `LookupMcpPort` using its existing MCP process,
RPC identity sequence, read budget and cleanup owner. The mapper uses the MCP #51 fixed
flattened argument shapes, never a host route or arbitrary tool name.

`negotiate_port` reads the selected producer capability envelope before query admission.
The runtime accepts the opt-in `negotiated-composition-v1` catalog beside its unchanged
legacy catalogs. Mixed catalogs must retain ordinary gameplay and discovery, reject stale
negotiation, duplicate/unknown tools and foreign lookup revisions or privilege hints, and
may omit unavailable lookup operations. The MCP #52 mapping reviewed is its merged
composition implementation at merge commit `03be7729635376325f2f6ee7c47c35896a742e0f`.
The reviewed catalog argument schema, request mapper, composition revision mapper and server
catalog envelope were independently compared byte-for-byte with that immutable GitHub commit.
Actual peer-process interoperability is a separate evidence lane.

`run_lookup_tool_loop` drives the additive `LookupAgentPort` contract for at most 32 turns:
typed lookup requests, bounded retained-source reads and a final action ID. Game information
arrives only in the feedback data channel; the owner binding and legal-action set are separate.
The final ID must belong to that unchanged host set. The loop returns a decision and never
dispatches a gameplay effect. Ordinary episode freshness and settlement remain required.

The local binding separately records project/run/episode/agent, game profile, content manifest,
locale, authority epoch and current snapshot. These owner facts do not become protocol fields.
Static queries have no game-run or snapshot requirement in their protocol binding. The existing
MCP route still requires a selected instance and lease; unallocated content access is unsupported.
Live detail must match the current owner snapshot and run. An observation change clears page
chains; revocation/restart/content/profile changes invalidate capabilities and require a new
session. A stale result is a typed `Reobserve`, never a replacement observation or action.

Only public static reference and player-visible live data are admitted. Research/profile lanes
are excluded. The closed schema rejects extra hidden/seed fields; all typed fields, availability,
source references, UTF-8 accounting, ordering, query and snapshot fences are validated.
The negotiated field and byte limits apply to the whole response, including fields omitted
from the optional view. No text classifier claims to prove text truthful: all game/mod/localized
text remains explicitly untrusted data. It cannot populate system instructions or action IDs.
The host legal-action set remains the separate decision authority.

The retained-source ceiling is the existing 64 KiB `MemoryCorpus` source limit, additionally
bounded by producer limits. There are at most 16 pages per operation and 256 session records.
All pages retain exact query, sequence, scope and source bytes, with cross-page duplicate,
cursor-cycle, ordering and total checks. The optional view uses the existing policy byte budget.
Larger accepted payloads yield a source reference with exact length/digest; bounded raw-byte
chunk reads expose the complete source without invented summaries or truncation. Chunks may
split UTF-8 and must be reassembled as bytes before text decoding.

`ascension.game-information-record.v1` is an additive harness record, separate from old closed
trajectory wire formats. Raw source SHA-256 and derived view SHA-256 remain distinct from the
schema digest and content-manifest identity. Errors are sanitized and recorded; rejected raw
payloads are not retained. Replay has no transport callback. It checks owner/content/query pins,
source retention and hash, full validation and derived-view identity. Absent, revoked or expired
retention is explicit. Callers supply the retention clock/window; session construction does not
read wall time. The caller must construct a fresh session with the current clock for later reads.

`export_archive` publishes the exact source entries through the existing encrypted
`DurableMemoryStore`, verifies the persisted sources, and returns a bounded
`ascension.game-information-archive.v1` manifest and digest. The caller retains that manifest
through its artifact owner and pins its digest independently. `import_archive` restores a new
corpus/session from the manifest and companion encrypted store; no MCP callback exists on
this path. The manifest is limited to 256 records and 262,144 bytes. Retain the store, encryption
key and policy dependencies separately; an absent dependency is explicit, never fetched today.
Historical records preserve their original snapshots even after the owner observes a newer one.

## Evidence and exclusions

Synthetic component tests exercise the production tool loop and `query_port` path, fixed MCP
mapping and wrapper validation, static lookup followed by live detail, a choice admitted by
`EpisodeLegalActionSet`, pages and retained chunks, pinned replay, missing retention, wrong
agent/profile, stale bindings, hidden fields and oversized results. Injection-like fixture text
remains a field value and cannot introduce a legal action. Archive tests close/reopen an actual
encrypted SQLite store and reproduce the exact original source bytes, including whitespace,
through a new session, with wrong owner/profile, missing/tampered retention and manifest failures.

These tests do not run an actual MCP/gateway process, game or provider. The new generic agent
port and mixed runtime adapter are production APIs; the existing native Exo process adapter
still speaks its frozen terminal-decision-only protocol. Translating that native provider's
tool calls to this additive agent API remains unfinished integration work, not a claimed
external blocker. The runtime does not automatically select the new tool loop for old Exo
profiles. Automatic producer lifecycle notification handling remains with the existing MCP
process boundary; failed/stale reads fail closed and do not silently renegotiate or requery.
Issue closure additionally requires merged owner evidence and exact-build peer/native acceptance.

| Mode / visibility | Consumer support | Evidence |
| --- | --- | --- |
| Static public, selected instance, no active game run | list/search/get/availability under negotiated capability subset | synthetic |
| Live player, exact run/epoch/snapshot | detail/availability under negotiated capability subset | synthetic |
| Hidden/profile/research | rejected | negative fixtures |
| Unknown kinds/fields, over negotiated or local bounds | explicit rejection | negative fixtures |
| Replay at original content and owner scope | encrypted retained bytes, no network fallback | SQLite restart fixtures |
| Unallocated static / native Exo tool translation / exact-host gameplay | unsupported or unfinished as above | unverified |
