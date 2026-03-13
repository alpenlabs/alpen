//! Inter-protocol message types for the bridge subprotocol.
//!
//! This crate exposes the incoming bridge messages and shared withdrawal output
//! payload so other subprotocols can dispatch withdrawals without pulling in the
//! bridge implementation crate.

use std::any::Any;

use arbitrary::Arbitrary;
use serde::{Deserialize, Serialize};
use ssz::{Decode, DecodeError, Encode};
use ssz_derive::{Decode as DeriveDecode, Encode as DeriveEncode};
use strata_asm_common::{InterprotoMsg, SubprotocolId};
use strata_asm_txs_bridge_v1::BRIDGE_V1_SUBPROTOCOL_ID;
use strata_bridge_types::OperatorSelection;
use strata_primitives::{bitcoin_bosd::Descriptor, l1::BitcoinAmount};

/// Bitcoin output specification for a withdrawal operation.
///
/// Each withdrawal output specifies a destination address (as a Bitcoin descriptor)
/// and the amount to be sent. This structure provides all information needed by
/// operators to construct the appropriate Bitcoin transaction output.
///
/// # Bitcoin Descriptors
///
/// The destination uses Bitcoin Output Script Descriptors (BOSD), which provide
/// a standardized way to specify Bitcoin addresses and locking conditions.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Arbitrary)]
pub struct WithdrawOutput {
    /// Bitcoin Output Script Descriptor specifying the destination address.
    pub destination: Descriptor,

    /// Amount to withdraw (in satoshis).
    pub amt: BitcoinAmount,
}

impl WithdrawOutput {
    /// Creates a new withdrawal output with the specified destination and amount.
    pub fn new(destination: Descriptor, amt: BitcoinAmount) -> Self {
        Self { destination, amt }
    }

    /// Returns a reference to the destination descriptor.
    pub fn destination(&self) -> &Descriptor {
        &self.destination
    }

    /// Returns the withdrawal amount.
    pub fn amt(&self) -> BitcoinAmount {
        self.amt
    }
}

/// SSZ-friendly representation of [`WithdrawOutput`].
#[derive(DeriveEncode, DeriveDecode)]
struct WithdrawOutputSsz {
    /// The Bitcoin Output Script Descriptor specifying the destination address.
    destination: Vec<u8>,

    /// The amount to withdraw (in satoshis).
    amt: BitcoinAmount,
}

impl Encode for WithdrawOutput {
    fn is_ssz_fixed_len() -> bool {
        <WithdrawOutputSsz as Encode>::is_ssz_fixed_len()
    }

    fn ssz_fixed_len() -> usize {
        <WithdrawOutputSsz as Encode>::ssz_fixed_len()
    }

    fn ssz_append(&self, buf: &mut Vec<u8>) {
        WithdrawOutputSsz {
            destination: self.destination.to_bytes(),
            amt: self.amt,
        }
        .ssz_append(buf);
    }

    fn ssz_bytes_len(&self) -> usize {
        WithdrawOutputSsz {
            destination: self.destination.to_bytes(),
            amt: self.amt,
        }
        .ssz_bytes_len()
    }
}

impl Decode for WithdrawOutput {
    fn is_ssz_fixed_len() -> bool {
        <WithdrawOutputSsz as Decode>::is_ssz_fixed_len()
    }

    fn ssz_fixed_len() -> usize {
        <WithdrawOutputSsz as Decode>::ssz_fixed_len()
    }

    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        let value = WithdrawOutputSsz::from_ssz_bytes(bytes)?;
        let destination = Descriptor::from_bytes(&value.destination)
            .map_err(|err| DecodeError::BytesInvalid(err.to_string()))?;

        Ok(Self {
            destination,
            amt: value.amt,
        })
    }
}

/// Incoming message types received from other subprotocols.
///
/// This enum represents all possible message types that the bridge subprotocol can
/// receive from other subprotocols in the ASM.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BridgeIncomingMsg {
    /// Emitted after a checkpoint proof has been validated. Contains the withdrawal command
    /// specifying the destination descriptor and amount to be withdrawn.
    DispatchWithdrawal {
        /// The withdrawal output (destination + amount).
        output: WithdrawOutput,
        /// User's operator selection for withdrawal assignment.
        selected_operator: OperatorSelection,
    },
}

impl InterprotoMsg for BridgeIncomingMsg {
    fn id(&self) -> SubprotocolId {
        BRIDGE_V1_SUBPROTOCOL_ID
    }

    fn as_dyn_any(&self) -> &dyn Any {
        self
    }
}
