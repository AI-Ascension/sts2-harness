// SPDX-License-Identifier: MIT

#[test]
fn trusted_render_pins_exo_source_without_conflating_native_model_revision() {
    let fixture = owner_fixture();
    assert_ne!(fixture.runtime_binding.model_revision, sts2_harness::EXO_SOURCE_REVISION);
    let mismatched_config = ExoConfig::new("f".repeat(40), 64 * 1024, 1024, 1_000)
        .expect("valid but unsupported Exo source pin");
    let error = fixture
        .owner
        .render_source_for_decision_with_config(
            &fixture.actor,
            &fixture.request,
            &fixture.snapshot.definition_digest,
            &fixture.runtime_binding,
            &fixture.control_limits,
            &fixture.input,
            &fixture.configuration.context_ref,
            &mismatched_config,
        )
        .expect_err("an unsupported Exo source revision must refuse trusted capture");
    assert_eq!(error.code, "context_render_config_stale");
    let directory = fixture.directory.clone();
    drop(fixture.owner);
    fs::remove_dir_all(&directory).expect("remove closed isolated owner database");
}
