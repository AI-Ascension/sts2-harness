// SPDX-License-Identifier: MIT

use super::bundle::{
    BundleManifest, MAP_MAX_BUNDLE_BYTES, MAP_MAX_PNG_BYTES, MAP_MAX_PRESENTATION_HEIGHT,
    MAP_MAX_PRESENTATION_PIXELS, MAP_MAX_PRESENTATION_WIDTH, MAP_MAX_SNAPSHOT_BYTES,
    MAP_MIN_PRESENTATION_HEIGHT, MAP_MIN_PRESENTATION_WIDTH, MapBundleError, MapViewBundle,
    RUNTIME_MAP_SCHEMA_DIGEST,
};
use super::bundle_validation::{
    check_digest, is_digest, validate_file_reference, validate_json_object,
    validate_snapshot_document, verify_optional_digest, verify_required_digest,
};
use super::canonical::canonical_bytes;

pub(super) fn validate_without_bundle_digest(bundle: &MapViewBundle) -> Result<(), MapBundleError> {
    if bundle.snapshot_bytes.len() > MAP_MAX_SNAPSHOT_BYTES {
        return Err(MapBundleError::TooLarge("snapshot"));
    }
    if bundle
        .png
        .as_ref()
        .is_some_and(|bytes| bytes.len() > MAP_MAX_PNG_BYTES)
    {
        return Err(MapBundleError::TooLarge("png"));
    }
    if bundle.manifest.bundle_version != super::bundle::MAP_BUNDLE_VERSION {
        return Err(MapBundleError::UnsupportedVersion(
            bundle.manifest.bundle_version.clone(),
        ));
    }
    validate_manifest(&bundle.manifest)?;
    bundle
        .analysis
        .verify_digest()
        .map_err(MapBundleError::Analysis)?;
    if bundle.analysis.snapshot_digest != bundle.manifest.snapshot_digest {
        return Err(MapBundleError::DigestMismatch("analysis.snapshot_digest"));
    }
    check_digest(
        "snapshot_digest",
        &bundle.manifest.snapshot_digest,
        &bundle.snapshot_bytes,
    )?;
    validate_snapshot_document(
        &bundle.snapshot_bytes,
        &bundle.manifest.schema_profile,
        &bundle.manifest.history,
        &bundle.manifest.map_instance,
        &bundle.manifest.act,
    )?;
    if bundle.manifest.analysis_digest != bundle.analysis.content_digest {
        return Err(MapBundleError::DigestMismatch("analysis_digest"));
    }
    match (
        bundle.manifest.renderer_version.as_str(),
        bundle.viewer.as_deref(),
    ) {
        ("unrendered", Some(bytes)) if bytes == b"{}" => {}
        ("unrendered", _) => return Err(MapBundleError::InvalidField("unrendered viewer")),
        (_, Some(bytes)) if bytes == b"{}" => {
            return Err(MapBundleError::InvalidField("rendered viewer"));
        }
        (_, Some(_)) => {}
        (_, None) => return Err(MapBundleError::DigestMismatch("viewer_digest")),
    }
    match (
        bundle.manifest.renderer_version.as_str(),
        bundle.manifest.presentation.as_ref(),
    ) {
        ("unrendered", None) => {}
        ("unrendered", Some(_)) => {
            return Err(MapBundleError::InvalidField("unrendered presentation"));
        }
        (_, Some(settings))
            if (MAP_MIN_PRESENTATION_WIDTH..=MAP_MAX_PRESENTATION_WIDTH)
                .contains(&settings.width)
                && (MAP_MIN_PRESENTATION_HEIGHT..=MAP_MAX_PRESENTATION_HEIGHT)
                    .contains(&settings.height)
                && u64::from(settings.width)
                    .checked_mul(u64::from(settings.height))
                    .is_some_and(|pixels| pixels <= MAP_MAX_PRESENTATION_PIXELS)
                && !settings.layout_version.is_empty()
                && settings.layout_version.len() <= super::bundle::MAX_BUNDLE_TEXT_BYTES => {}
        (_, _) => return Err(MapBundleError::InvalidField("presentation")),
    }
    if bundle.manifest.contents.snapshot_ref != "visible-map.json"
        || bundle.manifest.contents.analysis_ref != "analysis.json"
        || bundle.manifest.contents.decision_ref != "decision.json"
        || bundle.manifest.contents.viewer_ref != "viewer.json"
    {
        return Err(MapBundleError::InvalidField("content file layout"));
    }
    verify_optional_digest(
        "svg_digest",
        &bundle.manifest.contents.svg_digest,
        bundle.manifest.contents.svg_ref.as_ref(),
        bundle.svg.as_deref(),
    )?;
    verify_optional_digest(
        "png_digest",
        &bundle.manifest.contents.png_digest,
        bundle.manifest.contents.png_ref.as_ref(),
        bundle.png.as_deref(),
    )?;
    verify_required_digest(
        "decision_digest",
        &bundle.manifest.contents.decision_digest,
        &bundle.manifest.contents.decision_ref,
        bundle.decision.as_deref(),
    )?;
    verify_required_digest(
        "viewer_digest",
        &bundle.manifest.contents.viewer_digest,
        &bundle.manifest.contents.viewer_ref,
        bundle.viewer.as_deref(),
    )?;
    validate_json_object(
        bundle
            .decision
            .as_deref()
            .ok_or(MapBundleError::DigestMismatch("decision_digest"))?,
        "decision json",
    )?;
    validate_json_object(
        bundle
            .viewer
            .as_deref()
            .ok_or(MapBundleError::DigestMismatch("viewer_digest"))?,
        "viewer json",
    )?;
    if bundle.manifest.map_instance != bundle.analysis.map_instance
        || bundle.manifest.act != bundle.analysis.act
        || bundle.manifest.history.source_state_id != bundle.analysis.source_state_id
        || bundle.manifest.history.generation != bundle.analysis.generation
    {
        return Err(MapBundleError::IdentityMismatch);
    }
    let mut manifest = bundle.manifest.clone();
    manifest.bundle_digest.clear();
    let bytes = canonical_bytes(&manifest).map_err(MapBundleError::Canonical)?;
    if bytes.len() > MAP_MAX_BUNDLE_BYTES {
        return Err(MapBundleError::TooLarge("manifest"));
    }
    Ok(())
}

