//! RPC types for the Orchestration Layer.

use serde::{Deserialize, Serialize};
use strata_ol_chain_types_new::TransactionAttachment;
use strata_primitives::{HexBytes, HexBytes32};

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
