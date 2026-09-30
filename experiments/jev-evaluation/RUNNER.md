# Bounded independent Jev replay runner

The runner connects the opt-in bridge capture in #371 to the paired audit in #369.
It sends **the same approved input to two independent bridge invocations**, changing only
`model_execution_id`, and imports their redacted sidecars. The bridge path remains a
single argument, including spaces, as fixed in #370.

This is **decision replay**, not game replay: no returned action is dispatched to a game,
gateway or MCP server. No host state is advanced. No model weights, game installation,
save, runtime admission, tactical weight or installed binary is changed. The existing
Rust bridge and capture contract are used without modifications.

Unlike the offline readers, `run` **can make provider calls through the reviewed bridge**.
`plan` and `inspect` never execute it. Execution requires the exact SHA-256 printed by
`plan`; changing any manifest byte requires approval again. An evidence-kind label is
not a network sandbox: even a manifest labelled synthetic invokes its pinned executable
when approved. The runner's own shipped tests use a separate, explicitly synthetic,
socket-free oracle. That oracle stands in for the runner's accounting only; it is not
coverage of the real bridge's transport, which cannot be given a synthetic peer (see
"Testing the compiled bridge" below).

## Prerequisites and private input boundary

Use Unix and a rebuilt, reviewed `sts2-jev-bridge` supporting `--audit-dir`. Native Windows
execution is refused because #371's capture writer has no reviewed Windows ACL boundary.
Do not use a Windows-mounted WSL directory that reports mode 0777. No executable is
downloaded, installed, rebuilt, or admitted by this runner.

Place the manifest and approved original bridge inputs in a private directory owned by
the invoking user, mode `0700`; files must be regular, single-link, mode `0600`. Input
paths are relative to the manifest directory. Absolute paths, traversal, control characters,
backslashes and symlink components are rejected for inputs. Spaces in ordinary path
components are preserved. The output directory must **not exist** and its existing parent
must be private, canonical and user-owned. The runner creates only its new run directory
and its own children. It never chmods, rotates, removes or adopts an existing run.

The bridge locator must be a canonical absolute path to a non-group/world-writable
regular executable file owned by the current user or root. Its file-byte SHA-256 pin
is checked in preflight and again before each arm. These checks do not authenticate
mapped process pages, interpreters, libraries, endpoints or model weights. The bridge
performs the System One exchange in process through its own pinned TLS client, so this
manifest pins one artifact; a manifest that still carries the retired `transport` entry
from `ascension.jev-paired-runner.v1` is refused rather than silently half-honoured.
`source_revision` is a recorded operator assertion, not a verified build attestation.
Use an independently reviewed immutable model pin; denying familiar rolling aliases cannot
establish that a provider will keep an arbitrary model identifier immutable.

Original-input retention and both provider invocations require operator approval. A
sidecar fingerprint cannot reconstruct the input. The runner reads all approved inputs
and verifies their exact file hashes **before** launching anything. It keeps their parsed
values in bounded memory, does not copy them to output, and leaves their original files
untouched. Both arms receive the same JSON serialization except for generated, distinct
ASCII execution IDs. Duplicate keys, malformed UTF-8, unsafe integers, excessive nesting,
invalid catalogs and post-ID-injection payloads over 128 KiB are refused. JavaScript input
serialization is not a claim to reproduce arbitrary Rust `serde_json` byte encodings.

## Manifest

This example is intentionally not executable without replacing paths and digest placeholders.
The exact field names are accepted by `validateRunner`; no ambient defaults widen the plan.

```json
{
  "schema": "ascension.jev-paired-runner.v2",
  "experiment_id": "pilot-001",
  "evidence_kind": "operator_recorded",
  "source_revision": "REVIEWED_40_CHARACTER_SOURCE_COMMIT",
  "model": "REVIEWED_EXACT_MODEL",
  "bridge": {
    "path": "/var/lib/sts2-private/bin/sts2-jev-bridge",
    "sha256": "REPLACE_WITH_64_LOWERCASE_HEX"
  },
  "inherited_environment": ["PATH", "TYPESAFE_API_KEY"],
  "confidence_gate_percent": 20,
  "budgets": {
    "max_pairs": 10,
    "max_provider_attempts": 20,
    "per_arm_timeout_ms": 105000,
    "total_timeout_ms": 1800000,
    "max_total_input_bytes": 2097152
  },
  "output_directory": "/var/lib/sts2-private/pilot-001-output",
  "pairs": [{
    "pair_id": "decision-0001",
    "input_path": "inputs/decision 0001.json",
    "input_sha256": "REPLACE_WITH_64_LOWERCASE_HEX",
    "cluster_sha256": "REPLACE_WITH_64_LOWERCASE_HEX",
    "repetition": 0,
    "split": "held_out"
  }]
}
```

