use serde::{Deserialize, Serialize};
use strata_gchain_types::{ProcArtifact, ProcError};
use strata_ol_chain_types_v1::{OLBlockHeaderV1, OLLog};
use strata_ol_state_types_v1::WriteBatch;
use strata_serde_utils::{SerdeCodec, SerdeSsz};

use crate::graph_types::OLStateNode;

/// What the exec stage produced for a valid link.
///
/// The fields keep their protocol encodings inside the serde shims, so the
/// artifact as a whole is just a CBOR container around them.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OLExecOutput {
    /// The header of the block the link arrives at.
    ///
    /// For a block link this is just the block's header.  For a checkpoint
    /// link it's the terminal header reconstructed from the payload, which is
    /// the only place it exists.
    header: SerdeSsz<OLBlockHeaderV1>,

    /// The diff from the origin node's state to the target node's.
    write_batch: SerdeCodec<WriteBatch>,

    /// Logs emitted along the link, in emission order.
    logs: SerdeSsz<Vec<OLLog>>,
}

impl OLExecOutput {
    pub fn new(header: OLBlockHeaderV1, write_batch: WriteBatch, logs: Vec<OLLog>) -> Self {
        Self {
            header: SerdeSsz::new(header),
            write_batch: SerdeCodec::new(write_batch),
            logs: SerdeSsz::new(logs),
        }
    }

    pub fn header(&self) -> &OLBlockHeaderV1 {
        self.header.inner()
    }

    pub fn write_batch(&self) -> &WriteBatch {
        self.write_batch.inner()
    }

    pub fn logs(&self) -> &[OLLog] {
        self.logs.inner()
    }

    /// The node the link arrives at.
    pub fn target_node(&self) -> OLStateNode {
        OLStateNode::from_header(self.header())
    }
}

/// Artifact of the exec stage: the output for a valid link, or why the link
/// was rejected.
///
/// The rejection reason is kept as its rendered message, since the artifact
/// is a record of the verdict rather than an error to act on.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[expect(
    clippy::large_enum_variant,
    reason = "the executor holds artifacts behind an Arc, so boxing the output would only add an indirection"
)]
pub enum OLExecArtifact {
    Valid(OLExecOutput),
    Invalid(String),
}

impl OLExecArtifact {
    /// The output, if the link was valid.
    pub fn output(&self) -> Option<&OLExecOutput> {
        match self {
            Self::Valid(output) => Some(output),
            Self::Invalid(_) => None,
        }
    }

    /// Why the link was rejected, if it was.
    pub fn invalid_reason(&self) -> Option<&str> {
        match self {
            Self::Valid(_) => None,
            Self::Invalid(reason) => Some(reason),
        }
    }
}

impl ProcArtifact for OLExecArtifact {
    fn to_buf(&self) -> Result<Vec<u8>, ProcError> {
        let mut buf = Vec::new();
        ciborium::into_writer(self, &mut buf).map_err(ProcError::encode)?;
        Ok(buf)
    }

    fn from_buf(buf: &[u8]) -> Result<Self, ProcError> {
        ciborium::from_reader(buf).map_err(ProcError::decode)
    }

    fn is_link_valid(&self) -> bool {
        matches!(self, Self::Valid(_))
    }
}
