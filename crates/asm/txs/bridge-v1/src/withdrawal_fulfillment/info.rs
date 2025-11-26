use arbitrary::{Arbitrary, Unstructured};
use bitcoin::ScriptBuf;
use strata_primitives::{bitcoin_bosd::Descriptor, l1::BitcoinAmount};

use crate::withdrawal_fulfillment::aux::WithdrawalFulfillmentTxHeaderAux;

/// Information extracted from a Bitcoin withdrawal fulfillment transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WithdrawalFulfillmentInfo {
    /// Parsed SPS-50 auxiliary data.
    pub header_aux: WithdrawalFulfillmentTxHeaderAux,

    /// The Bitcoin script address where the withdrawn funds are being sent.
    pub withdrawal_destination: ScriptBuf,

    /// The amount of Bitcoin being withdrawn.
    pub withdrawal_amount: BitcoinAmount,
}

impl WithdrawalFulfillmentInfo {
    pub fn new(
        header_aux: WithdrawalFulfillmentTxHeaderAux,
        withdrawal_destination: ScriptBuf,
        withdrawal_amount: BitcoinAmount,
    ) -> Self {
        Self {
            header_aux,
            withdrawal_destination,
            withdrawal_amount,
        }
    }

    pub fn header_aux(&self) -> &WithdrawalFulfillmentTxHeaderAux {
        &self.header_aux
    }

    pub fn header_aux_mut(&mut self) -> &mut WithdrawalFulfillmentTxHeaderAux {
        &mut self.header_aux
    }

    pub fn withdrawal_destination(&self) -> &ScriptBuf {
        &self.withdrawal_destination
    }

    pub fn set_withdrawal_destination(&mut self, withdrawal_destination: ScriptBuf) {
        self.withdrawal_destination = withdrawal_destination;
    }

    pub fn withdrawal_amount(&self) -> BitcoinAmount {
        self.withdrawal_amount
    }

    pub fn set_withdrawal_amount(&mut self, withdrawal_amount: BitcoinAmount) {
        self.withdrawal_amount = withdrawal_amount;
    }
}

impl<'a> Arbitrary<'a> for WithdrawalFulfillmentInfo {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let withdrawal_destination = Descriptor::arbitrary(u)?.to_script();
        Ok(WithdrawalFulfillmentInfo {
            header_aux: WithdrawalFulfillmentTxHeaderAux::arbitrary(u)?,
            withdrawal_destination,
            withdrawal_amount: BitcoinAmount::from_sat(u64::arbitrary(u)?),
        })
    }
}