Declare only the environment names required by the reviewed bridge, using its actual
credential-variable names. Values come from the invoking process and are never serialized
by the runner. All other ambient variables are omitted. Empty/missing/oversized values,
duplicate names, common interpreter-injection variables and `JEV_CONTEXT_LOG` are rejected.
This denylist is not proof that an arbitrary bridge cannot log or leak credentials;
the reviewed executable and its dependencies remain trusted. PATH should itself be reviewed.

`cluster_sha256` declares the seed/run/checkpoint family used to prevent split leakage.
It is not inferred from a seed or validated against native history. Repetitions of the
same input must have different preregistered repetition numbers (0 through 99). Duplicate
pair IDs and input/repetition combinations are refused. Cluster IDs, raw input hashes and
semantically identical inputs excluding execution ID cannot cross calibration/held-out
splits. Related but nonidentical states require correct operator-supplied cluster IDs.

## Plan, execute, inspect

```sh
node experiments/jev-evaluation/runner-cli.mjs plan /private/manifest.json
node experiments/jev-evaluation/runner-cli.mjs run /private/manifest.json --approve PRINTED_SHA256
node experiments/jev-evaluation/runner-cli.mjs inspect /private/pilot-001-output
```

The planner hashes files and verifies all local prerequisites without creating output or
starting a child. Its success does not prove transport availability or provider admission.
The executor validates the plan again and checks approval before output creation. The plan
hash proves identity, not the time of preregistration. There is no automatic resume or retry.
Reusing a spent output directory is refused, including after a crash. A new experiment
requires an independently approved manifest and new output directory; the runner does not
enforce organization-wide deduplication across separately approved experiments.

Within a pair, hash-selected AB/BA order is deterministic, not guaranteed to be globally
50/50. Exactly one baseline and one tactical arm are scheduled; each may invoke the bridge
at most once. The full schedule is written before execution. An exclusive per-arm reservation
is written before launch; it consumes one attempt from the frozen plan even if the bridge
later selects a forced action with zero transport invocations. All scheduled arms must
fit the budget up front. Maximums are 256 pairs, 512 reserved attempts, 128 KiB per serialized
input, 32 MiB total original input bytes, 120,000 ms per arm and 3,600,000 ms execution budget.
The execution budget starts after preflight and plan storage, and governs admission of
subsequent arms and their child deadlines; it is not a hard deadline on filesystem operations.
The first scheduled arm is admitted whenever a run starts, and an admitted arm always launches its
child with the smaller of the per-arm timeout and the remaining execution budget.

“Provider attempts” means the bridge's counted **transport invocations**, not authenticated
HTTP arrivals, model inference count, or dollars charged. No hard token or monetary budget
is implemented. A provider may continue processing after a local timeout. Review any
internal transport retry behavior and provider-side limits separately.

## Process and failure behavior

The bridge runs serially in its own Unix process group with `shell: false`, an argument
array and a cleared, explicit environment. Stdin/stdout/stderr are serviced concurrently.
Stdout is bounded to 8 KiB, stderr to 64 KiB, and neither is written to runner artifacts.
The deadline includes unread input pipes and descendants retaining output pipes. Timeout,
cancellation, failure and overflow do not trigger another invocation. The process group
is signalled for termination; local pipe cleanup has a further 1000 ms grace bound. That bound is
host-load-dependent: a same-group descendant's inherited pipes close only once the child's `close`
event fires, which can lag group termination by hundreds of milliseconds under load, so
`child_closed: false` means closure was not confirmed within the bound rather than that cleanup
failed. A `child_closed` flag establishes the direct child's close event and closed pipes, not that
every escaped descendant has terminated. No later arm is launched when closure is unconfirmed.
Descendants deliberately creating new sessions require an external reviewed OS sandbox.

