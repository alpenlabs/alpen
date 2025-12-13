//! Interpretation of extra data.

use strata_acct_types::Hash;

use crate::ssz_generated::ssz::extra_data::UpdateExtraData;

// UpdateExtraData is now generated from ssz/extra_data.ssz

impl UpdateExtraData {
    pub fn new(new_tip_blkid: Hash, processed_inputs: u32, processed_fincls: u32) -> Self {
        Self {
            new_tip_blkid: new_tip_blkid.into(),
            processed_inputs,
            processed_fincls,
        }
    }

    pub fn new_tip_blkid(&self) -> &Hash {
        self.new_tip_blkid
            .as_ref()
            .try_into()
            .expect("FixedBytes<32> should convert to &[u8; 32]")
    }

    pub fn processed_inputs(&self) -> &u32 {
        &self.processed_inputs
    }

    pub fn processed_fincls(&self) -> &u32 {
        &self.processed_fincls
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use strata_test_utils_ssz::ssz_proptest;

    use crate::ssz_generated::ssz::extra_data::UpdateExtraData;

    ssz_proptest!(
        UpdateExtraData,
        (any::<[u8; 32]>(), any::<u32>(), any::<u32>()).prop_map(|(blkid, inputs, fincls)| {
            UpdateExtraData {
                new_tip_blkid: blkid.into(),
                processed_inputs: inputs,
                processed_fincls: fincls,
            }
        })
    );
}
