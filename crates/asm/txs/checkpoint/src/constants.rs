use std::sync::LazyLock;

use strata_asm_common::SubprotocolId;
use strata_l1_txfmt::TagData;

/// Subprotocol identifier assigned to checkpoint transactions.
pub const CHECKPOINT_SUBPROTOCOL_ID: SubprotocolId = 1;

/// Transaction type identifier for OL STF checkpoints.
pub const OL_STF_CHECKPOINT_TX_TYPE: u8 = 1;

/// Tag data for OL STF checkpoint transactions.
pub static OL_STF_CHECKPOINT_TX_TAG: LazyLock<TagData> = LazyLock::new(|| {
    TagData::new(CHECKPOINT_SUBPROTOCOL_ID, OL_STF_CHECKPOINT_TX_TYPE, vec![])
        .expect("valid checkpoint tag data")
});
