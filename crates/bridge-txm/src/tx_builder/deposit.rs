//! Builders related to building deposit-related transactions.
//!
//! Contains types, traits and implementations related to creating various transactions used in the
//! bridge-in dataflow.

use bitcoin::{Amount, OutPoint, XOnlyPublicKey};
use serde::{Deserialize, Serialize};

/// The deposit information  required to create the Deposit Transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepositInfo {
    /// The deposit request transaction outpoint from the user.
    deposit_request_outpoint: OutPoint,

    /// The execution layer address to mint the equivalent tokens to.
    /// As of now, this is just the 20-byte EVM address.
    el_address: Vec<u8>,

    /// The amount in bitcoins that the user wishes to deposit.
    amount: Amount,

    /// The metadata associated with the deposit request.
    metadata: DepositMetadata,
}

impl DepositInfo {
    /// Construct the deposit transaction based on some information that depends on the bridge
    /// implementation, the deposit request transaction created by the user, some metadata
    /// related to the rollup and the actual pubkeys of the operators in the federation.
    pub fn construct_deposit_tx(&self, _pubkeys: Vec<XOnlyPublicKey>) -> Vec<u8> {
        unimplemented!();
    }
}

/// The metadata associated with a deposit. This will be used to communicated additional
/// information to the rollup. For now, this only carries limited information but we may extend
/// it later.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepositMetadata {
    /// The protocol version that the deposit is associated with.
    version: Version,

    /// Special identifier that helps the `alpen-exrpress-btcio::reader` worker identify relevant
    /// deposits.
    // TODO: Convert this to an enum that handles various identifiers if necessary in the future.
    // For now, this identifier will be a constant.
    identifier: String,
}

/// The version of the bridge protocol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Version {
    /// Devnet bridge with cooperative withdrawal path only (N:N trust assumption).
    V0,
    /// Full trust-minimized bridge with both cooperative and uncooperatie withdrawals paths.
    V1,
}
