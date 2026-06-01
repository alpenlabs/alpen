//! L2/rollup related test utilities for the Alpen codebase.

use bitcoin::{
    hashes::Hash,
    params::Params as BitcoinParams,
    secp256k1::{SecretKey, SECP256K1},
    Amount, CompactTarget, XOnlyPublicKey,
};
use rand::{rngs::StdRng, SeedableRng};
use strata_crypto::EvenSecretKey;
use strata_params::{CredRule, Params, ProofPublishMode, RollupParams, SyncParams};
use strata_predicate::PredicateKey;
use strata_primitives::{
    buf::Buf32,
    constants::TIMESTAMPS_FOR_MEDIAN,
    l1::{BtcParams, GenesisL1View, L1BlockCommitment},
    L1BlockId,
};
use strata_test_utils_btc::BtcMainnetSegment;

/// Generates consensus [`Params`].
///
/// N.B. Currently, uses the same seed under the hood.
pub fn gen_params() -> Params {
    // TODO: create a random seed if we really need random op_pubkeys every time this is called
    gen_params_with_seed(0)
}

fn gen_params_with_seed(seed: u64) -> Params {
    let opkey = make_dummy_operator_pubkeys_with_seed(seed);
    let segment = BtcMainnetSegment::load();
    let genesis_l1_view = fetch_genesis_l1_view(&segment, 40_320);
    Params {
        rollup: RollupParams {
            magic_bytes: (*b"ALPN").into(),
            block_time: 1000,
            cred_rule: CredRule::Unchecked,
            genesis_l1_view,
            operators: vec![opkey],
            evm_genesis_block_hash:
                "0x37ad61cff1367467a98cf7c54c4ac99e989f1fbb1bc1e646235e90c065c565ba"
                    .parse()
                    .unwrap(),
            evm_genesis_block_state_root:
                "0x351714af72d74259f45cd7eab0b04527cd40e74836a45abcae50f92d919d988f"
                    .parse()
                    .unwrap(),
            l1_reorg_safe_depth: 3,
            target_l2_batch_size: 64,
            deposit_amount: Amount::from_sat(1_000_000_000),
            checkpoint_predicate: PredicateKey::never_accept(),
            dispatch_assignment_dur: 64,
            proof_publish_mode: ProofPublishMode::Strict,
            max_deposits_in_block: 16,
            network: bitcoin::Network::Regtest,
            recovery_delay: 1008,
        },
        run: SyncParams {
            l2_blocks_fetch_limit: 1000,
            l1_follow_distance: 3,
            client_checkpoint_interval: 10,
        },
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

fn make_dummy_operator_pubkeys_with_seed(seed: u64) -> XOnlyPublicKey {
    let mut rng = StdRng::seed_from_u64(seed);
    let sk = SecretKey::new(&mut rng);
    // Ensure the key has even parity for taproot compatibility
    let even_sk = EvenSecretKey::from(sk);
    even_sk.x_only_public_key(SECP256K1).0
}

/// Gets the operator secret key for testing.
/// This matches the key generation in `make_dummy_operator_pubkeys_with_seed(0)`.
pub fn get_test_operator_secret_key() -> SecretKey {
    let mut rng = StdRng::seed_from_u64(0);
    SecretKey::new(&mut rng)
}
