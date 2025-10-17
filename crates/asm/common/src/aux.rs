use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_aux::{AuxRequestSpec, AuxResponseSpec};

use crate::{AsmError, RequesterL1Index, Subprotocol};

/// A single auxiliary input request from a subprotocol during preprocessing.
///
/// The `data` field captures the schema required to derive the corresponding
/// auxiliary input data in the final [`AuxPayload`].
#[derive(Debug)]
pub struct AuxRequest {
    /// Schema describing how to fulfil the auxiliary input request.
    data: AuxRequestSpec,

    /// The L1 transaction index that this aux request is associated with.
    l1_tx_index: RequesterL1Index,
}

impl AuxRequest {
    pub fn new(data: AuxRequestSpec, l1_tx_index: RequesterL1Index) -> Self {
        Self { data, l1_tx_index }
    }

    pub fn data(&self) -> &AuxRequestSpec {
        &self.data
    }

    pub fn requester_index(&self) -> RequesterL1Index {
        self.l1_tx_index
    }

    pub fn encode(&self) -> Vec<u8> {
        borsh::to_vec(&self.data).expect("asm: serialize aux request")
    }
}

/// A single subprotocol's auxiliary input payload, containing processed auxiliary data.
///
/// Each [`AuxRequest`] must resolve into a corresponding [`AuxPayload`] before the main
/// processing phase can begin. The `data` field must deserialize into an instance of
/// [`Subprotocol::AuxInput`] for the associated subprotocol.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct AuxPayload {
    /// Processed auxiliary data returned by the aux service.
    pub data: AuxResponseSpec,

    /// The L1 transaction index that this aux payload is associated with.
    pub l1_tx_index: RequesterL1Index,
}

impl AuxPayload {
    pub fn new(data: AuxResponseSpec, l1_tx_index: RequesterL1Index) -> Self {
        Self { data, l1_tx_index }
    }

    pub fn response(&self) -> &AuxResponseSpec {
        &self.data
    }

    pub fn requester_index(&self) -> RequesterL1Index {
        self.l1_tx_index
    }

    /// Tries to merkle-proof validate the aux payload for the given subprotocol.
    pub fn validate<S: Subprotocol>(&self) -> Result<(), AsmError> {
        match &self.data {
            AuxResponseSpec::Single(_resp) => todo!(),
            AuxResponseSpec::Range(_resps) => todo!(),
        }
    }
}
