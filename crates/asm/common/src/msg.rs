//! Message related types using strata-msg-fmt.

use std::any::Any;

use borsh::{BorshDeserialize, BorshSerialize};
use strata_l1_txfmt::SubprotocolId;
// Re-export standard types for convenience
pub use strata_msg_fmt::{Error as MessageError, Msg, OwnedMsg, TypeId};

use crate::AsmError;

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

/// A wrapper around [`OwnedMsg`] that provides typed access to ASM messages.
///
/// `Message` encapsulates a message with a type identifier and serialized data body,
/// providing a consistent interface for storing and retrieving different types of ASM messages.
/// The underlying [`OwnedMsg`] handles the storage and encoding/decoding according to the
/// SPS-msg-fmt specification.
/// [PLACE_HOLDER] Update the names to align with the team's new naming convention.
///
/// [PLACE_HOLDER] TODO: Inter-protocol messaging design
/// Key points for future implementation:
/// - Don't pass raw OL logs as inter-proto messages
/// - Each subprotocol should export opaque enum types for messages it expects
/// - Use typed messages instead of raw OwnedMsg objects
/// - Example: BridgeMessage::Withdrawal { recipient, amount }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message(pub OwnedMsg);

impl Message {
    /// Creates a new message from type and body
    pub fn new(ty: TypeId, body: Vec<u8>) -> Result<Self, AsmError> {
        let owned_msg = OwnedMsg::new(ty, body).map_err(AsmError::from)?;
        Ok(Message(owned_msg))
    }

    /// Creates a message from raw encoded bytes
    pub fn from_encoded(encoded_bytes: Vec<u8>) -> Result<Self, AsmError> {
        let owned_msg = OwnedMsg::try_from(encoded_bytes.as_slice()).map_err(AsmError::from)?;
        Ok(Message(owned_msg))
    }

    /// Returns the message type
    pub fn ty(&self) -> TypeId {
        self.0.ty()
    }

    /// Returns the message body
    pub fn body(&self) -> &[u8] {
        self.0.body()
    }

    /// Encodes the message to SPS-msg-fmt bytes
    pub fn encode(&self) -> Result<Vec<u8>, AsmError> {
        let mut result = Vec::new();
        strata_msg_fmt::try_encode_into_buf(
            self.0.ty(),
            self.0.body().iter().copied(),
            &mut result,
        )
        .map_err(AsmError::from)?;
        Ok(result)
    }

    /// Returns the raw encoded message bytes (alias for encode)
    pub fn encoded(&self) -> Result<Vec<u8>, AsmError> {
        self.encode()
    }

    /// Converts to OwnedMsg
    pub fn to_msg(&self) -> OwnedMsg {
        self.0.clone()
    }
}

impl BorshSerialize for Message {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        // Serialize as (ty, body) tuple for Borsh compatibility
        (self.0.ty(), self.0.body().to_vec()).serialize(writer)
    }
}

impl BorshDeserialize for Message {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        // Deserialize as (ty, body) tuple for Borsh compatibility
        let (ty, body): (TypeId, Vec<u8>) = BorshDeserialize::deserialize_reader(reader)?;
        let owned_msg = OwnedMsg::new(ty, body)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        Ok(Message(owned_msg))
    }
}

/// Temporary alias for backwards compatibility
/// [PLACE_HOLDER] Update the names to align with the team’s new naming convention.
/// Temporary alias for backwards compatibility
pub type L2ToL1Msg = Message;

/// Generic container for multiple messages following SPS-msg-fmt
#[derive(Clone, Debug)]
pub struct MessagesContainer {
    /// The target subprotocol ID
    pub target_subprotocol: SubprotocolId,
    /// Messages using strata-msg-fmt
    pub messages: Vec<OwnedMsg>,
}

impl MessagesContainer {
    /// Creates a new messages container for a specific subprotocol
    pub fn new(target_subprotocol: SubprotocolId) -> Self {
        Self {
            target_subprotocol,
            messages: Vec::new(),
        }
    }

