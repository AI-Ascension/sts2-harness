// SPDX-License-Identifier: MIT

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapNode {
    pub id: String,
    pub visible: bool,
    pub kind: String,
    pub outcome: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapBundle {
    pub schema: String,
    pub bundle_id: String,
    pub generation: u64,
    pub scope: MemoryScope,
    pub nodes: Vec<MapNode>,
    pub edges: Vec<[String; 2]>,
    pub legal_next_nodes: Vec<String>,
    pub provenance: String,
}

impl MapBundle {
    pub fn validate(&self, scope: &MemoryScope, generation: u64) -> Result<(), MemoryError> {
        let ids: BTreeSet<&str> = self.nodes.iter().map(|node| node.id.as_str()).collect();
        if self.schema != "original.synthetic-map.v1"
            || !valid_id(&self.bundle_id)
            || self.scope != *scope
            || self.generation != generation
            || self.provenance.is_empty()
            || self.provenance.len() > 512
            || self.nodes.len() > 128
            || self.edges.len() > 256
            || ids.len() != self.nodes.len()
            || self.nodes.iter().any(|node| {
                !valid_id(&node.id)
                    || node.kind.is_empty()
                    || node.kind.len() > 64
                    || (!node.visible
                        && node.outcome.as_deref().is_some_and(|outcome| outcome != "unknown"))
            })
            || self
                .legal_next_nodes
                .iter()
                .any(|id| {
                    !ids.contains(id.as_str())
                        || self
                            .nodes
                            .iter()
                            .find(|node| node.id == id.as_str())
                            .is_some_and(|node| !node.visible)
                })
            || self
                .edges
                .iter()
                .any(|edge| !ids.contains(edge[0].as_str()) || !ids.contains(edge[1].as_str()))
        {
            return Err(MemoryError::InvalidEntry);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MapBundleFailpoint {
    BeforeSwap,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AtomicMapBundleStore {
    current: Option<MapBundle>,
    failpoint: Option<MapBundleFailpoint>,
}

impl AtomicMapBundleStore {
    pub fn set_failpoint(&mut self, failpoint: Option<MapBundleFailpoint>) {
        self.failpoint = failpoint;
    }

    pub fn commit(
        &mut self,
        bundle: MapBundle,
        scope: &MemoryScope,
        generation: u64,
    ) -> Result<(), MemoryError> {
        bundle.validate(scope, generation)?;
        if self.current.as_ref() == Some(&bundle) {
            return Ok(());
        }
        if self.failpoint.take().is_some() {
            return Err(MemoryError::PublicationFailed);
        }
        self.current = Some(bundle);
        Ok(())
    }

    pub fn current(&self) -> Option<&MapBundle> {
        self.current.as_ref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapMemoryCandidate {
    pub scope: MemoryScope,
    pub node_id: String,
    pub generation: u64,
    pub visible: bool,
    pub actor_id: Option<String>,
    pub legal_id: Option<String>,
    pub private_actor: bool,
    pub current_authority: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MapMemoryGate {
    scope: MemoryScope,
    generation: u64,
    current_legal_ids: BTreeSet<String>,
}

impl MapMemoryGate {
    pub fn new(
        scope: MemoryScope,
        generation: u64,
        current_legal_ids: impl IntoIterator<Item = String>,
    ) -> Result<Self, MemoryError> {
        if !scope.valid() || generation == 0 {
            return Err(MemoryError::InvalidScope);
        }
        let current_legal_ids = current_legal_ids.into_iter().collect::<BTreeSet<_>>();
        if current_legal_ids.iter().any(|id| !valid_id(id)) {
            return Err(MemoryError::InvalidEntry);
        }
        Ok(Self {
            scope,
            generation,
            current_legal_ids,
        })
    }

    pub fn accept(&self, candidate: MapMemoryCandidate) -> Result<MapMemoryCandidate, MemoryError> {
        if candidate.scope != self.scope
            || candidate.generation != self.generation
            || !valid_id(&candidate.node_id)
            || candidate
                .actor_id
                .as_deref()
                .is_some_and(|actor_id| !valid_id(actor_id))
        {
            return Err(MemoryError::InvalidEntry);
        }
        if !candidate.visible || candidate.private_actor {
            return Err(MemoryError::PermissionDenied);
        }
        let legal_id = candidate.legal_id.as_deref().ok_or(MemoryError::PermissionDenied)?;
        if !candidate.current_authority
            || !valid_id(legal_id)
            || !self.current_legal_ids.contains(legal_id)
        {
            return Err(MemoryError::PermissionDenied);
        }
        Ok(candidate)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MemoryTelemetry {
    pub admitted_sources: u64,
    pub rejected_sources: u64,
    pub retrieval_queries: u64,
    pub retrieval_cache_hits: u64,
    pub summary_jobs: u64,
    pub summary_reviews: u64,
    pub source_invalidations: u64,
    pub retained_bytes: u64,
    pub cleanup_backlog: u64,
}

impl MemoryTelemetry {
    pub fn public_snapshot(&self) -> Value {
        serde_json::json!({
            "schema": "ascension.context-memory.telemetry.v1",
            "admitted_sources": self.admitted_sources,
            "rejected_sources": self.rejected_sources,
            "retrieval_queries": self.retrieval_queries,
            "retrieval_cache_hits": self.retrieval_cache_hits,
            "summary_jobs": self.summary_jobs,
            "summary_reviews": self.summary_reviews,
            "source_invalidations": self.source_invalidations,
            "retained_bytes": self.retained_bytes,
            "cleanup_backlog": self.cleanup_backlog,
            "raw_query": Value::Null,
            "raw_content": Value::Null,
        })
    }
}
