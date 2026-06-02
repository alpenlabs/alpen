//! ASM params test fixtures for L2/CSM components.

use bitcoin::{hashes::Hash, params::Params as BitcoinParams, CompactTarget};
use strata_asm_params::AsmParams;
use strata_btc_verification::L1Anchor;
use strata_primitives::{
    buf::Buf32,
    constants::TIMESTAMPS_FOR_MEDIAN,
    l1::{BtcParams, GenesisL1View, L1BlockCommitment},
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
    let genesis_l1_view = fetch_genesis_l1_view(&segment, 40_320);
    let anchor = L1Anchor {
        block: genesis_l1_view.blk,
        next_target: genesis_l1_view.next_target,
        epoch_start_timestamp: genesis_l1_view.epoch_start_timestamp,
        network: bitcoin::Network::Regtest,
    };
    AsmParams {
        magic: (*b"ALPN").into(),
        anchor,
        subprotocols: Vec::new(),
    }
}

fn fetch_genesis_l1_view(segment: &BtcMainnetSegment, block_height: u32) -> GenesisL1View {
    let btc_params = BtcParams::from(BitcoinParams::from(bitcoin::Network::Bitcoin));
    let interval = btc_params.difficulty_adjustment_interval() as u32;

    let current_epoch_start_height = (block_height / interval) * interval;
    let current_epoch_start_header = segment
        .get_block_header_at(current_epoch_start_height)
        .expect("missing epoch-start header in BTC fixture");

    let block_header = segment
        .get_block_header_at(block_height)
        .expect("missing target header in BTC fixture");

    let timestamps = fetch_block_timestamps_ascending(segment, block_height, TIMESTAMPS_FOR_MEDIAN);
    let timestamps: [u32; TIMESTAMPS_FOR_MEDIAN] = timestamps
        .try_into()
        .expect("timestamp fetch should return TIMESTAMPS_FOR_MEDIAN entries");

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

    GenesisL1View {
        blk: L1BlockCommitment::new(block_height, block_id),
        next_target,
        epoch_start_timestamp: current_epoch_start_header.time,
        last_11_timestamps: timestamps,
    }
}

fn fetch_block_timestamps_ascending(
    segment: &BtcMainnetSegment,
    height: u32,
    count: usize,
) -> Vec<u32> {
    let mut timestamps = Vec::with_capacity(count);

    for i in 0..count {
        let current_height = height.saturating_sub(i as u32);
        if current_height < 1 {
            timestamps.push(0);
        } else {
            let header = segment
                .get_block_header_at(current_height)
                .expect("missing historical header in BTC fixture");
            timestamps.push(header.time);
        }
    }

    timestamps.reverse();
    timestamps
}
