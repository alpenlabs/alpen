//! Message related types.

use std::any::Any;

use borsh::{BorshDeserialize, BorshSerialize};

use crate::{BRIDGE_SUBPROTOCOL_ID, SubprotocolId};

/// Generic wrapper around a inter-proto msg.
pub trait InterprotoMsg: Any + 'static {
    /// Returns the ID of the subprotocol that produced this messages.
    fn id(&self) -> SubprotocolId;

    /// Converts the message ref into a `&dyn Any` for upcasting.
    ///
    /// The impl of this function should always be `self`.  For technical type
    /// system reasons, this cannot be provided as a default impl.
    ///
    /// This can be removed by using trait upcasting in Rust 1.86.
    fn as_dyn_any(&self) -> &dyn Any;
}

/// Empty impl that can't be constructed.
#[derive(Copy, Clone, Debug)]
pub struct NullMsg<const ID: SubprotocolId>;

impl<const ID: SubprotocolId> InterprotoMsg for NullMsg<ID> {
    fn id(&self) -> SubprotocolId {
        ID
    }

    fn as_dyn_any(&self) -> &dyn Any {
        self
    }
}

/// L2 to L1 message type representing withdrawal requests
/// TODO: Double check the fields and their types.
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize, PartialEq, Eq)]
pub struct L2ToL1Msg {
    /// Destination address on L1 (Bitcoin)
    pub dest_address: Vec<u8>,
    /// Amount to withdraw in satoshis
    pub amount: u64,
    /// Additional data payload for the withdrawal
    pub data: Vec<u8>,
    /// Nonce to prevent replay attacks?
    pub nonce: u64,
}

/// Withdrawal messages to be sent to Bridge subprotocol
#[derive(Clone, Debug)]
pub struct WithdrawalMsg {
    pub withdrawals: Vec<L2ToL1Msg>,
}

impl InterprotoMsg for WithdrawalMsg {
    fn id(&self) -> SubprotocolId {
        BRIDGE_SUBPROTOCOL_ID
    }

    fn as_dyn_any(&self) -> &dyn Any {
        self
    }
}

/// Stub type for SPS-msg-fmt log.
///
/// This should be converted to be a wrapper from the strata-common repo.
#[derive(Clone, Debug)]
pub struct Log {
    /// Type identifier
    ty: u16,
    /// Body of the message
    body: Vec<u8>,
}

impl Log {
    /// Constructor
    pub fn new(ty: u16, body: Vec<u8>) -> Self {
        Self { ty, body }
    }

    /// Returns type identifier
    pub fn ty(&self) -> u16 {
        self.ty
    }

    /// Returns slice of body
    pub fn body(&self) -> &[u8] {
        &self.body
    }
}

#[cfg(test)]
mod tests {
    use std::any::Any;

    use super::InterprotoMsg;
    use crate::SubprotocolId;

    #[derive(Clone)]
    struct Foo {
        x: u32,
    }

    impl Foo {
        fn x(&self) -> u32 {
            self.x
        }
    }

    impl InterprotoMsg for Foo {
        fn id(&self) -> SubprotocolId {
            42
        }

        fn as_dyn_any(&self) -> &dyn Any {
            self
        }
    }

    #[test]
    fn test() {
        // TODO
        let inst = Foo { x: 5 };
        inst.x();
        let _inst_box = Box::new(inst) as Box<dyn InterprotoMsg>;
    }
}
