use serde::{Deserialize, Serialize};
use strata_gchain_types::{ProcArtifact, ProcError};
use strata_ol_state_support_types::IndexerWrites;

/// Artifact of the index stage: the index writes a link implies.
///
/// Empty for a link the exec stage rejected, since there's nothing to index.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OLIndexArtifact {
    writes: IndexerWrites,
}

impl OLIndexArtifact {
    pub fn new(writes: IndexerWrites) -> Self {
        Self { writes }
    }

    pub fn writes(&self) -> &IndexerWrites {
        &self.writes
    }
}

impl ProcArtifact for OLIndexArtifact {
    fn to_buf(&self) -> Result<Vec<u8>, ProcError> {
        let mut buf = Vec::new();
        ciborium::into_writer(self, &mut buf).map_err(ProcError::encode)?;
        Ok(buf)
    }

    fn from_buf(buf: &[u8]) -> Result<Self, ProcError> {
        ciborium::from_reader(buf).map_err(ProcError::decode)
    }
}
