// SPDX-License-Identifier: MIT

fn select_provider_transport(
    config: &RuntimeConfig,
    settings: &RuntimeV3Settings,
    durable: durable::DurableHandle,
    authority_state: lifecycle_authority::RuntimeLifecycleAuthorityState,
) -> Result<lifecycle::RuntimeTransport, String> {
    lifecycle::admit(config, settings, durable, authority_state)
}
