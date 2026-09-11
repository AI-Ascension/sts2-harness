# Actual provider CLI capture

`capture.sh` is a bounded local integration check for the first Astra provider boundary. It
renders the checked-in dense `map-bundle-demo-v1` graph with the pinned product renderer, builds a
`sts2.exo-decision-map-v1` request containing that graph and PNG, and sends the serialized request
through a built `sts2-astra-bridge` executable.

During the bridge call, a private temporary `PATH` places a `codex` shim before the real command.
The shim captures its stdin prompt, the bytes named by `--image`, and NUL-delimited argv, then
writes a one-action JSON decision to the exact `--output-last-message` path. The verifier compares
the captured prompt to the request with only `bytes_base64` removed, compares the image bytes to
the renderer output, checks all graph node and edge identities, and checks the bridge's bounded
decision. The temporary directory is removed on success or failure.

The tool fails closed if either required executable is absent, if the renderer hash is not
`782d0d24d795a35335b84dcb8b08458af5e97eeca71b853914cc8b590ad1462f`, if the bridge descriptor
does not advertise the map/image capability, if the fixture is not the full 76-node/182-edge
graph, or if the fake CLI is not invoked exactly once. It does not launch a game, MCP server,
gateway, account, or real provider.

From the harness repository root:

```text
tools/map-provider-capture/capture.sh \
  --bridge <built-sts2-astra-bridge> \
  --renderer <map-visualizer-at-pinned-sha> \
  --fixture crates/harness/tests/fixtures/map-bundle-demo-v1 \
  --report .orchestration/provider-cli-capture.md
```

The report contains only sanitized hashes, synthetic execution identities, counts, and the
reproduction command. Raw prompts, image bytes, temporary paths, credentials, and provider output
are not retained.
