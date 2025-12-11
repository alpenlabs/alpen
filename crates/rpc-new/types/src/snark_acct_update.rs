use serde::{Deserialize, Serialize};
use strata_primitives::{HexBytes, HexBytes32};

/// Snark account update payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcSnarkAccountUpdate {
    /// The target account.
    target: HexBytes32,
    /// The encode update operation [`strata_snark_acct_types::UpdateOperationData`].
    update_operation_encoded: HexBytes,
    /// The update proof.
    update_proof: HexBytes,
}

impl RpcSnarkAccountUpdate {
    /// Creates a new [`RpcSnarkAccountUpdate`].
    pub fn new(
        target: HexBytes32,
        update_operation_encoded: HexBytes,
        update_proof: HexBytes,
    ) -> Self {
        Self {
            target,
            update_operation_encoded,
            update_proof,
        }
    }

    /// Returns the target account.
    pub fn target(&self) -> &HexBytes32 {
        &self.target
    }

    /// Returns encoded [`UpdateOperationData`]
    pub fn update_operation_encoded(&self) -> &HexBytes {
        &self.update_operation_encoded
    }

    /// Returns the update proof.
    pub fn update_proof(&self) -> &HexBytes {
        &self.update_proof
    }
}
