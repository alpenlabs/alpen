//! Types for managing pending bridging operations in the CL state.

use std::str::FromStr;

use rkyv::{
    rancor::Fallible,
    with::{ArchiveWith, DeserializeWith, SerializeWith},
    Archived, Place, Resolver,
};
use serde::{Deserialize, Serialize};
use strata_identifiers::SubjectId;
use strata_primitives::{bitcoin_bosd::Descriptor, buf::Buf32, l1::BitcoinAmount};

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

/// Describes an intent to withdraw that hasn't been dispatched yet.
#[derive(
    Clone,
    Debug,
    Eq,
    PartialEq,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
pub struct WithdrawalIntent {
    /// Quantity of L1 asset, for Bitcoin this is sats.
    amt: BitcoinAmount,

    /// Destination [`Descriptor`] for the withdrawal
    #[rkyv(with = DescriptorAsString)]
    destination: Descriptor,

    /// withdrawal request transaction id
    withdrawal_txid: Buf32,
}

impl WithdrawalIntent {
    pub const fn new(amt: BitcoinAmount, destination: Descriptor, withdrawal_txid: Buf32) -> Self {
        Self {
            amt,
            destination,
            withdrawal_txid,
        }
    }

    pub fn as_parts(&self) -> (u64, &Descriptor) {
        (self.amt.to_sat(), &self.destination)
    }

    pub const fn amt(&self) -> &BitcoinAmount {
        &self.amt
    }

    pub const fn destination(&self) -> &Descriptor {
        &self.destination
    }

    pub const fn withdrawal_txid(&self) -> &Buf32 {
        &self.withdrawal_txid
    }
}

/// Set of withdrawals that are assigned to a deposit bridge utxo.
#[derive(
    Clone,
    Debug,
    Eq,
    PartialEq,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
pub struct WithdrawalBatch {
    /// A series of [WithdrawalIntent]'s who sum does not exceed withdrawal denomination.
    intents: Vec<WithdrawalIntent>,
}

impl WithdrawalBatch {
    /// Creates a new instance.
    pub const fn new(intents: Vec<WithdrawalIntent>) -> Self {
        Self { intents }
    }

    /// Gets the total value of the batch.  This must be less than the size of
    /// the utxo it's assigned to.
    pub fn get_total_value(&self) -> BitcoinAmount {
        self.intents
            .iter()
            .fold(BitcoinAmount::ZERO, |acc, wi| acc.saturating_add(wi.amt))
    }

    pub fn intents(&self) -> &[WithdrawalIntent] {
        &self.intents[..]
    }
}

/// Describes a deposit data to be processed by an EE.
#[derive(Clone, Debug, Eq, PartialEq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct DepositIntent {
    /// Quantity in the L1 asset, for Bitcoin this is sats.
    amt: BitcoinAmount,

    /// Destination subject identifier within the execution environment.
    dest_ident: SubjectId,
}

impl DepositIntent {
    pub const fn new(amt: BitcoinAmount, dest_ident: SubjectId) -> Self {
        Self { amt, dest_ident }
    }

    pub fn amt(&self) -> u64 {
        self.amt.to_sat()
    }

    pub const fn dest_ident(&self) -> &SubjectId {
        &self.dest_ident
    }
}
