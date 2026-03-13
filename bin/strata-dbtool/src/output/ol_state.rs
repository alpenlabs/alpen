//! OL state formatting implementations

use strata_identifiers::{Epoch, L1Height, OLBlockId, Slot};
use strata_primitives::{l1::L1BlockId, prelude::EpochCommitment};

use super::{helpers::porcelain_field, traits::Formattable};

/// OL state information displayed to the user.
#[derive(serde::Serialize)]
pub(crate) struct OLStateInfo<'a> {
    pub(crate) block_id: &'a OLBlockId,
    pub(crate) current_slot: Slot,
    pub(crate) current_epoch: Epoch,
    pub(crate) is_epoch_finishing: bool,
    pub(crate) previous_epoch: &'a EpochCommitment,
    pub(crate) finalized_epoch: &'a EpochCommitment,
    pub(crate) l1_next_expected_height: L1Height,
    pub(crate) l1_safe_block_height: L1Height,
    pub(crate) l1_safe_block_blkid: &'a L1BlockId,
}

impl<'a> Formattable for OLStateInfo<'a> {
    fn format_porcelain(&self) -> String {
        let mut output = Vec::new();

        output.push(porcelain_field(
            "olstate.block_id",
            format!("{:?}", self.block_id),
        ));
        output.push(porcelain_field("olstate.current_slot", self.current_slot));
        output.push(porcelain_field("olstate.current_epoch", self.current_epoch));
        output.push(porcelain_field(
            "olstate.is_epoch_finishing",
            if self.is_epoch_finishing {
                "true"
            } else {
                "false"
            },
        ));

        output.push(porcelain_field(
            "olstate.prev_epoch.epoch",
            self.previous_epoch.epoch(),
        ));
        output.push(porcelain_field(
            "olstate.prev_epoch.last_slot",
            self.previous_epoch.last_slot(),
        ));
        output.push(porcelain_field(
            "olstate.prev_epoch.last_blkid",
            format!("{:?}", self.previous_epoch.last_blkid()),
        ));

        output.push(porcelain_field(
            "olstate.finalized_epoch.epoch",
            self.finalized_epoch.epoch(),
        ));
        output.push(porcelain_field(
            "olstate.finalized_epoch.last_slot",
            self.finalized_epoch.last_slot(),
        ));
        output.push(porcelain_field(
            "olstate.finalized_epoch.last_blkid",
            format!("{:?}", self.finalized_epoch.last_blkid()),
        ));

        output.push(porcelain_field(
            "olstate.l1_view.next_expected_height",
            self.l1_next_expected_height,
        ));
        output.push(porcelain_field(
            "olstate.l1_view.safe_block.height",
            self.l1_safe_block_height,
        ));
        output.push(porcelain_field(
            "olstate.l1_view.safe_block.blkid",
            format!("{:?}", self.l1_safe_block_blkid),
        ));

        output.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use strata_identifiers::{Buf32, OLBlockId};
    use strata_primitives::{l1::L1BlockId, prelude::EpochCommitment};

    use super::*;
    use crate::{cli::OutputFormat, output::helpers::output_to};

    fn create_test_epoch_commitment(epoch: u32, last_slot: u64) -> EpochCommitment {
        let block_id = OLBlockId::from(Buf32::from([0x12; 32]));
        EpochCommitment::new(epoch, last_slot, block_id)
    }

    fn create_test_l1_block_id() -> L1BlockId {
        L1BlockId::from(Buf32::from([0x34; 32]))
    }

    #[test]
    fn test_olstate_info_json_format() {
        let previous_epoch = create_test_epoch_commitment(0, 100);
        let finalized_epoch = create_test_epoch_commitment(1, 200);
        let l1_safe_block_blkid = create_test_l1_block_id();

        let olstate_info = OLStateInfo {
            block_id: &OLBlockId::from(Buf32::from([0x12; 32])),
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
        let result = output_to(&olstate_info, OutputFormat::Json, &mut buffer);
        assert!(result.is_ok());

        let output = String::from_utf8(buffer.into_inner()).unwrap();

        assert!(output.contains(
            "\"block_id\": \"1212121212121212121212121212121212121212121212121212121212121212\""
        ));
        assert!(output.contains("\"current_slot\": 175"));
        assert!(output.contains("\"current_epoch\": 2"));
        assert!(output.contains("\"is_epoch_finishing\": true"));
        assert!(output.contains("\"l1_next_expected_height\": 1000"));
        assert!(output.contains("\"l1_safe_block_height\": 950"));
        assert!(output.contains("\"previous_epoch\""));
        assert!(output.contains("\"finalized_epoch\""));
        assert!(output.contains("\"l1_safe_block_blkid\""));
    }

    #[test]
    fn test_olstate_info_porcelain_format() {
        let previous_epoch = create_test_epoch_commitment(0, 100);
        let finalized_epoch = create_test_epoch_commitment(1, 200);
        let l1_safe_block_blkid = create_test_l1_block_id();

        let olstate_info = OLStateInfo {
            block_id: &OLBlockId::from(Buf32::from([0x12; 32])),
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
        let result = output_to(&olstate_info, OutputFormat::Porcelain, &mut buffer);
        assert!(result.is_ok());

        let output = String::from_utf8(buffer.into_inner()).unwrap();

        assert!(output.contains(
            "olstate.block_id: 1212121212121212121212121212121212121212121212121212121212121212"
        ));
        assert!(output.contains("olstate.current_slot: 175"));
        assert!(output.contains("olstate.current_epoch: 2"));
        assert!(output.contains("olstate.is_epoch_finishing: true"));
        assert!(output.contains("olstate.prev_epoch.epoch: 0"));
        assert!(output.contains("olstate.prev_epoch.last_slot: 100"));
        assert!(output.contains("olstate.finalized_epoch.epoch: 1"));
        assert!(output.contains("olstate.finalized_epoch.last_slot: 200"));
        assert!(output.contains("olstate.l1_view.next_expected_height: 1000"));
        assert!(output.contains("olstate.l1_view.safe_block.height: 950"));
        assert!(output.contains("olstate.prev_epoch.last_blkid:"));
        assert!(output.contains("olstate.finalized_epoch.last_blkid:"));
        assert!(output.contains("olstate.l1_view.safe_block.blkid:"));
    }
}