SIGINT and SIGTERM request cancellation. Already reserved work remains accounted for;
remaining arms become `not_started`. SIGKILL, power loss or an I/O failure can leave a
reservation without a terminal journal; `inspect` reports `interrupted_unknown`. That
label does not prove a crash: the original run may still be active. Inspection is read-only,
does not determine process liveness, does not re-run the paired audit and never resubmits work.
Malformed journals are errors rather than missing outcomes. Files are synced, but power-loss
durability remains filesystem-dependent and is not tested here.

The bridge's capture must have the expected profile, binary, requested model, execution
fingerprint, gate and catalog count. Pending and final identities must match. On exit 0,
the returned decision is validated against both the original host catalog and the sidecar;
indices use Rust-compatible UTF-8 catalog ordering. Out-of-catalog, malformed or inconsistent
stdout cannot contribute an action pair. A complete sidecar left by a failed process is
quarantined from the paired manifest. Raw files remain private for investigation. Process
outcomes and capture-validation outcomes are recorded separately.

## Artifacts and interpretation

All runner files are mode `0600` under new `0700` directories. They contain hashes,
bounded numeric diagnostics and generated relative locators, not original inputs, action
IDs, rationale text, stdout, stderr or environment values. The private `pairs.json` also
contains the reviewed model name and sanitized pair IDs required by the existing reader.
Fingerprints can be correlated or guessed; they are not anonymization. This is not an OS
sandbox against a privileged user or a hostile owner racing filesystem replacements.

Artifacts include `run.pending.json`, per-slot reservation/result journals, the bridge's
unchanged pending/result sidecars, `pairs.json`, `audit.json`, separate nonempty
`audit-calibration.json` / `audit-held_out.json`, and `run.result.json`.

The existing capture auditor separates independent agreement from the tactical batch's
internal action question. It keeps refusals, fallbacks, missing arms, failed captures,
model drift and incomparable contexts outside the comparable-action denominator. Failed
stdout validation is also retained in the execution summary even when its sidecar cannot
be admitted to `pairs.json`. Per-arm token, transport-attempt and latency summaries retain
known sums and unknown counts. `elapsed_ms` here covers the child process; `capture_elapsed_ms`
uses #371's decision-calculation measurement. Neither is whole-game latency. Costs are not guessed.

CLI exit 0 means a valid plan, a complete execution/paired audit, or a structurally complete
inspection, depending on the command. Exit 3 returns an incomplete report; exit 2 is an
invalid configuration, artifact or I/O error. None establishes better gameplay. This tool
does not verify native outcome witnesses, add confidence intervals, alter cohort outcome
accounting, promote a policy or replace the Rust benchmark-manifest/receipt validators.

## Validation boundary

```sh
node --test experiments/jev-evaluation/*.test.mjs
cargo run --locked --package repo-policy -- --strict
```

The existing [Node workflow](../../.github/workflows/jev-evaluation.yml) discovers the new
tests automatically; no workflow, Rust source, dependency, provider transport or runtime
argument-admission change is needed. These tests use a hand-authored socket-free bridge
oracle, exact command arguments, real Unix child processes and private scratch directories
under `target/`. The existing shared Rust/Node capture golden is unchanged. These tests
do not execute a newly compiled real bridge, a provider, a game or a Windows ACL writer.

### Testing the compiled bridge

The oracle above is synthetic on purpose, and it is not a stand-in for the real bridge's
transport. `sts2-jev-bridge` performs the System One exchange itself against
`api.typesafe.ai`, with trust anchors compiled in from `webpki-roots` (ADR 0053). Host,
port and root store are compile-time constants with no injection seam, so a synthetic peer
cannot satisfy the binary, and a local server presenting a substituted CA is refused by
design. What the compiled-CI lane does exercise is real: that the compiled binary is
admitted by the runner's digest pin, and that it refuses — without contacting any provider
— on the paths that refuse before an exchange.

Response-class coverage lives in-process instead: framing, status, `chunked` and
`close_notify` against literal bytes in `jev_tls_transport_tests.rs`, catalog and confidence
refusals in `sts2_jev_bridge_tests.rs`, and the ordering only a real peer can show —
handshake completing before a request is written, and a peer that never answers being
refused on the deadline — against a loopback TLS peer in
`jev_tls_transport_loopback_tests.rs`. Those open a real loopback socket. They are not
provider traffic and they do not stand in for it.

The honest gap: **no automated lane exercises a real provider exchange end to end through
the compiled binary.** Closing that would require a test-only seam in production TLS code,
which is a larger change than the coverage it buys.

See the package validation report for actual executed commands and remaining integration gaps.
