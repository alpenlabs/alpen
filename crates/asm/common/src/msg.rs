//! Message related types using strata-msg-fmt.

use std::any::Any;

use borsh::{BorshDeserialize, BorshSerialize};
use strata_l1_txfmt::SubprotocolId;
// Re-export standard types for convenience
pub use strata_msg_fmt::{Error as MessageError, Msg, OwnedMsg as Message, TypeId as MessageType};
use strata_msg_fmt::{OwnedMsg, TypeId};

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

/// Encodes a message type and body into SPS-msg-fmt format
fn encode_sps_msg(ty: TypeId, body: &[u8]) -> Result<Vec<u8>, AsmError> {
    let mut result = Vec::new();
    if ty < 0x80 {
        // Single byte encoding
        result.push(ty as u8);
    } else if ty < 0x8000 {
        // Two byte encoding
        result.push(0x80 | ((ty >> 8) as u8));
        result.push(ty as u8);
    } else {
        // Use strata_msg_fmt::Error directly - AsmError will convert it
        return Err(MessageError::TypeOutOfBounds(ty).into());
    }
    result.extend_from_slice(body);
    Ok(result)
}

/// Decodes SPS-msg-fmt format into message type and body
fn decode_sps_msg(encoded: &[u8]) -> Result<(TypeId, &[u8]), AsmError> {
    if encoded.is_empty() {
        return Err(MessageError::BufEmpty.into());
    }

    let (ty, body_start) = if encoded[0] < 0x80 {
        // Single byte encoding
        (encoded[0] as TypeId, 1)
    } else {
        // Two byte encoding
        if encoded.len() < 2 {
            return Err(MessageError::BufTooShort.into());
        }
        let ty = (((encoded[0] & 0x7F) as TypeId) << 8) | (encoded[1] as TypeId);
        (ty, 2)
    };

    Ok((ty, &encoded[body_start..]))
}

/// Generic message from OL to ASM using strata-msg-fmt
///
/// This type wraps messages in the SPS-msg-fmt format, allowing for
/// different message types to be sent from OL to ASM (e.g., withdrawals,
/// upgrade messages, etc.)
///
/// We store the type and body separately for Borsh compatibility while
/// maintaining full SPS-msg-fmt compliance through encoding/decoding.
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize, PartialEq, Eq)]
pub struct OLToASMMessage {
    /// Message type identifier
    ty: TypeId,
    /// Message body
    body: Vec<u8>,
}

impl OLToASMMessage {
    /// Creates a new OL to ASM message from type and body
    pub fn new(ty: TypeId, body: Vec<u8>) -> Result<Self, AsmError> {
        // Validate type is within SPS-msg-fmt bounds
        if ty > 0x7FFF {
            return Err(MessageError::TypeOutOfBounds(ty).into());
        }
        Ok(Self { ty, body })
    }

    /// Creates an OL to ASM message from an OwnedMsg
    pub fn from_msg(msg: &OwnedMsg) -> Self {
        Self {
            ty: msg.ty(),
            body: msg.body().to_vec(),
        }
    }

    /// Creates a new OL to ASM message from raw encoded bytes
    pub fn from_encoded(encoded_bytes: Vec<u8>) -> Result<Self, AsmError> {
        let (ty, body) = decode_sps_msg(&encoded_bytes)?;
        Ok(Self {
            ty,
            body: body.to_vec(),
        })
    }

    /// Creates an OwnedMsg from this message
    pub fn to_msg(&self) -> Result<OwnedMsg, MessageError> {
        OwnedMsg::new(self.ty, self.body.clone())
    }

    /// Returns the message type
    pub fn ty(&self) -> TypeId {
        self.ty
    }

    /// Returns the message body
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// Returns the message body as Vec
    pub fn body_vec(&self) -> Vec<u8> {
        self.body.clone()
    }

    /// Encodes the message to SPS-msg-fmt bytes
    pub fn encode(&self) -> Result<Vec<u8>, AsmError> {
        encode_sps_msg(self.ty, &self.body)
    }

    /// Returns the raw encoded message bytes (alias for encode)
    pub fn encoded(&self) -> Result<Vec<u8>, AsmError> {
        self.encode()
    }

