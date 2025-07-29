//! Bridge state types.
//!
//! This just implements a very simple n-of-n multisig bridge.  It will be
//! extended to a more sophisticated design when we have that specced out.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_primitives::{l1::BitcoinAmount, operator::OperatorPubkeys};

/// Configuration for the BridgeV1 subprotocol.
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct BridgeV1Config {
    /// Initial operator public keys for the bridge
    pub operators: Vec<OperatorPubkeys>,
    /// Expected deposit denomination for validation
    pub denomination: BitcoinAmount,
    /// Duration in blocks for assignment execution deadlines
    pub deadline_duration: u64,
}
