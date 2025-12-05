//! Chainstate formatting implementations

use strata_primitives::{l1::L1BlockId, l2::L2BlockId, prelude::EpochCommitment};

use super::{helpers::porcelain_field, traits::Formattable};

/// Chainstate information displayed to the user
#[derive(serde::Serialize)]
pub(crate) struct ChainstateInfo<'a> {
    pub block_id: &'a L2BlockId,
    pub current_slot: u64,
    pub current_epoch: u64,
    pub is_epoch_finishing: bool,
    pub previous_epoch: &'a EpochCommitment,
    pub finalized_epoch: &'a EpochCommitment,
    pub l1_next_expected_height: u64,
    pub l1_safe_block_height: u64,
    pub l1_safe_block_blkid: &'a L1BlockId,
}

impl<'a> Formattable for ChainstateInfo<'a> {
    fn format_porcelain(&self) -> String {
        let mut output = Vec::new();

        output.push(porcelain_field(
            "chainstate.block_id",
            format!("{:?}", self.block_id),
        ));
        output.push(porcelain_field(
            "chainstate.current_slot",
            self.current_slot,
        ));
        output.push(porcelain_field(
            "chainstate.current_epoch",
            self.current_epoch,
        ));
        output.push(porcelain_field(
            "chainstate.is_epoch_finishing",
            if self.is_epoch_finishing {
                "true"
            } else {
                "false"
            },
        ));

        // Format previous epoch
        output.push(porcelain_field(
            "chainstate.prev_epoch.epoch",
            self.previous_epoch.epoch(),
        ));
        output.push(porcelain_field(
            "chainstate.prev_epoch.last_slot",
            self.previous_epoch.last_slot(),
        ));
        output.push(porcelain_field(
            "chainstate.prev_epoch.last_blkid",
            format!("{:?}", self.previous_epoch.last_blkid()),
        ));

        // Format finalized epoch
        output.push(porcelain_field(
            "chainstate.finalized_epoch.epoch",
            self.finalized_epoch.epoch(),
        ));
        output.push(porcelain_field(
            "chainstate.finalized_epoch.last_slot",
            self.finalized_epoch.last_slot(),
        ));
        output.push(porcelain_field(
            "chainstate.finalized_epoch.last_blkid",
            format!("{:?}", self.finalized_epoch.last_blkid()),
        ));

        // Format L1 view
        output.push(porcelain_field(
            "chainstate.l1_view.next_expected_height",
            self.l1_next_expected_height,
        ));
        output.push(porcelain_field(
            "chainstate.l1_view.safe_block.height",
            self.l1_safe_block_height,
        ));
        output.push(porcelain_field(
            "chainstate.l1_view.safe_block.blkid",
            format!("{:?}", self.l1_safe_block_blkid),
        ));

        output.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use strata_primitives::{
        buf::Buf32,
        l1::L1BlockId,
        prelude::{EpochCommitment, L2BlockId},
    };

    use super::*;
    use crate::{cli::OutputFormat, output::helpers::output_to};

    fn create_test_epoch_commitment(epoch: u32, last_slot: u64) -> EpochCommitment {
        let block_id = L2BlockId::from(Buf32::from([0x12; 32]));
        EpochCommitment::new(epoch, last_slot, block_id)
    }

    fn create_test_l1_block_id() -> L1BlockId {
        L1BlockId::from(Buf32::from([0x34; 32]))
    }

    #[test]
    fn test_chainstate_info_json_format() {
        let previous_epoch = create_test_epoch_commitment(0, 100);
        let finalized_epoch = create_test_epoch_commitment(1, 200);
        let l1_safe_block_blkid = create_test_l1_block_id();

        let chainstate_info = ChainstateInfo {
            block_id: &L2BlockId::from(Buf32::from([0x12; 32])),
            current_slot: 175,
            current_epoch: 2,
            is_epoch_finishing: true,
            previous_epoch: &previous_epoch,
            finalized_epoch: &finalized_epoch,
            l1_next_expected_height: 1000,
            l1_safe_block_height: 950,
            l1_safe_block_blkid: &l1_safe_block_blkid,
        };

        let mut buffer = Cursor::new(Vec::new());
        let result = output_to(&chainstate_info, OutputFormat::Json, &mut buffer);
        assert!(result.is_ok());

        let output = String::from_utf8(buffer.into_inner()).unwrap();

        // Verify JSON structure and content
        assert!(output.contains(
            "\"block_id\": \"1212121212121212121212121212121212121212121212121212121212121212\""
        ));
        assert!(output.contains("\"current_slot\": 175"));
        assert!(output.contains("\"current_epoch\": 2"));
        assert!(output.contains("\"is_epoch_finishing\": true"));
        assert!(output.contains("\"l1_next_expected_height\": 1000"));
        assert!(output.contains("\"l1_safe_block_height\": 950"));

        // Verify epoch information is present
        assert!(output.contains("\"previous_epoch\""));
        assert!(output.contains("\"finalized_epoch\""));
        assert!(output.contains("\"l1_safe_block_blkid\""));
    }

    #[test]
    fn test_chainstate_info_porcelain_format() {
        let previous_epoch = create_test_epoch_commitment(0, 100);
        let finalized_epoch = create_test_epoch_commitment(1, 200);
        let l1_safe_block_blkid = create_test_l1_block_id();

        let chainstate_info = ChainstateInfo {
            block_id: &L2BlockId::from(Buf32::from([0x12; 32])),
            current_slot: 175,
            current_epoch: 2,
            is_epoch_finishing: true,
            previous_epoch: &previous_epoch,
            finalized_epoch: &finalized_epoch,
            l1_next_expected_height: 1000,
            l1_safe_block_height: 950,
            l1_safe_block_blkid: &l1_safe_block_blkid,
        };

        let mut buffer = Cursor::new(Vec::new());
        let result = output_to(&chainstate_info, OutputFormat::Porcelain, &mut buffer);
        assert!(result.is_ok());

        let output = String::from_utf8(buffer.into_inner()).unwrap();

        // Verify porcelain format structure
        assert!(output.contains(
            "chainstate.block_id: 1212121212121212121212121212121212121212121212121212121212121212"
        ));
        assert!(output.contains("chainstate.current_slot: 175"));
        assert!(output.contains("chainstate.current_epoch: 2"));
        assert!(output.contains("chainstate.is_epoch_finishing: true"));
        assert!(output.contains("chainstate.prev_epoch.epoch: 0"));
        assert!(output.contains("chainstate.prev_epoch.last_slot: 100"));
        assert!(output.contains("chainstate.finalized_epoch.epoch: 1"));
        assert!(output.contains("chainstate.finalized_epoch.last_slot: 200"));
        assert!(output.contains("chainstate.l1_view.next_expected_height: 1000"));
        assert!(output.contains("chainstate.l1_view.safe_block.height: 950"));

        // Verify block IDs are formatted correctly
        assert!(output.contains("chainstate.prev_epoch.last_blkid:"));
        assert!(output.contains("chainstate.finalized_epoch.last_blkid:"));
        assert!(output.contains("chainstate.l1_view.safe_block.blkid:"));
    }
}
