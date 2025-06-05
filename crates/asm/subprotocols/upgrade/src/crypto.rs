//! FIXME: All of the code here is only meant as placeholder for now. This needs to based on the
//! strata-crypto crate
use borsh::{BorshDeserialize, BorshSerialize};
use strata_primitives::{buf::Buf32, hash};

use crate::error::VoteValidationError;

/// Macro to define a newtype wrapper around `Vec<u8>` with common implementations.
macro_rules! define_byte_wrapper {
    ($name:ident) => {
        /// A type wrapping a [`Vec<u8>`] with common trait implementations,
        /// allowing easy serialization, comparison, and other utility operations.
        #[derive(Debug, Clone, BorshSerialize, BorshDeserialize, PartialEq, Eq, Default)]
        pub struct $name(Vec<u8>);

        impl $name {
            /// Creates a new instance from a `Vec<u8>`.
            pub fn new(data: Vec<u8>) -> Self {
                Self(data)
            }

            /// Returns a reference to the inner byte slice.
            pub fn as_bytes(&self) -> &[u8] {
                &self.0
            }

            /// Consumes the wrapper and returns the inner `Vec<u8>`.
            pub fn into_inner(self) -> Vec<u8> {
                self.0
            }

            /// Checks if the byte vector is empty.
            pub fn is_empty(&self) -> bool {
                self.0.is_empty()
            }
        }

        impl From<$name> for Vec<u8> {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl From<&$name> for Vec<u8> {
            fn from(value: &$name) -> Self {
                value.0.clone()
            }
        }

        impl From<&[u8]> for $name {
            fn from(value: &[u8]) -> Self {
                Self(value.to_vec())
            }
        }
    };
}

// Use the macro to define the specific types.
define_byte_wrapper!(PubKey);
define_byte_wrapper!(Signature);

// FIXME: handle
pub fn aggregate_pubkeys(_keys: &[PubKey]) -> Result<PubKey, VoteValidationError> {
    Ok(PubKey::default())
}

// FIXME: handle
pub fn verify_sig(_pk: &PubKey, _msg_hash: &Buf32, _sig: &Signature) -> bool {
    true
}

/// Compute a “tagged” SHA-256 digest of any Borsh‐serializable object.
pub fn tagged_hash<T: BorshSerialize>(tag_bytes: &[u8], value: &T) -> Buf32 {
    let serialized = borsh::to_vec(value).expect("borsh serialization failed");

    // Allocate a Vec just large enough for tag + data:
    let mut buf = Vec::with_capacity(tag_bytes.len() + serialized.len());
    buf.extend_from_slice(tag_bytes);
    buf.extend_from_slice(&serialized);

    // Perform raw SHA-256
    hash::raw(&buf)
}
