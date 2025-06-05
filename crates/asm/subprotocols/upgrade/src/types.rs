use borsh::{BorshDeserialize, BorshSerialize};

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
// FIXME: This is only meant as placeholder for now. This needs to based on the strata-crypto crate
define_byte_wrapper!(PubKey);
define_byte_wrapper!(Signature);
