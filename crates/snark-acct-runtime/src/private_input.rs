//! Private inputs to runtime update proof.

use rkyv::{Archive, Deserialize, Serialize};
use rkyv_impl::archive_impl;
use ssz::{Decode, DecodeError, Encode};
use strata_snark_acct_types::UpdateProofPubParams;

use crate::IInnerState;

/// Private inputs we expose to the runtime.
#[derive(Clone, Debug, Archive, Deserialize, Serialize)]
#[rkyv(derive(Debug))]
pub struct PrivateInput {
    update_pub_params_ssz: Vec<u8>,
    raw_pre_state: Vec<u8>,
    coinputs: Vec<Coinput>,
}

impl PrivateInput {
    pub fn new(
        update_pub_params: UpdateProofPubParams,
        raw_pre_state: Vec<u8>,
        coinputs: Vec<Coinput>,
    ) -> Self {
        Self {
            update_pub_params_ssz: update_pub_params.as_ssz_bytes(),
            raw_pre_state,
            coinputs,
        }
    }

    pub fn coinputs(&self) -> &[Coinput] {
        &self.coinputs
    }
}

#[archive_impl]
impl PrivateInput {
    pub fn update_pub_params_ssz(&self) -> &[u8] {
        &self.update_pub_params_ssz
    }

    pub fn raw_pre_state(&self) -> &[u8] {
        &self.raw_pre_state
    }

    /// Tries to decode the proof pub params as its type.
    pub fn try_decode_update_pub_params(&self) -> Result<UpdateProofPubParams, DecodeError> {
        UpdateProofPubParams::from_ssz_bytes(self.update_pub_params_ssz())
    }

    /// Tries to decode the inner pre-state as its type, generically.
    pub fn try_decode_pre_state<S: IInnerState>(&self) -> Result<S, DecodeError> {
        S::from_ssz_bytes(self.raw_pre_state())
    }
}

impl ArchivedPrivateInput {
    pub fn coinputs(&self) -> &[ArchivedCoinput] {
        &self.coinputs
    }
}

/// Coinput data.
#[derive(Clone, Debug, Archive, Deserialize, Serialize)]
#[rkyv(derive(Debug))]
pub struct Coinput {
    raw_data: Vec<u8>,
}

impl Coinput {
    pub fn new(raw_data: Vec<u8>) -> Self {
        Self { raw_data }
    }
}

#[archive_impl]
impl Coinput {
    pub fn raw_data(&self) -> &[u8] {
        &self.raw_data
    }
}
