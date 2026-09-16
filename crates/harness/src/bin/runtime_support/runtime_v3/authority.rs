// SPDX-License-Identifier: MIT

use super::super::{config::RuntimeConfig, runtime_v3_settings::RuntimeV3Settings};

/// Returns the byte-derived runtime configuration identity used by durable
/// execution records. The served workflow adapter uses the same identity when
/// it creates its immutable context-authority provenance.
pub(crate) fn authority_configuration_digest(
    config: &RuntimeConfig,
    settings: &RuntimeV3Settings,
) -> Result<String, String> {
    super::durable::authority_configuration_digest(config, settings)
}