    /// Adds a message to the container
    pub fn add_message(&mut self, message: OwnedMsg) {
        self.messages.push(message);
    }

    /// Creates a new container with the provided messages
    pub fn with_messages(target_subprotocol: SubprotocolId, messages: Vec<OwnedMsg>) -> Self {
        Self {
            target_subprotocol,
            messages,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::any::Any;

    use strata_l1_txfmt::SubprotocolId;
    use strata_msg_fmt::{Msg, OwnedMsg};

    use super::{InterprotoMsg, Message};

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
    fn test_interproto_msg_trait_object() {
        let inst = Foo { x: 5 };
        inst.x();
        let _inst_box = Box::new(inst) as Box<dyn InterprotoMsg>;
    }

    #[test]
    fn test_msg_fmt_encoding() {
        // type 0x00 body "hello" → 0068656c6c6f
        let mut encoded = Vec::new();
        strata_msg_fmt::try_encode_into_buf(0x00, b"hello".iter().copied(), &mut encoded).unwrap();
        assert_eq!(encoded, vec![0x00, 0x68, 0x65, 0x6c, 0x6c, 0x6f]);
        let owned_msg = OwnedMsg::try_from(encoded.as_slice()).unwrap();
        assert_eq!(owned_msg.ty(), 0x00);
        assert_eq!(owned_msg.body(), b"hello");

        // type 0x80 body "abc" → 8080616263
        let mut encoded = Vec::new();
        strata_msg_fmt::try_encode_into_buf(0x80, b"abc".iter().copied(), &mut encoded).unwrap();
        assert_eq!(encoded, vec![0x80, 0x80, 0x61, 0x62, 0x63]);
        let owned_msg = OwnedMsg::try_from(encoded.as_slice()).unwrap();
        assert_eq!(owned_msg.ty(), 0x80);
        assert_eq!(owned_msg.body(), b"abc");

        // type 0x1234 body "xyz" → 923478797a
        let mut encoded = Vec::new();
        strata_msg_fmt::try_encode_into_buf(0x1234, b"xyz".iter().copied(), &mut encoded).unwrap();
        assert_eq!(encoded, vec![0x92, 0x34, 0x78, 0x79, 0x7a]);
        let owned_msg = OwnedMsg::try_from(encoded.as_slice()).unwrap();
        assert_eq!(owned_msg.ty(), 0x1234);
        assert_eq!(owned_msg.body(), b"xyz");
    }

    #[test]
    fn test_message_wrapper() {
        let type_id = 0x1234;
        let body = vec![0x01, 0x02, 0x03];
        let msg = Message::new(type_id, body.clone()).unwrap();

        assert_eq!(msg.ty(), type_id);
        assert_eq!(msg.body(), &body);

        // Test encoding/decoding roundtrip
        let encoded = msg.encode().unwrap();
        let decoded_msg = Message::from_encoded(encoded).unwrap();
        assert_eq!(decoded_msg.ty(), type_id);
        assert_eq!(decoded_msg.body(), &body);
    }

    #[test]
    fn test_compatibility_with_strata_msg_fmt() {
        let type_id = 0x42;
        let body = vec![0x11, 0x22, 0x33];

        // Create using strata-msg-fmt directly
        let msg = OwnedMsg::new(type_id, body.clone()).unwrap();

        // Test SPS-msg-fmt compliance via Message
        let message_wrapper = Message(msg.clone());
        let encoded = message_wrapper.encode().unwrap();
        let parsed_message_wrapper = Message::from_encoded(encoded).unwrap();
        assert_eq!(parsed_message_wrapper.ty(), msg.ty());
        assert_eq!(parsed_message_wrapper.body(), msg.body());

        // Test OwnedMsg conversion roundtrip
        let converted_msg = message_wrapper.to_msg();
        assert_eq!(converted_msg.ty(), type_id);
        assert_eq!(converted_msg.body(), body);
    }
}
