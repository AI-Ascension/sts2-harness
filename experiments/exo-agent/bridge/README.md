# Owned bridge placement and dependency boundary

The repository-owned bridge implementation is the Rust `ExoProcessTransport` at
`crates/harness/src/exo_process.rs`. It starts one operator-selected executable, passes only the
allowlisted environment names, writes one `sts2-exo-bridge-wire-v1` request to stdin, shuts stdin
down for EOF, bounds stdout, and maps timeout/non-zero/oversize/malformed outcomes to a
fail-closed transport error. The executable itself is operator-owned and must be supplied at the
configured process path; it is not copied into this repository.

The bridge may depend only on the Rust transport/configuration and the closed wire types exported
by `sts2_harness`. It must not import the Exo CLI, call `/health` or `/request`, open a second
HTTP executor path, invoke a shell, access game/host/loader/mod/save state, or add model/provider
credentials to the wire. The executable/package digest is a required independent preflight axis.

The paired extension package is fixed at
`experiments/exo-agent/extension/package.json` with entry
`experiments/exo-agent/extension/src/index.ts`. Its approved runtime dependencies are the
candidate `@exo/harness` and `@exo/model-runtime` packages only; Node built-ins may be used for
bounded static inputs. Direct OpenAI SDK, Exo CLI, substrate HTTP, shell, game-host, and private
state dependencies are outside the contract. The package manifest is a placement/dependency
boundary fixture, not evidence that the candidate package is installed or executable here.
