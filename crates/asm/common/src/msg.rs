//! Message related types.

use std::any::Any;

use borsh::{BorshDeserialize, BorshSerialize};

use crate::SubprotocolId;

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

/// Generic message from OL to ASM
///
/// This type wraps messages in the SPS-msg-fmt format, allowing for
/// different message types to be sent from OL to ASM (e.g., withdrawals,
/// upgrade messages, etc.)
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize, PartialEq, Eq)]
pub struct OLToASMMessage {
    /// The encoded message following SPS-msg-fmt
    encoded_message: Vec<u8>,
}

impl OLToASMMessage {
    /// Creates a new OL to ASM message from a Message
    pub fn new(message: &Message) -> Self {
        Self {
            encoded_message: message.encode(),
        }
    }

    /// Creates a new OL to ASM message from raw encoded bytes
    pub fn from_encoded(encoded_message: Vec<u8>) -> Self {
        Self { encoded_message }
    }

    /// Decodes the message
    pub fn decode(&self) -> Result<Message, MessageError> {
        Message::decode(&self.encoded_message)
    }

    /// Returns the raw encoded message bytes
    pub fn encoded(&self) -> &[u8] {
        &self.encoded_message
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
    /// Messages encoded in SPS-msg-fmt format
    pub messages: Vec<Message>,
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
    pub fn add_message(&mut self, message: Message) {
        self.messages.push(message);
    }

    /// Creates a new container with the provided messages
    pub fn with_messages(target_subprotocol: SubprotocolId, messages: Vec<Message>) -> Self {
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

/// Message type identifier following SPS-msg-fmt spec
pub type MessageType = u16;

/// Generic message format following SPS-msg-fmt spec
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    /// Type identifier (0..32768)
    ty: MessageType,
    /// Body of the message
    body: Vec<u8>,
}

impl Message {
    /// Maximum allowed message type value (2^15 - 1)
    pub const MAX_TYPE: u16 = 0x7FFF;

    /// Constructor
    pub fn new(ty: MessageType, body: Vec<u8>) -> Result<Self, MessageError> {
        if ty > Self::MAX_TYPE {
            return Err(MessageError::TypeOutOfBounds(ty));
        }
        Ok(Self { ty, body })
    }

    /// Returns type identifier
    pub fn ty(&self) -> MessageType {
        self.ty
    }

    /// Returns slice of body
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// Encodes the message into bytes following SPS-msg-fmt
    pub fn encode(&self) -> Vec<u8> {
        encode_message(self.ty, &self.body)
    }

    /// Decodes a message from bytes following SPS-msg-fmt
    pub fn decode(buf: &[u8]) -> Result<Self, MessageError> {
        let (ty, body) = decode_message(buf)?;
        Ok(Self {
            ty,
            body: body.to_vec(),
        })
    }
}

/// Errors that can occur during message encoding/decoding
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageError {
    /// Buffer is too short
    BufferTooShort,
    /// Type value is out of bounds (> 0x7FFF)
    TypeOutOfBounds(u16),
    /// Non-minimal encoding was used
    NonMinimalEncoding(u16),
}

impl std::fmt::Display for MessageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MessageError::BufferTooShort => write!(f, "buffer too short"),
            MessageError::TypeOutOfBounds(ty) => write!(f, "type {ty:#x} out of bounds"),
            MessageError::NonMinimalEncoding(ty) => {
                write!(f, "non-minimal encoding for type {ty:#x}")
            }
        }
    }
}

impl std::error::Error for MessageError {}

/// Decodes a message type and body from a buffer following SPS-msg-fmt
pub fn decode_message(buf: &[u8]) -> Result<(MessageType, &[u8]), MessageError> {
    if buf.is_empty() {
        return Err(MessageError::BufferTooShort);
    }

    let first_byte = buf[0];

    if first_byte & 0x80 == 0 {
        // Single byte encoding: type is in range 0..128
        Ok((first_byte as u16, &buf[1..]))
    } else {
        // Two byte encoding: type is in range 128..32768
        if buf.len() < 2 {
            return Err(MessageError::BufferTooShort);
        }

        let second_byte = buf[1];
        let ty = ((first_byte & 0x7F) as u16) << 8 | (second_byte as u16);

        // Check for non-minimal encoding
        if ty < 0x80 {
            return Err(MessageError::NonMinimalEncoding(ty));
        }

        Ok((ty, &buf[2..]))
    }
}

