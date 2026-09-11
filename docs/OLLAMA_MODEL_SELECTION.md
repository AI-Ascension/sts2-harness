# Selecting an Ollama model

The `sts2-ollama-bridge` accepts a user-selected Ollama model identifier. It sends the identifier
unchanged in the `/api/chat` request to the existing loopback endpoint `127.0.0.1:11434`.
There is no model allowlist and no fallback to another model. Omitting `--model` preserves the
legacy default `gemma4:31b-cloud`.

Build and inspect the selected configuration without making a provider request:

```sh
cargo build --locked --package sts2-harness --bin sts2-ollama-bridge
target/debug/sts2-ollama-bridge --model team/custom-model:7b --describe
```

The model in this example is illustrative. Install or configure your chosen model in your Ollama
service separately. `--describe` reports requested configuration, not model availability or proof
that inference used that model. Identifiers must be nonempty, at most 240 UTF-8 bytes, contain no
whitespace/control characters, and must not start with `-`. Unknown, duplicate, or malformed options
fail before input is read or a connection is opened.

For an already configured, authorized runtime-v3 combat fixture, set the bridge arguments as a JSON
array alongside its exact binary digest:

```sh
export STS2_PROVIDER_KIND=ollama
export STS2_COMBAT_DEMO=true
export STS2_EXO_BRIDGE_BINARY="$(pwd)/target/debug/sts2-ollama-bridge"
export STS2_EXO_REVISION="$(sha256sum "$STS2_EXO_BRIDGE_BINARY" | cut -d ' ' -f 1)"
export STS2_EXO_BRIDGE_ARGS_JSON='["--model","team/custom-model:7b"]'
```

These settings are only the provider portion of the runtime configuration. They do not launch the
game or replace gateway/MCP configuration, leases, or fixture authorization. The runtime accepts
only the exact two-element model option for this bridge; `--describe` is not an execution argument.
The existing Astra-only live-episode mode is unchanged. Other providers need their own compatible
adapter; accepting an arbitrary Ollama model name is not universal provider support.

The chosen model must follow the structured decision schema and return an action from the supplied
legal-action catalog. The bridge continues to reject invalid decisions and transport failures.
Record the requested model with your experiment configuration; a response alone does not establish
model identity, gameplay correctness, or broader compatibility. Offline tests check option parsing,
runtime argument admission, and exact selected-model/captured-body equality using a synthetic
loopback HTTP server, without invoking Ollama or a game.