pub(super) fn validate_manifest(manifest: &BundleManifest) -> Result<(), MapBundleError> {
    let fields = [
        &manifest.snapshot_digest,
        &manifest.analysis_digest,
        &manifest.map_instance,
        &manifest.act,
        &manifest.run_id,
        &manifest.episode_id,
        &manifest.trajectory_id,
        &manifest.schema_profile,
        &manifest.schema_digest,
        &manifest.analysis_version,
        &manifest.renderer_version,
        &manifest.origin.owner,
        &manifest.origin.source,
        &manifest.origin.generator,
        &manifest.origin.license,
        &manifest.history.source_state_id,
        &manifest.history.action_catalog_digest,
    ];
    if fields
        .iter()
        .any(|value| value.is_empty() || value.len() > super::bundle::MAX_BUNDLE_TEXT_BYTES)
    {
        return Err(MapBundleError::InvalidField("manifest text"));
    }
    if !is_digest(&manifest.schema_digest)
        || !is_digest(&manifest.snapshot_digest)
        || !is_digest(&manifest.analysis_digest)
        || !is_digest(&manifest.history.action_catalog_digest)
    {
        return Err(MapBundleError::InvalidDigest("manifest"));
    }
    if manifest.schema_profile == "runtime-map-v1"
        && manifest.schema_digest != RUNTIME_MAP_SCHEMA_DIGEST
    {
        return Err(MapBundleError::DigestMismatch("schema_digest"));
    }
    if manifest
        .model_execution_id
        .as_ref()
        .is_some_and(|value| value.is_empty() || value.len() > super::bundle::MAX_BUNDLE_TEXT_BYTES)
    {
        return Err(MapBundleError::InvalidField("model_execution_id"));
    }
    for reference in [
        &manifest.contents.snapshot_ref,
        &manifest.contents.analysis_ref,
        &manifest.contents.decision_ref,
        &manifest.contents.viewer_ref,
    ] {
        validate_file_reference(reference)?;
    }
    for reference in [
        manifest.contents.svg_ref.as_ref(),
        manifest.contents.png_ref.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        validate_file_reference(reference)?;
    }
    Ok(())
}
