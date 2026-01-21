//! Types relating to EE block related structures with SSZ support.

use strata_acct_types::Hash;

use crate::{ExecBlockCommitment, ExecBlockPackage, ExecInputs, ExecOutputs};

impl ExecBlockPackage {
    pub fn new(commitment: ExecBlockCommitment, inputs: ExecInputs, outputs: ExecOutputs) -> Self {
        Self {
            commitment,
            inputs,
            outputs,
        }
    }

    pub fn commitment(&self) -> &ExecBlockCommitment {
        &self.commitment
    }

    pub fn exec_blkid(&self) -> Hash {
        self.commitment().exec_blkid()
    }

    pub fn raw_block_encoded_hash(&self) -> Hash {
        self.commitment().raw_block_encoded_hash()
    }

    pub fn inputs(&self) -> &ExecInputs {
        &self.inputs
    }

    pub fn outputs(&self) -> &ExecOutputs {
        &self.outputs
    }
}

impl ExecBlockCommitment {
    pub fn new(exec_blkid: Hash, raw_block_encoded_hash: Hash) -> Self {
        Self {
            exec_blkid: exec_blkid.0.into(),
            raw_block_encoded_hash: raw_block_encoded_hash.0.into(),
        }
    }

    pub fn exec_blkid(&self) -> Hash {
        let mut result = [0u8; 32];
        result.copy_from_slice(self.exec_blkid.as_ref());
        Hash::new(result)
    }

    pub fn raw_block_encoded_hash(&self) -> Hash {
        let mut result = [0u8; 32];
        result.copy_from_slice(self.raw_block_encoded_hash.as_ref());
        Hash::new(result)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use strata_test_utils_ssz::ssz_proptest;

    use super::*;

    mod exec_block_package {
        use super::*;

        #[test]
        fn test_new() {
            let commitment = ExecBlockCommitment::new(Hash::new([0xff; 32]), Hash::new([0x11; 32]));
            let inputs = ExecInputs::new_empty();
            let outputs = ExecOutputs::new_empty();

            let block = ExecBlockPackage::new(commitment, inputs, outputs);

            assert_eq!(block.exec_blkid(), Hash::new([0xff; 32]));
            assert_eq!(block.raw_block_encoded_hash(), Hash::new([0x11; 32]));
            assert_eq!(block.inputs().total_inputs(), 0);
        }
    }
}
