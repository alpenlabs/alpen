//! Bridge state types.
//!
//! This just implements a very simple n-of-n multisig bridge.  It will be
//! extended to a more sophisticated design when we have that specced out.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_crypto::EvenPublicKey;
use strata_primitives::l1::{BitcoinAmount, L1Height};

/// Configuration for the BridgeV1 subprotocol.
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct BridgeV1Config {
    /// Initial operator MuSig2 public keys for the bridge
    pub operators: Vec<EvenPublicKey>,
    /// The amount of bitcoin expected to be locked in the N/N multisig.
    pub denomination: BitcoinAmount,
    /// Duration in blocks for assignment execution deadlines
    pub assignment_duration: L1Height,
    /// Amount the operator can take as fees for processing withdrawal.
    pub operator_fee: BitcoinAmount,
    /// Number of blocks after Deposit Request Transaction that the depositor can reclaim funds if
    /// operators fail to process the deposit.
    pub recovery_delay: u32,
}
