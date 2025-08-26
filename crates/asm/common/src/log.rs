// Re-export from the separate logs crate
use borsh::{BorshDeserialize, BorshSerialize};
use strata_msg_fmt::{Msg, OwnedMsg, TypeId};

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

/// A wrapper around [`OwnedMsg`] that provides typed access to ASM log entries.
///
/// `AsmLogEntry` encapsulates a message with a type identifier and serialized data body,
/// providing a consistent interface for storing and retrieving different types of ASM log
/// events. Each log entry contains:
///
/// - A [`TypeId`] that uniquely identifies the log type
/// - A serialized data body containing the log event information
///
/// The underlying [`OwnedMsg`] handles the storage of the type identifier and serialized
/// data, while `AsmLogEntry` provides type-safe methods for creating and accessing log
/// entries through the [`AsmLog`] trait.
///
/// # Usage
///
/// Create log entries using [`AsmLogEntry::from_log`] and retrieve typed data using
/// [`AsmLogEntry::try_into_log`]. The type safety is enforced through the [`TypeId`]
/// matching system.
#[derive(Clone, Debug)]
pub struct AsmLogEntry(pub OwnedMsg);

impl AsmLogEntry {
    pub fn ty(&self) -> TypeId {
        self.0.ty()
    }

    /// Create an AsmLogEntry from any type that implements AsmLog.
    pub fn from_log<T: AsmLog>(log: &T) -> AsmResult<Self> {
        let ty = TypeId::from(T::TY);
        // TODO as above, migrate from borsh for this
        let body = borsh::to_vec(log).map_err(|e| AsmError::TypeIdSerialization(ty, e))?;
        let owned_msg = OwnedMsg::new(ty, body)?;
        Ok(AsmLogEntry(owned_msg))
    }

    /// Try to deserialize the log entry to a specific AsmLog type.
    pub fn try_into_log<T: AsmLog>(&self) -> AsmResult<T> {
        let expected_ty = T::TY;
        let actual_ty = self.0.ty();

        if actual_ty != expected_ty {
            return Err(AsmError::TypeIdMismatch(crate::Mismatched {
                expected: expected_ty,
                actual: actual_ty,
            }));
        }

        // TODO as above, migrate from borsh for this
        borsh::from_slice(self.0.body())
            .map_err(|e| AsmError::TypeIdDeserialization(expected_ty, e))
    }
}
