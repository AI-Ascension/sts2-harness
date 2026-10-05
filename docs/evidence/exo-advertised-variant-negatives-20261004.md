# Exo advertised-variant process evidence — 2026-10-04

[`exo-advertised-variant-negatives-20261004.json`](exo-advertised-variant-negatives-20261004.json)
was emitted by the current-source ignored process test
`advertised_variants_and_zero_model_probes` at harness revision
`03f0422fa11c7110e5479ce6383dc318a53b4c65`. Its output and exit status are in the task log
`orchestration/tasks/harness109-delivery/advertised-variant-oracle-current-03f0422.log`. That log
does not contain the argv or environment; the exact runner receipt is
`orchestration/tasks/harness109-delivery/advertised-variant-oracle-run-receipt-03f0422.json`.
The result was 1 passed, 0 failed.

The report records all five probes: `describe`, `describe_repeated`,
`map_refused_pre_inference`, `management_refused_pre_inference`, and
`tampered_config_rejected`. Each is non-inferencing; the synthetic loopback model observed zero
requests. The actual bridge digest is
`97985d0d77cb0ab70765d7f176a99ec4c442fac1a314d5784ed0f98c9cb29e70`; Exo revision is
`b06869ab789dee3f80ca474b5fa89dbe47ccb859`. Full runtime admission, provider behavior, and native
game behavior remain unverified. The manifest embedding boundary is described in the
[process evidence note](exo-executor-process-oracle-20261004.md).
