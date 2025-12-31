//! EE account initialization data

use strata_codec::{Codec, CodecError, Decoder, Encoder};

/// Fields that initialize the EE account state
#[derive(Debug, Clone)]
pub struct EeAccountInit {
    /// EeAccountState encoded as SSZ bytes
    astate_ssz: Vec<u8>,

    /// Previous ProofState (before the update) encoded as SSZ bytes
    /// The guest will verify that tree_hash_root(astate) matches this state
    prev_proof_state_ssz: Vec<u8>,
}

impl EeAccountInit {
    /// Create a new EeAccountInit
    pub fn new(astate_ssz: Vec<u8>, prev_proof_state_ssz: Vec<u8>) -> Self {
        Self {
            astate_ssz,
            prev_proof_state_ssz,
        }
    }

    /// Get reference to astate SSZ bytes
    pub fn astate_ssz(&self) -> &[u8] {
        &self.astate_ssz
    }

    /// Get reference to previous proof state SSZ bytes
    pub fn prev_proof_state_ssz(&self) -> &[u8] {
        &self.prev_proof_state_ssz
    }

    /// Consume and return astate SSZ bytes
    pub fn into_astate_ssz(self) -> Vec<u8> {
        self.astate_ssz
    }

    /// Consume and return previous proof state SSZ bytes
    pub fn into_prev_proof_state_ssz(self) -> Vec<u8> {
        self.prev_proof_state_ssz
    }

    /// Destructure into both components
    pub fn into_parts(self) -> (Vec<u8>, Vec<u8>) {
        (self.astate_ssz, self.prev_proof_state_ssz)
    }
}

impl Codec for EeAccountInit {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        // Encode astate_ssz length and data
        let astate_len = self.astate_ssz.len() as u32;
        astate_len.encode(enc)?;
        enc.write_buf(&self.astate_ssz)?;

        // Encode prev_proof_state_ssz length and data
        let prev_state_len = self.prev_proof_state_ssz.len() as u32;
        prev_state_len.encode(enc)?;
        enc.write_buf(&self.prev_proof_state_ssz)?;

        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        // Decode astate_ssz
        let astate_len = u32::decode(dec)? as usize;
        let mut astate_ssz = vec![0u8; astate_len];
        dec.read_buf(&mut astate_ssz)?;

        // Decode prev_proof_state_ssz
        let prev_state_len = u32::decode(dec)? as usize;
        let mut prev_proof_state_ssz = vec![0u8; prev_state_len];
        dec.read_buf(&mut prev_proof_state_ssz)?;

        Ok(Self {
            astate_ssz,
            prev_proof_state_ssz,
        })
    }
}
