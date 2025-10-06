//! Sync information formatting implementations

use strata_db::traits::BlockStatus;
use strata_primitives::{
    l1::L1BlockId,
    l2::L2BlockId,
    prelude::{EpochCommitment, L1BlockCommitment, L2BlockCommitment},
};

use super::{helpers::porcelain_field, traits::Formattable};

/// Sync information displayed to the user
#[derive(serde::Serialize)]
pub(crate) struct SyncInfo<'a> {
    pub l1_tip_height: u64,
    pub l1_tip_block_id: &'a L1BlockId,
    pub l2_tip_height: u64,
    pub l2_tip_block_id: &'a L2BlockId,
    pub l2_tip_block_status: &'a BlockStatus,
    pub l2_finalized_block_id: &'a L2BlockId,
    pub current_epoch: u64,
    pub current_slot: u64,
    pub previous_block: &'a L2BlockCommitment,
    pub previous_epoch: &'a EpochCommitment,
    pub finalized_epoch: &'a EpochCommitment,
    pub safe_block: &'a L1BlockCommitment,
}

impl<'a> Formattable for SyncInfo<'a> {
    fn format_porcelain(&self) -> String {
        let mut output = Vec::new();

        // L1 tip information
        output.push(porcelain_field("l1_tip.height", self.l1_tip_height));
        output.push(porcelain_field(
            "l1_tip.block_id",
            format!("{:?}", self.l1_tip_block_id),
        ));

        // L2 tip information
        output.push(porcelain_field("l2_tip.height", self.l2_tip_height));
        output.push(porcelain_field(
            "l2_tip.block_id",
            format!("{:?}", self.l2_tip_block_id),
        ));
        output.push(porcelain_field(
            "l2_tip.block_status",
            format!("{:?}", self.l2_tip_block_status),
        ));

        // Top level state information
        output.push(porcelain_field(
            "top_level_state.current_epoch",
            self.current_epoch,
        ));
        output.push(porcelain_field(
            "top_level_state.current_slot",
            self.current_slot,
        ));

        // Previous block information
        output.push(porcelain_field(
            "syncinfo.top_level_state.prev_block.height",
            self.previous_block.slot(),
        ));
        output.push(porcelain_field(
            "syncinfo.top_level_state.prev_block.blkid",
            format!("{:?}", self.previous_block.blkid()),
        ));

        // Previous epoch information
        output.push(porcelain_field(
            "top_level_state.prev_epoch.epoch",
            format!("{:?}", self.previous_epoch.epoch()),
        ));
        output.push(porcelain_field(
            "top_level_state.prev_epoch.last_slot",
            format!("{:?}", self.previous_epoch.last_slot()),
        ));
        output.push(porcelain_field(
            "top_level_state.prev_epoch.last_block_id",
            format!("{:?}", self.previous_epoch.last_blkid()),
        ));

        // Finalized epoch information
        output.push(porcelain_field(
            "top_level_state.finalized_epoch.epoch",
            format!("{:?}", self.finalized_epoch.epoch()),
        ));
        output.push(porcelain_field(
            "top_level_state.finalized_epoch.last_slot",
            format!("{:?}", self.finalized_epoch.last_slot()),
        ));
        output.push(porcelain_field(
            "top_level_state.finalized_epoch.last_block_id",
            format!("{:?}", self.finalized_epoch.last_blkid()),
        ));

        // L1 safe block information
        output.push(porcelain_field(
            "top_level_state.l1_view.safe_block.height",
            self.safe_block.height(),
        ));
        output.push(porcelain_field(
            "top_level_state.l1_view.safe_block.block_id",
            format!("{:?}", self.safe_block.blkid()),
        ));

        output.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use strata_db::traits::BlockStatus;
    use strata_primitives::{
        buf::Buf32,
        l1::L1BlockId,
        l2::L2BlockId,
        prelude::{EpochCommitment, L1BlockCommitment, L2BlockCommitment},
    };

    use super::*;
    use crate::{cli::OutputFormat, output::helpers::output_to};

    fn create_test_block_id() -> L2BlockId {
        L2BlockId::from(Buf32::from([0x12; 32]))
    }

    fn create_test_l1_block_id() -> L1BlockId {
        L1BlockId::from(Buf32::from([0x34; 32]))
    }

    fn create_test_epoch_commitment(epoch: u64, last_slot: u64) -> EpochCommitment {
        let block_id = create_test_block_id();
        EpochCommitment::new(epoch, last_slot, block_id)
    }

    fn create_test_l2_block_commitment(slot: u64) -> L2BlockCommitment {
        let block_id = create_test_block_id();
        L2BlockCommitment::new(slot, block_id)
    }

    fn create_test_l1_block_commitment(height: u64) -> L1BlockCommitment {
        let block_id = create_test_l1_block_id();
        L1BlockCommitment::from_height_u64(height, block_id).expect("height should be valid")
    }

    #[test]
    fn test_syncinfo_json_format() {
        let l1_tip_block_id = create_test_l1_block_id();
        let l2_tip_block_id = create_test_block_id();
        let l2_finalized_block_id = create_test_block_id();
        let previous_block = create_test_l2_block_commitment(150);
        let previous_epoch = create_test_epoch_commitment(0, 100);
        let finalized_epoch = create_test_epoch_commitment(1, 200);
        let safe_block = create_test_l1_block_commitment(950);

        let sync_info = SyncInfo {
            l1_tip_height: 1000,
            l1_tip_block_id: &l1_tip_block_id,
            l2_tip_height: 175,
            l2_tip_block_id: &l2_tip_block_id,
            l2_tip_block_status: &BlockStatus::Valid,
            l2_finalized_block_id: &l2_finalized_block_id,
            current_epoch: 2,
            current_slot: 175,
            previous_block: &previous_block,
            previous_epoch: &previous_epoch,
            finalized_epoch: &finalized_epoch,
            safe_block: &safe_block,
        };

        let mut buffer = Cursor::new(Vec::new());
        let result = output_to(&sync_info, OutputFormat::Json, &mut buffer);
        assert!(result.is_ok());

        let output = String::from_utf8(buffer.into_inner()).unwrap();

        // Verify JSON structure and content
        assert!(output.contains("\"l1_tip_height\": 1000"));
        assert!(output.contains("\"l2_tip_height\": 175"));
        assert!(output.contains("\"current_epoch\": 2"));
        assert!(output.contains("\"current_slot\": 175"));

        // Verify block IDs are present
        assert!(output.contains("\"l1_tip_block_id\""));
        assert!(output.contains("\"l2_tip_block_id\""));
        assert!(output.contains("\"l2_finalized_block_id\""));

        // Verify status is present
        assert!(output.contains("\"l2_tip_block_status\": \"Valid\""));

        // Verify epoch and block information is present
        assert!(output.contains("\"previous_block\""));
        assert!(output.contains("\"previous_epoch\""));
        assert!(output.contains("\"finalized_epoch\""));
        assert!(output.contains("\"safe_block\""));
    }

    #[test]
    fn test_syncinfo_porcelain_format() {
        let l1_tip_block_id = create_test_l1_block_id();
        let l2_tip_block_id = create_test_block_id();
        let l2_finalized_block_id = create_test_block_id();
        let previous_block = create_test_l2_block_commitment(150);
        let previous_epoch = create_test_epoch_commitment(0, 100);
        let finalized_epoch = create_test_epoch_commitment(1, 200);
        let safe_block = create_test_l1_block_commitment(950);

        let sync_info = SyncInfo {
            l1_tip_height: 1000,
            l1_tip_block_id: &l1_tip_block_id,
            l2_tip_height: 175,
            l2_tip_block_id: &l2_tip_block_id,
            l2_tip_block_status: &BlockStatus::Valid,
            l2_finalized_block_id: &l2_finalized_block_id,
            current_epoch: 2,
            current_slot: 175,
            previous_block: &previous_block,
            previous_epoch: &previous_epoch,
            finalized_epoch: &finalized_epoch,
            safe_block: &safe_block,
        };

        let mut buffer = Cursor::new(Vec::new());
        let result = output_to(&sync_info, OutputFormat::Porcelain, &mut buffer);
        assert!(result.is_ok());

        let output = String::from_utf8(buffer.into_inner()).unwrap();

        // Verify porcelain format structure
        assert!(output.contains("l1_tip.height: 1000"));
        assert!(output.contains("l2_tip.height: 175"));
        assert!(output.contains("top_level_state.current_epoch: 2"));
        assert!(output.contains("top_level_state.current_slot: 175"));

        // Verify block IDs are formatted correctly
        assert!(output.contains("l1_tip.block_id:"));
        assert!(output.contains("l2_tip.block_id:"));
        assert!(output.contains("l2_tip.block_status: Valid"));

        // Verify previous block information
        assert!(output.contains("syncinfo.top_level_state.prev_block.height: 150"));
        assert!(output.contains("syncinfo.top_level_state.prev_block.blkid:"));

        // Verify epoch information
        assert!(output.contains("top_level_state.prev_epoch.epoch:"));
        assert!(output.contains("top_level_state.prev_epoch.last_slot:"));
        assert!(output.contains("top_level_state.prev_epoch.last_block_id:"));
        assert!(output.contains("top_level_state.finalized_epoch.epoch:"));
        assert!(output.contains("top_level_state.finalized_epoch.last_slot:"));
        assert!(output.contains("top_level_state.finalized_epoch.last_block_id:"));

        // Verify safe block information
        assert!(output.contains("top_level_state.l1_view.safe_block.height: 950"));
        assert!(output.contains("top_level_state.l1_view.safe_block.block_id:"));
    }

    #[test]
    fn test_syncinfo_different_block_statuses() {
        let l1_tip_block_id = create_test_l1_block_id();
        let l2_tip_block_id = create_test_block_id();
        let l2_finalized_block_id = create_test_block_id();
        let previous_block = create_test_l2_block_commitment(150);
        let previous_epoch = create_test_epoch_commitment(0, 100);
        let finalized_epoch = create_test_epoch_commitment(1, 200);
        let safe_block = create_test_l1_block_commitment(950);

        // Test with Unchecked status
        let sync_info = SyncInfo {
            l1_tip_height: 1000,
            l1_tip_block_id: &l1_tip_block_id,
            l2_tip_height: 175,
            l2_tip_block_id: &l2_tip_block_id,
            l2_tip_block_status: &BlockStatus::Unchecked,
            l2_finalized_block_id: &l2_finalized_block_id,
            current_epoch: 2,
            current_slot: 175,
            previous_block: &previous_block,
            previous_epoch: &previous_epoch,
            finalized_epoch: &finalized_epoch,
            safe_block: &safe_block,
        };

        let mut buffer = Cursor::new(Vec::new());
        let result = output_to(&sync_info, OutputFormat::Porcelain, &mut buffer);
        assert!(result.is_ok());

        let output = String::from_utf8(buffer.into_inner()).unwrap();
        assert!(output.contains("l2_tip.block_status: Unchecked"));
    }
}
