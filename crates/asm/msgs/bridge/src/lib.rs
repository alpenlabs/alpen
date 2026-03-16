//! Inter-protocol message types for the bridge subprotocol.
//!
//! This crate exposes the incoming bridge messages and shared withdrawal output
//! payload so other subprotocols can dispatch withdrawals without pulling in the
//! bridge implementation crate.

use std::any::Any;

use arbitrary::{Arbitrary, Unstructured};
use strata_asm_common::{InterprotoMsg, SubprotocolId};
use strata_asm_txs_bridge_v1::BRIDGE_V1_SUBPROTOCOL_ID;
use strata_bridge_types::OperatorSelection;
use strata_btc_types::BitcoinAmount;
use strata_primitives::bitcoin_bosd::Descriptor;

#[allow(
    clippy::all,
    unreachable_pub,
    clippy::allow_attributes,
    clippy::absolute_paths,
    reason = "generated code"
)]
mod ssz_generated {
    include!(concat!(env!("OUT_DIR"), "/generated.rs"));
}

pub use ssz_generated::ssz::messages::{
    BridgeIncomingMsg, BridgeIncomingMsgRef, DescriptorBytes, DispatchWithdrawal,
    DispatchWithdrawalRef, WithdrawOutput, WithdrawOutputRef,
};

fn encode_descriptor(destination: Descriptor) -> DescriptorBytes {
    DescriptorBytes::new(destination.to_bytes())
        .expect("bridge descriptor must stay within SSZ bounds")
}

fn decode_descriptor(bytes: &[u8]) -> Descriptor {
    Descriptor::from_vec(bytes.to_vec()).expect("bridge descriptor bytes must remain valid")
}

impl WithdrawOutput {
    /// Creates a new withdrawal output with the specified destination and amount.
    pub fn new(destination: Descriptor, amt: BitcoinAmount) -> Self {
        Self {
            destination: encode_descriptor(destination),
            amt,
        }
    }

    /// Returns the destination descriptor.
    pub fn destination(&self) -> Descriptor {
        decode_descriptor(&self.destination)
    }

    /// Returns the withdrawal amount.
    pub fn amt(&self) -> BitcoinAmount {
        self.amt
    }
}

impl<'a> Arbitrary<'a> for WithdrawOutput {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let amt = BitcoinAmount::arbitrary(u)?;
        let payload: [u8; 20] = u.arbitrary()?;

        let destination = match u.int_in_range(0..=2)? {
            0 => Descriptor::new_p2pkh(&payload),
            1 => Descriptor::new_p2sh(&payload),
            _ => Descriptor::new_p2wpkh(&payload),
        };

        Ok(Self::new(destination, amt))
    }
}

impl DispatchWithdrawal {
    /// Creates a dispatch-withdrawal payload from the user intent.
    pub fn new(output: WithdrawOutput, selected_operator: OperatorSelection) -> Self {
        Self {
            output,
            selected_operator: selected_operator.raw(),
        }
    }

    /// Returns the withdrawal output carried by the message.
    pub fn output(&self) -> &WithdrawOutput {
        &self.output
    }

    /// Returns the user's preferred operator selection.
    pub fn selected_operator(&self) -> OperatorSelection {
        OperatorSelection::from_raw(self.selected_operator)
    }
}

impl BridgeIncomingMsg {
    /// Creates a bridge incoming message for withdrawal dispatch.
    pub fn dispatch_withdrawal(
        output: WithdrawOutput,
        selected_operator: OperatorSelection,
    ) -> Self {
        Self::DispatchWithdrawal(DispatchWithdrawal::new(output, selected_operator))
    }
}

impl InterprotoMsg for BridgeIncomingMsg {
    fn id(&self) -> SubprotocolId {
        BRIDGE_V1_SUBPROTOCOL_ID
    }

    fn as_dyn_any(&self) -> &dyn Any {
        self
    }
}
