//! L1 segment types.

#[derive(Clone, Debug)]
pub struct AsmManifest {
    /// Bitcoin block header.
    // TODO figure out what to do here
    header_buf: [u8; 80],

    /// Merkle root of the witness structure, so we can make proofs against it.
    wtxid_root: [u8; 32],

    /// Logs produced by the ASM as of this block.
    logs: Vec<AsmLog>,
}

impl AsmManifest {
    pub fn header_buf(&self) -> [u8; 80] {
        self.header_buf
    }

    pub fn wtxid_root(&self) -> [u8; 32] {
        self.wtxid_root
    }

    pub fn logs(&self) -> &[AsmLog] {
        &self.logs
    }
}

#[derive(Clone, Debug)]
pub struct AsmLog {
    payload: Vec<u8>,
}

impl AsmLog {
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
}
