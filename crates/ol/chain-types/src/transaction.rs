use std::fmt;

use int_enum::IntEnum;
use strata_acct_types::{AccountId, VarVec};
use strata_codec::{Codec, CodecError, Decoder, Encoder};
use strata_codec_derive::Codec;
use strata_snark_acct_types::SnarkAccountUpdateContainer;

use crate::Slot;

/// Represents a single transaction within a block.
#[derive(Clone, Debug, Codec)]
pub struct OLTransaction {
    /// Any extra data associated with the transaction.
    extra: TransactionAttachment,

    /// The actual payload for the transaction.
    payload: TransactionPayload,
}

impl OLTransaction {
    // TODO use a builder
    pub fn new(extra: TransactionAttachment, payload: TransactionPayload) -> Self {
        Self { payload, extra }
    }

    pub fn attachments(&self) -> &TransactionAttachment {
        &self.extra
    }

    pub fn payload(&self) -> &TransactionPayload {
        &self.payload
    }

    pub fn type_id(&self) -> TxTypeId {
        self.payload().type_id()
    }
}

/// The actual payload of the transaction.
// TODO probably convert these from being struct-like variants
#[derive(Clone, Debug)]
pub enum TransactionPayload {
    GenericAccountMessage(GamTxPayload),
    SnarkAccountUpdate(SnarkAccountUpdateTxPayload),
}

impl TransactionPayload {
    pub fn type_id(&self) -> TxTypeId {
        match self {
            TransactionPayload::GenericAccountMessage { .. } => TxTypeId::GenericAccountMessage,
            TransactionPayload::SnarkAccountUpdate { .. } => TxTypeId::SnarkAccountUpdate,
        }
    }
}

impl Codec for TransactionPayload {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        match self {
            TransactionPayload::GenericAccountMessage(payload) => {
                1u8.encode(enc)?;
                payload.encode(enc)?;
            }
            TransactionPayload::SnarkAccountUpdate(payload) => {
                2u8.encode(enc)?;
                payload.encode(enc)?;
            }
        }
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let variant = u8::decode(dec)?;
        match variant {
            1 => {
                let payload = GamTxPayload::decode(dec)?;
                Ok(TransactionPayload::GenericAccountMessage(payload))
            }
            2 => {
                let payload = SnarkAccountUpdateTxPayload::decode(dec)?;
                Ok(TransactionPayload::SnarkAccountUpdate(payload))
            }
            _ => Err(CodecError::InvalidVariant("TransactionPayload")),
        }
    }
}

/// Additional constraints that we can place on a transaction.
///
/// This isn't *that* useful for now, but will be in the future.
#[derive(Clone, Debug, Default)]
pub struct TransactionAttachment {
    min_slot: Option<Slot>,
    max_slot: Option<Slot>,
}

impl TransactionAttachment {
    pub fn new_empty() -> Self {
        Self {
            min_slot: None,
            max_slot: None,
        }
    }

    pub fn min_slot(&self) -> Option<Slot> {
        self.min_slot
    }

    pub fn set_min_slot(&mut self, min_slot: Option<Slot>) {
        self.min_slot = min_slot;
    }

    pub fn max_slot(&self) -> Option<Slot> {
        self.max_slot
    }

    pub fn set_max_slot(&mut self, max_slot: Option<Slot>) {
        self.max_slot = max_slot;
    }
}

impl Codec for TransactionAttachment {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        // Encode Option fields as bool (is_some) followed by value if present
        match self.min_slot {
            Some(slot) => {
                true.encode(enc)?;
                slot.encode(enc)?;
            }
            None => {
                false.encode(enc)?;
            }
        }
        match self.max_slot {
            Some(slot) => {
                true.encode(enc)?;
                slot.encode(enc)?;
            }
            None => {
                false.encode(enc)?;
            }
        }
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let min_slot = if bool::decode(dec)? {
            Some(Slot::decode(dec)?)
        } else {
            None
        };
        let max_slot = if bool::decode(dec)? {
            Some(Slot::decode(dec)?)
        } else {
            None
        };
        Ok(Self { min_slot, max_slot })
    }
}

/// Type ID to indicate transaction types.
#[repr(u16)]
#[derive(Copy, Clone, Debug, Hash, Eq, PartialEq, Ord, PartialOrd, IntEnum)]
pub enum TxTypeId {
    /// Transactions that are messages being sent to other accounts.
    GenericAccountMessage = 1,

    /// Transactions that are snark account updates.
    SnarkAccountUpdate = 2,
}

impl fmt::Display for TxTypeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            TxTypeId::SnarkAccountUpdate => "snark-account-update",
            TxTypeId::GenericAccountMessage => "generic-account-message",
        };
        f.write_str(s)
    }
}

/// "Generic Account Message" tx payload.
#[derive(Clone, Debug, Codec)]
pub struct GamTxPayload {
    target: AccountId,
    payload: VarVec<u8>,
}

impl GamTxPayload {
    pub fn new(target: AccountId, payload: VarVec<u8>) -> Self {
        Self { target, payload }
    }

    pub fn target(&self) -> &AccountId {
        &self.target
    }

    pub fn payload(&self) -> &[u8] {
        self.payload.as_ref()
    }
}

/// Snark account update payload.
#[derive(Clone, Debug, Codec)]
pub struct SnarkAccountUpdateTxPayload {
    target: AccountId,
    update_container: SnarkAccountUpdateContainer,
}

impl SnarkAccountUpdateTxPayload {
    pub fn new(target: AccountId, update_container: SnarkAccountUpdateContainer) -> Self {
        Self {
            target,
            update_container,
        }
    }

    pub fn target(&self) -> &AccountId {
        &self.target
    }

    pub fn update_container(&self) -> &SnarkAccountUpdateContainer {
        &self.update_container
    }
}
