//! RPC types for the Orchestration Layer.

use serde::{Deserialize, Serialize};
use ssz::Decode;
use strata_acct_types::AccountId;
use strata_ol_chain_types_new::TransactionAttachment;
use strata_ol_mempool::OLMempoolTransaction;
use strata_primitives::{HexBytes, HexBytes32};
use strata_snark_acct_types::{SnarkAccountUpdate, UpdateOperationData};

use crate::RpcSnarkAccountUpdate;

/// OL transaction for submission (excludes accumulator proofs).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcOLTransaction {
    /// The payload.
    payload: RpcTransactionPayload,

    /// The attachments.
    attachments: RpcTransactionAttachment,
}

impl RpcOLTransaction {
    /// Creates a new [`RpcOLTransaction`].
    pub fn new(payload: RpcTransactionPayload, attachments: RpcTransactionAttachment) -> Self {
        Self {
            payload,
            attachments,
        }
    }

    /// Returns the payload.
    pub fn payload(&self) -> &RpcTransactionPayload {
        &self.payload
    }

    /// Returns the attachments.
    pub fn attachments(&self) -> &RpcTransactionAttachment {
        &self.attachments
    }
}

/// Transaction payload: generic message or snark account update.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RpcTransactionPayload {
    /// Generic account message.
    GenericAccountMessage(RpcGenericAccountMessage),

    /// Snark account update.
    SnarkAccountUpdate(RpcSnarkAccountUpdate),
}

/// Generic account message payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcGenericAccountMessage {
    /// The target account.
    target: HexBytes32,

    /// The payload.
    payload: HexBytes,
}

impl RpcGenericAccountMessage {
    /// Creates a new [`RpcGenericAccountMessage`].
    pub fn new(target: HexBytes32, payload: HexBytes) -> Self {
        Self { target, payload }
    }

    /// Returns the target account.
    pub fn target(&self) -> &HexBytes32 {
        &self.target
    }

    /// Returns the payload.
    pub fn payload(&self) -> &HexBytes {
        &self.payload
    }
}

/// Transaction extra: slot constraints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcTransactionAttachment {
    /// Minimum slot.
    min_slot: Option<u64>,

    /// Maximum slot.
    max_slot: Option<u64>,
}

impl RpcTransactionAttachment {
    /// Creates a new [`RpcTransactionAttachment`].
    pub fn new(min_slot: Option<u64>, max_slot: Option<u64>) -> Self {
        Self { min_slot, max_slot }
    }

    /// Returns the minimum slot.
    pub fn min_slot(&self) -> Option<u64> {
        self.min_slot
    }

    /// Returns the maximum slot.
    pub fn max_slot(&self) -> Option<u64> {
        self.max_slot
    }
}

impl From<TransactionAttachment> for RpcTransactionAttachment {
    fn from(extra: TransactionAttachment) -> Self {
        Self {
            min_slot: extra.min_slot(),
            max_slot: extra.max_slot(),
        }
    }
}

impl From<RpcTransactionAttachment> for TransactionAttachment {
    fn from(rpc: RpcTransactionAttachment) -> Self {
        TransactionAttachment::new(rpc.min_slot, rpc.max_slot)
    }
}

/// Error type for transaction conversion.
#[derive(Debug, thiserror::Error)]
pub enum RpcTxConversionError {
    /// Failed to decode update operation data.
    #[error("failed to decode update operation data: {0}")]
    DecodeOperationData(String),

    /// Failed to create generic account message (payload too large).
    #[error("failed to create generic account message: {0}")]
    InvalidGenericMessage(&'static str),
}

impl TryFrom<RpcOLTransaction> for OLMempoolTransaction {
    type Error = RpcTxConversionError;

    fn try_from(rpc_tx: RpcOLTransaction) -> Result<Self, Self::Error> {
        let attachment: TransactionAttachment = rpc_tx.attachments.into();

        match rpc_tx.payload {
            RpcTransactionPayload::GenericAccountMessage(gam) => {
                let target = AccountId::new(gam.target.0);
                OLMempoolTransaction::new_generic_account_message(target, gam.payload.0, attachment)
                    .map_err(RpcTxConversionError::InvalidGenericMessage)
            }
            RpcTransactionPayload::SnarkAccountUpdate(sau) => {
                let target = AccountId::new(sau.target().0);

                // Decode UpdateOperationData from SSZ-encoded bytes
                let operation =
                    UpdateOperationData::from_ssz_bytes(&sau.update_operation_encoded().0)
                        .map_err(|e| RpcTxConversionError::DecodeOperationData(e.to_string()))?;

                let base_update = SnarkAccountUpdate::new(operation, sau.update_proof().0.clone());

                Ok(OLMempoolTransaction::new_snark_account_update(
                    target,
                    base_update,
                    attachment,
                ))
            }
        }
    }
}
