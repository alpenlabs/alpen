use borsh::BorshDeserialize;
use revm_primitives::{FixedBytes, B256};
use strata_primitives::evm_exec::EVMExtraPayload;
use strata_state::block::{L2Block, L2BlockBundle};
use thiserror::Error;

pub(crate) struct EVML2Block {
    #[allow(dead_code)]
    l2_block: L2Block,
    extra_payload: EVMExtraPayload,
}

impl EVML2Block {
    /// Attempts to construct an instance from an L2 block bundle.
    pub(crate) fn try_extract(bundle: &L2BlockBundle) -> Result<Self, ConversionError> {
        let extra_payload_slice = bundle.exec_segment().update().input().extra_payload();
        let extra_payload = EVMExtraPayload::try_from_slice(extra_payload_slice)
            .or(Err(ConversionError::InvalidExecPayload))?;

        Ok(Self {
            l2_block: bundle.block().to_owned(),
            extra_payload,
        })
    }

    /// Compute the hash of the extra payload, which would be the EVM exec
    /// payload.
    pub(crate) fn block_hash(&self) -> B256 {
        FixedBytes(*self.extra_payload.block_hash().as_ref())
    }
}

#[derive(Debug, Error)]
pub(crate) enum ConversionError {
    #[error("invalid EVM exec payload")]
    InvalidExecPayload,
}
