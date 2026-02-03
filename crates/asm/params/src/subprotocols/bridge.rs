#[cfg(feature = "arbitrary")]
use arbitrary::{Arbitrary, Unstructured};
use serde::{Deserialize, Serialize};
use strata_btc_types::BitcoinAmount;
use strata_crypto::EvenPublicKey;

/// Configuration for the BridgeV1 subprotocol.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
pub struct BridgeV1Config {
    /// Initial operator MuSig2 public keys for the bridge
    pub operators: Vec<EvenPublicKey>,
    /// The amount of bitcoin expected to be locked in the N/N multisig.
    pub denomination: BitcoinAmount,
    /// Duration in blocks for assignment execution deadlines
    pub assignment_duration: u64,
    /// Amount the operator can take as fees for processing withdrawal.
    pub operator_fee: BitcoinAmount,
    /// Number of blocks after Deposit Request Transaction that the depositor can reclaim funds if
    /// operators fail to process the deposit.
    pub recovery_delay: u32,
}
