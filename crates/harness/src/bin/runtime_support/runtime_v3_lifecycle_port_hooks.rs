// SPDX-License-Identifier: MIT

impl RuntimeV3Port {
    fn require_lifecycle_lease_authority(&mut self) -> Result<(), sts2_harness::PortError> {
        if self
            .lifecycle_authority
            .activate(&self.config.lease_id, self.config.lease_epoch)
            .is_ok()
        {
            return Ok(());
        }
        let release = self.release_lease_inner();
        Err(wire::port_error(
            "lifecycle_authority_invalid",
            wire::combine_cleanup(
                String::from("validated gateway lease could not be installed as runtime authority"),
                Ok(()),
                release,
            ),
            false,
        ))
    }

    fn observe_lifecycle_authority(
        &self,
        observation: &sts2_harness::EpisodeObservation,
        actions: &sts2_harness::EpisodeLegalActionSet,
    ) -> Result<(), String> {
        self.lifecycle_authority
            .observe(observation, actions)
            .map_err(|_| String::from("runtime authority rejected the MCP observation"))
    }

    fn update_lifecycle_catalog_authority(
        &self,
        actions: &sts2_harness::EpisodeLegalActionSet,
    ) -> Result<(), sts2_harness::PortError> {
        self.lifecycle_authority
            .update_catalog(actions)
            .map_err(|_| {
                wire::port_error(
                    "lifecycle_authority_invalid",
                    "runtime authority rejected the current legal-action catalog",
                    false,
                )
            })
    }
}
