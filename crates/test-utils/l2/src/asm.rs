//! ASM params test fixtures for L2/CSM components.

use bitcoin::{hashes::Hash, params::Params as BitcoinParams, CompactTarget};
use strata_asm_params::AsmParams;
use strata_btc_verification::L1Anchor;
use strata_primitives::{
    buf::Buf32,
    l1::{BtcParams, L1BlockCommitment},
    L1BlockId,
};
use strata_test_utils_btc::BtcMainnetSegment;

/// Generates a minimal [`AsmParams`] for CSM/L2 tests.
///
/// Mirrors the genesis L1 anchor and magic bytes the legacy params fixture
/// produced. Subprotocols are left empty since current consumers only read the
/// magic bytes and the genesis anchor block.
// TODO: populate `subprotocols` (bridge/checkpoint) if a test needs them.
pub fn gen_asm_params() -> AsmParams {
    let segment = BtcMainnetSegment::load();
    let anchor = compute_l1_anchor(&segment, 40_320);
    AsmParams {
        magic: (*b"ALPN").into(),
        anchor,
        subprotocols: Vec::new(),
    }
}

fn compute_l1_anchor(segment: &BtcMainnetSegment, block_height: u32) -> L1Anchor {
    let btc_params = BtcParams::from(BitcoinParams::from(bitcoin::Network::Bitcoin));
    let interval = btc_params.difficulty_adjustment_interval() as u32;

    let current_epoch_start_height = (block_height / interval) * interval;
    let current_epoch_start_header = segment
        .get_block_header_at(current_epoch_start_height)
        .expect("missing epoch-start header in BTC fixture");

    let block_header = segment
        .get_block_header_at(block_height)
        .expect("missing target header in BTC fixture");

    let block_id = L1BlockId::from(Buf32::from(
        block_header.block_hash().as_raw_hash().to_byte_array(),
    ));

    let next_target =
        if (block_height as u64 + 1).is_multiple_of(btc_params.difficulty_adjustment_interval()) {
            CompactTarget::from_next_work_required(
                block_header.bits,
                (block_header.time - current_epoch_start_header.time) as u64,
                &btc_params,
            )
            .to_consensus()
        } else {
            block_header.target().to_compact_lossy().to_consensus()
        };

    L1Anchor {
        block: L1BlockCommitment::new(block_height, block_id),
        next_target,
        epoch_start_timestamp: current_epoch_start_header.time,
        network: bitcoin::Network::Regtest,
    }
}