    /// Compatibility method for old API - returns OwnedMsg
    pub fn decode(&self) -> Result<OwnedMsg, MessageError> {
        self.to_msg()
    }
}

/// Temporary alias for backwards compatibility
/// TODO: Remove once all code is updated to use OLToASMMessage
pub type L2ToL1Msg = OLToASMMessage;

/// Generic container for multiple messages following SPS-msg-fmt
#[derive(Clone, Debug)]
pub struct MessagesContainer {
    /// The target subprotocol ID
    target_subprotocol: SubprotocolId,
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

impl InterprotoMsg for MessagesContainer {
    fn id(&self) -> SubprotocolId {
        self.target_subprotocol
    }

    fn as_dyn_any(&self) -> &dyn Any {
        self
    }
}

/// Alias for OwnedMsg to maintain compatibility
pub type Log = OwnedMsg;

#[cfg(test)]
mod tests {
    use std::any::Any;

    use strata_l1_txfmt::SubprotocolId;
    use strata_msg_fmt::{Msg, OwnedMsg};

    use super::{InterprotoMsg, OLToASMMessage, decode_sps_msg, encode_sps_msg};

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
        let inst = Foo { x: 5 };
        inst.x();
        let _inst_box = Box::new(inst) as Box<dyn InterprotoMsg>;
    }

    #[test]
    fn test_sps_msg_fmt_encoding() {
        // type 0x00 body "hello" → 0068656c6c6f
        let encoded = encode_sps_msg(0x00, b"hello").unwrap();
        assert_eq!(encoded, vec![0x00, 0x68, 0x65, 0x6c, 0x6c, 0x6f]);
        let (ty, body) = decode_sps_msg(&encoded).unwrap();
        assert_eq!(ty, 0x00);
        assert_eq!(body, b"hello");

        // type 0x80 body "abc" → 8080616263
        let encoded = encode_sps_msg(0x80, b"abc").unwrap();
        assert_eq!(encoded, vec![0x80, 0x80, 0x61, 0x62, 0x63]);
        let (ty, body) = decode_sps_msg(&encoded).unwrap();
        assert_eq!(ty, 0x80);
        assert_eq!(body, b"abc");

        // type 0x1234 body "xyz" → 923478797a
        let encoded = encode_sps_msg(0x1234, b"xyz").unwrap();
        assert_eq!(encoded, vec![0x92, 0x34, 0x78, 0x79, 0x7a]);
        let (ty, body) = decode_sps_msg(&encoded).unwrap();
        assert_eq!(ty, 0x1234);
        assert_eq!(body, b"xyz");
    }

    #[test]
    fn test_ol_to_asm_message() {
        let type_id = 0x1234;
        let body = vec![0x01, 0x02, 0x03];
        let ol_msg = OLToASMMessage::new(type_id, body.clone()).unwrap();

        assert_eq!(ol_msg.ty(), type_id);
        assert_eq!(ol_msg.body(), &body);

        // Test encoding/decoding roundtrip
        let encoded = ol_msg.encode().unwrap();
        let decoded_ol_msg = OLToASMMessage::from_encoded(encoded).unwrap();
        assert_eq!(decoded_ol_msg.ty(), type_id);
        assert_eq!(decoded_ol_msg.body(), &body);
    }

    #[test]
    fn test_compatibility_with_strata_msg_fmt() {
        let type_id = 0x42;
        let body = vec![0x11, 0x22, 0x33];

        // Create using strata-msg-fmt directly
        let msg = OwnedMsg::new(type_id, body.clone()).unwrap();

        // Test SPS-msg-fmt compliance via OLToASMMessage
        let ol_msg = OLToASMMessage::from_msg(&msg);
        let encoded = ol_msg.encode().unwrap();
        let parsed_ol_msg = OLToASMMessage::from_encoded(encoded).unwrap();
        assert_eq!(parsed_ol_msg.ty(), msg.ty());
        assert_eq!(parsed_ol_msg.body(), msg.body());

        // Test OwnedMsg conversion roundtrip
        let converted_msg = ol_msg.to_msg().unwrap();
        assert_eq!(converted_msg.ty(), type_id);
        assert_eq!(converted_msg.body(), body);
    }
}
