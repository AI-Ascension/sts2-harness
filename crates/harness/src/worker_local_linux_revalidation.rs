// SPDX-License-Identifier: MIT

//! Repeat native identity verification over the original held connection.

use rustix::fd::AsFd;
use tokio::time::Instant;

use super::worker_linux_verifier::{VerifierFailure, VerifierOutcome};
use super::worker_local_linux_image::HeldImage;
use super::{LinuxPeerWitness, LinuxTransportError, LinuxWorkerListener};

impl LinuxWorkerListener {
    pub(super) async fn verify_peer(
        &self,
        stream: &impl AsFd,
        deadline: Instant,
    ) -> Result<LinuxPeerWitness, LinuxTransportError> {
        let endpoint = self.endpoint.duplicate_verifier_path()?;
        let outcome = self
            .verifier
            .verify(stream, &self.peer, &self.image, endpoint, deadline)
            .await
            .map_err(|error| match error {
                VerifierFailure::Configuration => LinuxTransportError::Configuration,
                VerifierFailure::Busy => LinuxTransportError::Busy,
                VerifierFailure::Deadline => LinuxTransportError::Deadline,
                VerifierFailure::Poisoned | VerifierFailure::Io => LinuxTransportError::Io,
            })?;
        match outcome {
            VerifierOutcome::Accepted { pidfd, image_fd } => Ok(LinuxPeerWitness::from_verified(
                pidfd,
                self.peer.pid,
                self.peer.uid,
                HeldImage::from_verified_fd(
                    image_fd,
                    self.peer.uid,
                    self.image.identity(),
                    &self.peer.executable_sha256,
                )
                .map_err(|_| LinuxTransportError::Io)?,
            )),
            VerifierOutcome::Rejected => Err(LinuxTransportError::Peer),
            VerifierOutcome::Closed => Err(LinuxTransportError::Closed),
        }
    }
}
