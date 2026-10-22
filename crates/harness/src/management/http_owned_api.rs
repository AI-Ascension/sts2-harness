// SPDX-License-Identifier: MIT

use super::*;

/// The shipped binary uses this before opening any owner journal or constructing live ports.
/// Only the inherited-socket backend invocation constructs the authoritative service.
pub fn owned_management_backend_requested() -> bool {
    #[cfg(target_os = "linux")]
    {
        super::owned_backend::requested()
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// A helper must exit without attempting synchronous diagnostics after its owner channel closes.
/// The frontend already observes its typed channel/exit outcome.
pub fn owned_management_supervisor_requested() -> bool {
    std::env::var("STS2_MANAGEMENT_OWNED_ROLE").as_deref() == Ok("supervisor")
}

/// Run the shipped binary's Linux process-backed management transport.
///
/// The same executable must call this entrypoint before constructing services, and route its
/// backend role to the normal service composition. This is not an arbitrary callback runner.
/// The embedding process never enables subreaper or performs an all-child wait.
/// Inherited ignored SIGCHLD is refused before spawning; the caller must preserve child-wait
/// semantics and must not install an auto-reaper or consume this transport's child exit statuses.
pub fn serve_owned_management(listen: SocketAddr) -> Result<(), HttpError> {
    #[cfg(target_os = "linux")]
    {
        if owned_management_supervisor_requested() {
            return super::owned_supervisor::run();
        }
        if std::env::var_os(super::owned_backend::BACKEND_ROLE).is_some() {
            return Err(HttpError::new(
                "owned_management_role",
                "invalid owned management process role",
            ));
        }
        let mut limits = HttpLimits::default();
        match std::env::var("STS2_MANAGEMENT_DISPATCH_MILLIS") {
            Ok(value) => {
                let millis = value.parse::<u64>().map_err(|_| {
                    HttpError::new(
                        "owned_dispatch_budget",
                        "dispatch budget must be an integer number of milliseconds",
                    )
                })?;
                limits.dispatch_deadline = Duration::from_millis(millis);
            }
            Err(std::env::VarError::NotPresent) => {}
            Err(_) => {
                return Err(HttpError::new(
                    "owned_dispatch_budget",
                    "dispatch budget must be UTF-8",
                ));
            }
        }
        super::owned_frontend::serve(listen, limits)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = listen;
        Err(HttpError::new(
            "owned_management_platform",
            "process-backed management serving requires Linux",
        ))
    }
}

pub(in crate::management) fn serve_owned_backend(
    service: Arc<ManagementService>,
    authenticator: Arc<dyn Authenticator>,
) -> Result<(), HttpError> {
    #[cfg(target_os = "linux")]
    {
        super::owned_backend::run(service, authenticator)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (service, authenticator);
        Err(HttpError::new(
            "owned_management_platform",
            "process-backed management serving requires Linux",
        ))
    }
}
