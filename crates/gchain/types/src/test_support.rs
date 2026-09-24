//! Artifact types for exercising the processor traits.

use crate::errors::ProcError;
use crate::processor::ProcArtifact;

/// An artifact with no say in link validity.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct CountArtifact(pub(crate) u32);

impl ProcArtifact for CountArtifact {
    fn to_buf(&self) -> Result<Vec<u8>, ProcError> {
        Ok(self.0.to_be_bytes().to_vec())
    }

    fn from_buf(buf: &[u8]) -> Result<Self, ProcError> {
        let raw = buf
            .try_into()
            .map_err(|_| ProcError::Decode("bad CountArtifact length".into()))?;
        Ok(Self(u32::from_be_bytes(raw)))
    }
}

/// An artifact whose verdict on the link is its payload.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct FlagArtifact(pub(crate) bool);

impl ProcArtifact for FlagArtifact {
    fn to_buf(&self) -> Result<Vec<u8>, ProcError> {
        Ok(vec![self.0 as u8])
    }

    fn from_buf(buf: &[u8]) -> Result<Self, ProcError> {
        match buf {
            [b] => Ok(Self(*b != 0)),
            _ => Err(ProcError::Decode("bad FlagArtifact length".into())),
        }
    }

    fn is_link_valid(&self) -> bool {
        self.0
    }
}
