use serde::{Deserialize, Serialize};
use strata_btc_types::BitcoinAmount;
use strata_crypto::EvenPublicKey;

/// Configuration for the BridgeV1 subprotocol.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeV1Config {
    /// Initial operator MuSig2 public keys for the bridge
    pub operators: Vec<EvenPublicKey>,
    /// The amount of bitcoin expected to be locked in the N/N multisig.
    pub denomination: BitcoinAmount,
    /// Number of Bitcoin blocks an operator has to fulfill a withdrawal before it is reassigned to
    /// a different operator.
    pub assignment_duration: u16,
    /// Amount the operator can take as fees for processing withdrawal.
    pub operator_fee: BitcoinAmount,
    /// Number of Bitcoin blocks after Deposit Request Transaction that the depositor can reclaim
    /// funds if operators fail to process the deposit.
    pub recovery_delay: u16,
}

#[cfg(feature = "arbitrary")]
impl<'a> arbitrary::Arbitrary<'a> for BridgeV1Config {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        // Generate at least one operator.
        let len = u.int_in_range(1..=5)?;
        let operators = (0..len)
            .map(|_| u.arbitrary())
            .collect::<arbitrary::Result<Vec<EvenPublicKey>>>()?;

        Ok(Self {
            operators,
            denomination: u.arbitrary()?,
            assignment_duration: u.arbitrary()?,
            operator_fee: u.arbitrary()?,
            recovery_delay: u.arbitrary()?,
        })
    }
}
