// SPDX-License-Identifier: MIT

impl RuntimeV3Port {
    fn launch_mcp(&mut self) -> Result<(), String> {
        let normal_profile = if self.is_expert_profile() {
            "runtime-v3-gameplay"
        } else {
            self.config.runtime_profile.as_str()
        };
        let lookup_discovery_request = match self.config.lookup_binding_enabled() {
            Ok(true) => {
                let Some(owner) = self.lookup_policy_owner.as_ref() else {
                    let release = self.release_lease_inner();
                    return Err(wire::combine_cleanup(
                        String::from("lookup binding requires its selected policy owner"),
                        Ok(()),
                        release,
                    ));
                };
                match owner.lookup_binding_discovery_request() {
                    Ok((binding, request)) => {
                        self.lookup_policy_binding = Some(binding);
                        self.lookup_binding_discovery_request = Some(request.clone());
                        Some(request)
                    }
                    Err(error) => {
                        let release = self.release_lease_inner();
                        return Err(wire::combine_cleanup(error, Ok(()), release));
                    }
                }
            }
            Ok(false) => None,
            Err(error) => {
                let release = self.release_lease_inner();
                return Err(wire::combine_cleanup(error, Ok(()), release));
            }
        };
        let spawned = match lookup_discovery_request.as_ref() {
            Some(request) => McpProcess::spawn_profile_with_lookup_discovery_request(
                &self.config,
                normal_profile,
                request,
            ),
            None => McpProcess::spawn_profile(&self.config, normal_profile),
        };
        let mut mcp = match spawned {
            Ok(mcp) => mcp,
            Err(error) => {
                let release = self.release_lease_inner();
                return Err(wire::combine_cleanup(error, Ok(()), release));
            }
        };
        if let Err(error) = wire::initialize_mcp_profile(&mut mcp, normal_profile) {
            let close = mcp.close();
            let release = self.release_lease_inner();
            return Err(wire::combine_cleanup(error, close, release));
        }
        self.mcp = Some(mcp);
        if self.is_expert_profile() {
            let profile = self.expert_mcp_profile();
            let mut expert = match McpProcess::spawn_profile(&self.config, profile) {
                Ok(expert) => expert,
                Err(error) => {
                    let close = self.mcp.as_mut().map_or(Ok(()), McpProcess::close);
                    let release = self.release_lease_inner();
                    return Err(wire::combine_cleanup(error, close, release));
                }
            };
            if let Err(error) = wire::initialize_mcp_profile(&mut expert, profile) {
                let expert_close = expert.close();
                let normal_close = self.mcp.as_mut().map_or(Ok(()), McpProcess::close);
                let close = match (expert_close, normal_close) {
                    (Ok(()), Ok(())) => Ok(()),
                    (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
                    (Err(first), Err(second)) => Err(format!("{first}; {second}")),
                };
                let release = self.release_lease_inner();
                return Err(wire::combine_cleanup(error, close, release));
            }
            self.expert_mcp = Some(expert);
        }
        Ok(())
    }
}
