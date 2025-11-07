use borsh::{BorshDeserialize, BorshSerialize};

use crate::{AsmError, Subprotocol};

/// A single auxiliary input request from a subprotocol during preprocessing.
///
/// The `data` field contains the raw bytes that will be processed to generate
/// the corresponding auxiliary input data in the final [`AuxPayload`].
#[derive(Debug)]
pub struct AuxRequest {
    /// Raw data for the auxiliary input request.
    data: Vec<u8>,
}

impl AuxRequest {
    pub fn new(data: Vec<u8>) -> Self {
        Self { data }
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }
}

/// A single subprotocol's auxiliary input payload, containing processed auxiliary data.
///
/// Each [`AuxRequest`] must resolve into a corresponding [`AuxPayload`] before the main
/// processing phase can begin. The `data` field must deserialize into an instance of
/// [`Subprotocol::AuxInput`] for the associated subprotocol.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct AuxPayload {
    /// Processed auxiliary input data as raw bytes.
    ///
    /// This `Vec<u8>` must deserialize into one
    /// `<P as Subprotocol>::AuxInput` for the corresponding subprotocol P.
    pub data: Vec<u8>,
}

impl AuxPayload {
    pub fn new(data: Vec<u8>) -> Self {
        Self { data }
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Tries to parse as a subprotocol's aux input.
    ///
    /// This MUST NOT be called on a payload that does not correspond to the
    /// subprotocol type, because this may lead to silent errors.
    pub fn try_to_aux_input<S: Subprotocol>(&self) -> Result<S::AuxInput, AsmError> {
        <S::AuxInput as BorshDeserialize>::try_from_slice(&self.data)
            .map_err(|e| AsmError::Deserialization(S::ID, e))
    }
}
