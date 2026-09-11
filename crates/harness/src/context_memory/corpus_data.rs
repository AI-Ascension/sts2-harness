// SPDX-License-Identifier: MIT

#[derive(Clone, Debug)]
pub struct MemoryCorpus {
    scope: MemoryScope,
    entries: BTreeMap<MemoryRef, MemoryEntry>,
    max_entries: usize,
    max_bytes: usize,
    total_bytes: usize,
    generation: u64,
    projection_generation: u64,
    projection_healthy: bool,
    revocation_epoch: u64,
    revoked: BTreeSet<MemoryRef>,
    enabled: bool,
    failpoint: Option<PublicationFailpoint>,
}
