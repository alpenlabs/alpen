use strata_l1_txfmt::{MagicBytes, ParseConfig, TagDataRef};

use crate::constants::{CHECKPOINTING_V0_SUBPROTOCOL_ID, OL_STF_CHECKPOINT_TX_TYPE};

/// Construct the SPS-50 tag for a checkpoint transaction.
///
/// Layout: `[magic][subprotocol_id][tx_type][aux]`. The auxiliary portion is currently empty for
/// checkpointing v0.
// TODO: Move this tag builder into strata-l1tx or strata-common repo
pub fn encode_checkpoint_tag(magic: MagicBytes) -> Vec<u8> {
    let config = ParseConfig::new(magic);
    let tag = TagDataRef::new(
        CHECKPOINTING_V0_SUBPROTOCOL_ID,
        OL_STF_CHECKPOINT_TX_TYPE,
        &[],
    )
    .expect("checkpoint tag encoding");

    config
        .encode_tag_buf(&tag)
        .expect("checkpoint tag encoding")
}
