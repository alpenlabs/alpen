//! Types relating to EE block related structures.

type Hash = [u8; 32];

/// Container for an execution block that signals additional data with it.
// TODO better name, using an intentionally bad one for now
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecBlockNotpackage {
    /// Execution blkid, which commits to things more cleanly.
    exec_blkid: Hash,

    /// Encoded block hash to more cleanly link things.
    block_raw_hash: Hash,

    /// Inputs processed in the block.
    inputs: BlockInputs,

    /// Outputs produced in the block.
    outputs: BlockOutputs,
}

impl ExecBlockNotpackage {
    pub fn exec_blkid(&self) -> [u8; 32] {
        self.exec_blkid
    }

    pub fn block_raw_hash(&self) -> [u8; 32] {
        self.block_raw_hash
    }

    pub fn inputs(&self) -> &BlockInputs {
        &self.inputs
    }

    pub fn outputs(&self) -> &BlockOutputs {
        &self.outputs
    }
}

/// Inputs from the OL to the EE processed in a single EE block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockInputs {
    // TODO messages
}

/// Outputs from an EE to the OL produced in a single EE block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockOutputs {
    // TODO messages
}