/// Encodes a message type and body into a buffer following SPS-msg-fmt
pub fn encode_message(ty: MessageType, body: &[u8]) -> Vec<u8> {
    if ty < 0x80 {
        // Single byte encoding
        let mut result = vec![ty as u8];
        result.extend_from_slice(body);
        result
    } else {
        // Two byte encoding
        let first_byte = (ty >> 8) as u8 | 0x80;
        let second_byte = ty as u8;
        let mut result = vec![first_byte, second_byte];
        result.extend_from_slice(body);
        result
    }
}

/// Alias for Message to maintain compatibility
pub type Log = Message;

#[cfg(test)]
mod tests {
    use std::any::Any;

    use super::{InterprotoMsg, Message, MessageError, decode_message, encode_message};
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

    #[test]
    fn test_sps_msg_fmt_encoding() {
        // Test vectors from the spec that should encode and decode successfully

        // type 0x00 body "hello" → 0068656c6c6f
        let encoded = encode_message(0x00, b"hello");
        assert_eq!(encoded, vec![0x00, 0x68, 0x65, 0x6c, 0x6c, 0x6f]);
        let (ty, body) = decode_message(&encoded).unwrap();
        assert_eq!(ty, 0x00);
        assert_eq!(body, b"hello");

        // type 0x01 body (empty string) → 01
        let encoded = encode_message(0x01, b"");
        assert_eq!(encoded, vec![0x01]);
        let (ty, body) = decode_message(&encoded).unwrap();
        assert_eq!(ty, 0x01);
        assert_eq!(body, b"");

        // type 0x7f body 00ff (raw) → 7f00ff
        let encoded = encode_message(0x7f, &[0x00, 0xff]);
        assert_eq!(encoded, vec![0x7f, 0x00, 0xff]);
        let (ty, body) = decode_message(&encoded).unwrap();
        assert_eq!(ty, 0x7f);
        assert_eq!(body, &[0x00, 0xff]);

        // type 0x80 body "abc" → 8080616263
        let encoded = encode_message(0x80, b"abc");
        assert_eq!(encoded, vec![0x80, 0x80, 0x61, 0x62, 0x63]);
        let (ty, body) = decode_message(&encoded).unwrap();
        assert_eq!(ty, 0x80);
        assert_eq!(body, b"abc");

        // type 0x1234 body "xyz" → 923478797a
        let encoded = encode_message(0x1234, b"xyz");
        assert_eq!(encoded, vec![0x92, 0x34, 0x78, 0x79, 0x7a]);
        let (ty, body) = decode_message(&encoded).unwrap();
        assert_eq!(ty, 0x1234);
        assert_eq!(body, b"xyz");

        // type 0x7fff body 1020 (raw) → ffff1020
        let encoded = encode_message(0x7fff, &[0x10, 0x20]);
        assert_eq!(encoded, vec![0xff, 0xff, 0x10, 0x20]);
        let (ty, body) = decode_message(&encoded).unwrap();
        assert_eq!(ty, 0x7fff);
        assert_eq!(body, &[0x10, 0x20]);
    }

    #[test]
    fn test_sps_msg_fmt_decoding_errors() {
        // Test vectors that should fail decoding

        // (empty buffer) - empty buffer
        assert_eq!(decode_message(&[]), Err(MessageError::BufferTooShort));

        // 80 - buffer too short
        assert_eq!(decode_message(&[0x80]), Err(MessageError::BufferTooShort));

        // 8000 - non-minimal encoding
        assert_eq!(
            decode_message(&[0x80, 0x00]),
            Err(MessageError::NonMinimalEncoding(0))
        );

        // 807f - non-minimal encoding
        assert_eq!(
            decode_message(&[0x80, 0x7f]),
            Err(MessageError::NonMinimalEncoding(0x7f))
        );
    }

    #[test]
    fn test_message_struct() {
        // Test the Message struct methods
        let msg = Message::new(0x42, b"test data".to_vec()).unwrap();
        assert_eq!(msg.ty(), 0x42);
        assert_eq!(msg.body(), b"test data");

        // Test encoding/decoding roundtrip
        let encoded = msg.encode();
        let decoded = Message::decode(&encoded).unwrap();
        assert_eq!(decoded.ty(), msg.ty());
        assert_eq!(decoded.body(), msg.body());

        // Test type out of bounds
        assert!(matches!(
            Message::new(0x8000, vec![]),
            Err(MessageError::TypeOutOfBounds(0x8000))
        ));
    }
}
