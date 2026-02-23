use serde::{Deserialize, Serialize};
use ssz::{Decode, Encode};
use strata_checkpoint_types_ssz::CheckpointPayload;
use strata_ol_block_assembly::FullBlockTemplate;
use strata_ol_chain_types_new::{OLBlockBody, OLBlockHeader};
use strata_ol_sequencer::{BlockSigningDuty, CheckpointSigningDuty, Duty};
use thiserror::Error;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub enum RpcDuty {
    SignBlock(RpcBlockSigningDuty),
    SignCheckpoint(RpcCheckpointSigningDuty),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct RpcBlockSigningDuty {
    template: RpcBlockTemplate,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct RpcBlockTemplate {
    /// ssz serialized OL block header
    header: Vec<u8>,
    /// ssz serialized OL block body
    body: Vec<u8>,
}

impl From<Duty> for RpcDuty {
    fn from(d: Duty) -> Self {
        match d {
            Duty::SignBlock(blkduty) => {
                let tmp = blkduty.template;
                let template = RpcBlockTemplate {
                    header: tmp.header().as_ssz_bytes(),
                    body: tmp.body().as_ssz_bytes(),
                };
                RpcDuty::SignBlock(RpcBlockSigningDuty { template })
            }
            Duty::SignCheckpoint(c) => {
                let checkpoint = c.checkpoint().as_ssz_bytes();
                RpcDuty::SignCheckpoint(RpcCheckpointSigningDuty { checkpoint })
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct RpcCheckpointSigningDuty {
    /// ssz serialized checkpoint payload.
    checkpoint: Vec<u8>,
}

/// Error type for RpcDuty conversion
#[derive(Debug, Error)]
pub enum RpcDutyConversionError {
    #[error("Failed to decode OL block header: {0}")]
    HeaderDecodeError(String),

    #[error("Failed to decode OL block body: {0}")]
    BodyDecodeError(String),

    #[error("Failed to decode checkpoint payload: {0}")]
    CheckpointDecodeError(String),
}

impl TryFrom<RpcDuty> for Duty {
    type Error = RpcDutyConversionError;

    fn try_from(rpc_duty: RpcDuty) -> Result<Self, Self::Error> {
        match rpc_duty {
            RpcDuty::SignBlock(rpc_block_duty) => {
                let header = OLBlockHeader::from_ssz_bytes(&rpc_block_duty.template.header)
                    .map_err(|e| RpcDutyConversionError::HeaderDecodeError(e.to_string()))?;

                let body = OLBlockBody::from_ssz_bytes(&rpc_block_duty.template.body)
                    .map_err(|e| RpcDutyConversionError::BodyDecodeError(e.to_string()))?;

                let template = FullBlockTemplate::new(header, body);
                let duty = BlockSigningDuty { template };
                Ok(Duty::SignBlock(duty))
            }
            RpcDuty::SignCheckpoint(rpc_checkpoint_duty) => {
                let checkpoint = CheckpointPayload::from_ssz_bytes(&rpc_checkpoint_duty.checkpoint)
                    .map_err(|e| RpcDutyConversionError::CheckpointDecodeError(e.to_string()))?;

                let duty = CheckpointSigningDuty::new(checkpoint);
                Ok(Duty::SignCheckpoint(duty))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::convert::TryInto;

    use strata_checkpoint_types_ssz::{CheckpointSidecar, CheckpointTip};
    use strata_primitives::{Buf32, OLBlockCommitment};

    use super::*;

    fn create_checkpoint_payload(epoch: u32) -> CheckpointPayload {
        let tip = CheckpointTip {
            epoch,
            l1_height: 200,
            l2_commitment: OLBlockCommitment::new(1, Buf32::zero().into()),
        };
        let sidecar = CheckpointSidecar {
            ol_state_diff: vec![2; 100].into(),
            ol_logs: vec![].into(),
        };
        CheckpointPayload {
            new_tip: tip,
            sidecar,
            proof: vec![0].into(),
        }
    }

    #[test]
    fn test_rpc_duty_roundtrip_conversion() {
        // Create a simple checkpoint duty for testing
        let checkpoint_payload = create_checkpoint_payload(1);
        let checkpoint_duty = CheckpointSigningDuty::new(checkpoint_payload);
        let duty = Duty::SignCheckpoint(checkpoint_duty);

        // Convert to RpcDuty
        let rpc_duty: RpcDuty = duty.clone().into();

        // Convert back to Duty
        let converted_duty: Duty = rpc_duty.try_into().unwrap();

        // Verify the checkpoint data is preserved
        if let (Duty::SignCheckpoint(orig), Duty::SignCheckpoint(conv)) = (&duty, &converted_duty) {
            assert_eq!(
                orig.checkpoint().as_ssz_bytes(),
                conv.checkpoint().as_ssz_bytes()
            );
        } else {
            panic!("Duty type mismatch after conversion");
        }
    }
}
