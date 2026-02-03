//! Inter-protocol message types for the bridge subprotocol.
//!
//! This crate exposes the incoming bridge messages and shared withdrawal output
//! payload so other subprotocols can dispatch withdrawals without pulling in the
//! bridge implementation crate.

use std::{any::Any, str::FromStr};

use arbitrary::Arbitrary;
use rkyv::{
    Archived, Place, Resolver,
    rancor::Fallible,
    with::{ArchiveWith, DeserializeWith, SerializeWith},
};
use serde::{Deserialize, Serialize};
use strata_asm_common::{InterprotoMsg, SubprotocolId};
use strata_asm_txs_bridge_v1::BRIDGE_V1_SUBPROTOCOL_ID;
use strata_primitives::{bitcoin_bosd::Descriptor, l1::BitcoinAmount};

/// Serializer for [`Descriptor`] as string for rkyv.
struct DescriptorAsString;

impl ArchiveWith<Descriptor> for DescriptorAsString {
    type Archived = Archived<String>;
    type Resolver = Resolver<String>;

    fn resolve_with(field: &Descriptor, resolver: Self::Resolver, out: Place<Self::Archived>) {
        rkyv::Archive::resolve(&field.to_string(), resolver, out);
    }
}

impl<S> SerializeWith<Descriptor, S> for DescriptorAsString
where
    S: Fallible + ?Sized,
    String: rkyv::Serialize<S>,
{
    fn serialize_with(field: &Descriptor, serializer: &mut S) -> Result<Self::Resolver, S::Error> {
        rkyv::Serialize::serialize(&field.to_string(), serializer)
    }
}

impl<D> DeserializeWith<Archived<String>, Descriptor, D> for DescriptorAsString
where
    D: Fallible + ?Sized,
    Archived<String>: rkyv::Deserialize<String, D>,
{
    fn deserialize_with(
        field: &Archived<String>,
        deserializer: &mut D,
    ) -> Result<Descriptor, D::Error> {
        let desc = rkyv::Deserialize::deserialize(field, deserializer)?;
        Ok(Descriptor::from_str(&desc).expect("stored descriptor should be valid"))
    }
}

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
#[derive(
    Clone,
    Debug,
    Eq,
    PartialEq,
    Serialize,
    Deserialize,
    Arbitrary,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
pub struct WithdrawOutput {
    /// Bitcoin Output Script Descriptor specifying the destination address.
    #[rkyv(with = DescriptorAsString)]
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

/// Incoming message types received from other subprotocols.
///
/// This enum represents all possible message types that the bridge subprotocol can
/// receive from other subprotocols in the ASM.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BridgeIncomingMsg {
    /// Emitted after a checkpoint proof has been validated. Contains the withdrawal command
    /// specifying the destination descriptor and amount to be withdrawn.
    DispatchWithdrawal(WithdrawOutput),
}

impl InterprotoMsg for BridgeIncomingMsg {
    fn id(&self) -> SubprotocolId {
        BRIDGE_V1_SUBPROTOCOL_ID
    }

    fn as_dyn_any(&self) -> &dyn Any {
        self
    }
}
