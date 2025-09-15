// Re-export from the separate logs crate
use borsh::{BorshDeserialize, BorshSerialize};
use strata_msg_fmt::{Msg, MsgRef, OwnedMsg, TypeId};

use crate::{AsmError, AsmResult};

/// Trait for ASM log types that can be serialized and stored.
///
/// This trait provides a consistent interface for log entries that need to be
/// serialized, stored, and later deserialized from the ASM state. Each log type
/// has a unique type identifier and must be serializable.
// TODO migrate from borsh for this
pub trait AsmLog: BorshSerialize + BorshDeserialize {
    /// Unique type identifier for this log type.
    ///
    /// This constant is used to distinguish between different log types when
    /// serializing and deserializing log entries.
    const TY: TypeId;
}

/// A wrapper around raw bytes that provides typed access to ASM log entries.
///
/// `AsmLogEntry` encapsulates raw log data as bytes, providing a consistent interface
/// for storing and retrieving different types of ASM log events. The raw bytes can
/// optionally be interpreted as an SPS-52 message with type information.
///
/// # Usage
///
/// Create log entries using [`AsmLogEntry::from_log`], [`AsmLogEntry::from_raw`], or
/// [`AsmLogEntry::from_msg`], and retrieve typed data using [`AsmLogEntry::try_into_log`]
/// or check if it's a valid SPS-52 message using [`AsmLogEntry::try_as_msg`].
#[derive(Clone, Debug, PartialEq)]
pub struct AsmLogEntry(pub Vec<u8>);

impl AsmLogEntry {
    /// Create an AsmLogEntry directly from raw bytes.
    ///
    /// This is the most basic constructor - logs are just bytes.
    pub fn from_raw(bytes: Vec<u8>) -> Self {
        AsmLogEntry(bytes)
    }

    /// Create an AsmLogEntry from SPS-52 message components.
    ///
    /// This creates a properly formatted SPS-52 message with type ID and body.
    pub fn from_msg(ty: TypeId, body: Vec<u8>) -> AsmResult<Self> {
        let owned_msg = OwnedMsg::new(ty, body)?;
        Ok(AsmLogEntry(owned_msg.to_vec()))
    }

    /// Create an AsmLogEntry from any type that implements AsmLog.
    ///
    /// This provides backwards compatibility with typed log entries.
    pub fn from_log<T: AsmLog>(log: &T) -> AsmResult<Self> {
        let ty = TypeId::from(T::TY);
        // TODO as above, migrate from borsh for this
        let body = borsh::to_vec(log).map_err(|e| AsmError::TypeIdSerialization(ty, e))?;
        Self::from_msg(ty, body)
    }

    /// Try to interpret the raw bytes as an SPS-52 message.
    ///
    /// Returns None if the bytes don't form a valid SPS-52 message.
    /// This allows logs to be either structured messages or arbitrary bytes.
    pub fn try_as_msg(&self) -> Option<MsgRef<'_>> {
        MsgRef::try_from(self.0.as_slice()).ok()
    }

    /// Get the type ID if this is a valid SPS-52 message.
    ///
    /// Returns None if the log is not a valid message.
    pub fn ty(&self) -> Option<TypeId> {
        self.try_as_msg().map(|msg| msg.ty())
    }

    /// Try to deserialize the log entry to a specific AsmLog type.
    ///
    /// This only works if the log is a valid SPS-52 message with the correct type ID.
    pub fn try_into_log<T: AsmLog>(&self) -> AsmResult<T> {
        // Parse as message, propagating any parsing errors
        let msg = MsgRef::try_from(self.0.as_slice())?;

        let expected_ty = T::TY;
        let actual_ty = msg.ty();

        if actual_ty != expected_ty {
            return Err(AsmError::TypeIdMismatch(crate::Mismatched {
                expected: expected_ty,
                actual: actual_ty,
            }));
        }

        // TODO as above, migrate from borsh for this
        borsh::from_slice(msg.body()).map_err(|e| AsmError::TypeIdDeserialization(expected_ty, e))
    }

    /// Get the raw bytes of this log entry.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Consume the log entry and return the raw bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

// TODO: Remove these Borsh implementations when upstream OwnedMsg implements
// Serialize/Deserialize The Message wrapper was primarily created to add serialization support.
impl BorshSerialize for AsmLogEntry {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        // Serialize as (ty, body) tuple for Borsh compatibility
        (self.0.ty(), self.0.body().to_vec()).serialize(writer)
    }
}

impl BorshDeserialize for AsmLogEntry {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        // Deserialize as (ty, body) tuple for Borsh compatibility
        let (ty, body): (TypeId, Vec<u8>) = BorshDeserialize::deserialize_reader(reader)?;
        let owned_msg = OwnedMsg::new(ty, body)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        Ok(AsmLogEntry(owned_msg))
    }
}
