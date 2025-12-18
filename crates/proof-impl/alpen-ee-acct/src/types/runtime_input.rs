//! Runtime update input data

use strata_codec::{Codec, CodecError, Decoder, Encoder};

use super::CommitBlockPackage;

/// Fields we plug into the EE runtime to process the update
#[derive(Debug, Clone)]
pub struct RuntimeUpdateInput {
    /// UpdateOperationData encoded as SSZ bytes
    operation_ssz: Vec<u8>,

    /// Coinput witness data for messages
    coinputs: Vec<Vec<u8>>,

    /// Serialized block packages for building CommitChainSegment in guest.
    /// Each package contains execution metadata and raw block body.
    /// TODO: still not sure with the name, even for the Data type.
    commit_block_packages: Vec<CommitBlockPackage>,

    /// Previous header (raw bytes)
    raw_prev_header: Vec<u8>,

    /// Partial pre-state (raw bytes)
    raw_partial_pre_state: Vec<u8>,
}

impl RuntimeUpdateInput {
    /// Create a new RuntimeUpdateInput
    pub fn new(
        operation_ssz: Vec<u8>,
        coinputs: Vec<Vec<u8>>,
        commit_block_packages: Vec<CommitBlockPackage>,
        raw_prev_header: Vec<u8>,
        raw_partial_pre_state: Vec<u8>,
    ) -> Self {
        Self {
            operation_ssz,
            coinputs,
            commit_block_packages,
            raw_prev_header,
            raw_partial_pre_state,
        }
    }

    /// Get reference to operation SSZ bytes
    pub fn operation_ssz(&self) -> &[u8] {
        &self.operation_ssz
    }

    /// Get reference to coinputs
    pub fn coinputs(&self) -> &[Vec<u8>] {
        &self.coinputs
    }

    /// Get reference to commit block packages
    pub fn commit_block_packages(&self) -> &[CommitBlockPackage] {
        &self.commit_block_packages
    }

    /// Get reference to raw previous header
    pub fn raw_prev_header(&self) -> &[u8] {
        &self.raw_prev_header
    }

    /// Get reference to raw partial pre-state
    pub fn raw_partial_pre_state(&self) -> &[u8] {
        &self.raw_partial_pre_state
    }
}

impl Codec for RuntimeUpdateInput {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        // Encode operation_ssz
        let op_len = self.operation_ssz.len() as u32;
        op_len.encode(enc)?;
        enc.write_buf(&self.operation_ssz)?;

        // Encode coinputs (Vec<Vec<u8>>)
        let coinputs_len = self.coinputs.len() as u32;
        coinputs_len.encode(enc)?;
        for coinput in &self.coinputs {
            let coinput_len = coinput.len() as u32;
            coinput_len.encode(enc)?;
            enc.write_buf(coinput)?;
        }

        // Encode commit_block_packages (Vec<CommitBlockPackage>)
        let packages_len = self.commit_block_packages.len() as u32;
        packages_len.encode(enc)?;
        for package in &self.commit_block_packages {
            package.encode(enc)?;
        }

        // Encode raw_prev_header
        let prev_header_len = self.raw_prev_header.len() as u32;
        prev_header_len.encode(enc)?;
        enc.write_buf(&self.raw_prev_header)?;

        // Encode raw_partial_pre_state
        let pre_state_len = self.raw_partial_pre_state.len() as u32;
        pre_state_len.encode(enc)?;
        enc.write_buf(&self.raw_partial_pre_state)?;

        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        // Decode operation_ssz
        let op_len = u32::decode(dec)? as usize;
        let mut operation_ssz = vec![0u8; op_len];
        dec.read_buf(&mut operation_ssz)?;

        // Decode coinputs
        let coinputs_len = u32::decode(dec)? as usize;
        let mut coinputs = Vec::with_capacity(coinputs_len);
        for _ in 0..coinputs_len {
            let coinput_len = u32::decode(dec)? as usize;
            let mut coinput = vec![0u8; coinput_len];
            dec.read_buf(&mut coinput)?;
            coinputs.push(coinput);
        }

        // Decode commit_block_packages
        let packages_len = u32::decode(dec)? as usize;
        let mut commit_block_packages = Vec::with_capacity(packages_len);
        for _ in 0..packages_len {
            let package = CommitBlockPackage::decode(dec)?;
            commit_block_packages.push(package);
        }

        // Decode raw_prev_header
        let prev_header_len = u32::decode(dec)? as usize;
        let mut raw_prev_header = vec![0u8; prev_header_len];
        dec.read_buf(&mut raw_prev_header)?;

        // Decode raw_partial_pre_state
        let pre_state_len = u32::decode(dec)? as usize;
        let mut raw_partial_pre_state = vec![0u8; pre_state_len];
        dec.read_buf(&mut raw_partial_pre_state)?;

        Ok(Self {
            operation_ssz,
            coinputs,
            commit_block_packages,
            raw_prev_header,
            raw_partial_pre_state,
        })
    }
}
