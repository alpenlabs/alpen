//! L2/rollup related test utilities for the Alpen codebase.

use std::time::{SystemTime, UNIX_EPOCH};

use bitcoin::{
    secp256k1::{SecretKey, SECP256K1},
    Amount,
};
use borsh::to_vec;
use rand::{rngs::StdRng, SeedableRng};
use strata_bridge_types::OperatorPubkeys;
use strata_checkpoint_types::{Checkpoint, CheckpointSidecar, SignedCheckpoint};
use strata_consensus_logic::genesis::make_l2_genesis;
use strata_crypto::EvenSecretKey;
use strata_ol_chain_types::{
    L2Block, L2BlockAccessory, L2BlockBody, L2BlockBundle, L2BlockHeader, L2Header,
    SignedL2BlockHeader,
};
use strata_ol_chainstate_types::Chainstate;
use strata_params::{OperatorConfig, Params, ProofPublishMode, RollupParams, SyncParams};
use strata_predicate::PredicateKey;
use strata_primitives::{block_credential, buf::Buf64};
use strata_test_utils::ArbitraryGenerator;
use strata_test_utils_btc::segment::BtcChainSegment;

/// Generates a sequence of L2 block bundles starting from an optional parent block.
///
/// # Arguments
///
/// * `parent` - An optional [`SignedL2BlockHeader`] representing the parent block to build upon. If
///   `None`, the genesis or default starting point is assumed.
/// * `blocks_num` - The number of L2 blocks to generate.
///
/// # Returns
///
/// A vector containing [`L2BlockBundle`] instances forming the generated L2 chain.
pub fn gen_l2_chain(parent: Option<SignedL2BlockHeader>, blocks_num: usize) -> Vec<L2BlockBundle> {
    let mut blocks = Vec::new();
    let mut parent = match parent {
        Some(p) => p,
        None => {
            let p = gen_block(None);
            blocks.push(p.clone());
            p.header().clone()
        }
    };

    for _ in 0..blocks_num {
        let block = gen_block(Some(&parent));
        blocks.push(block.clone());
        parent = block.header().clone()
    }

    blocks
}

fn gen_block(parent: Option<&SignedL2BlockHeader>) -> L2BlockBundle {
    let mut arb = ArbitraryGenerator::new_with_size(1 << 12);
    let header: L2BlockHeader = arb.generate();
    let body: L2BlockBody = arb.generate();
    let accessory: L2BlockAccessory = arb.generate();

    let current_timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;

    let block_idx = parent.map(|h| h.slot() + 1).unwrap_or(0);
    let prev_block = parent.map(|h| h.get_blockid()).unwrap_or_default();
    let timestamp = parent
        .map(|h| h.timestamp() + 100)
        .unwrap_or(current_timestamp);

    let header = L2BlockHeader::new(
        block_idx,
        parent.map(|h| h.epoch()).unwrap_or(0),
        timestamp,
        prev_block,
        &body,
        *header.state_root(),
    );
    let empty_sig = Buf64::zero();
    let signed_header = SignedL2BlockHeader::new(header, empty_sig);
    let block = L2Block::new(signed_header, body);
    L2BlockBundle::new(block, accessory)
}

/// Generates consensus [`Params`].
///
/// N.B. Currently, uses the same seed under the hood.
pub fn gen_params() -> Params {
    // TODO: create a random seed if we really need random op_pubkeys every time this is called
    gen_params_with_seed(0)
}

fn gen_params_with_seed(seed: u64) -> Params {
    let opkeys = make_dummy_operator_pubkeys_with_seed(seed);
    let genesis_l1_view = BtcChainSegment::load()
        .fetch_genesis_l1_view(40320)
        .unwrap();
    Params {
        rollup: RollupParams {
            magic_bytes: (*b"ALPN").into(),
            block_time: 1000,
            cred_rule: block_credential::CredRule::Unchecked,
            genesis_l1_view,
            operator_config: OperatorConfig::Static(vec![opkeys]),
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
        },
        run: SyncParams {
            l2_blocks_fetch_limit: 1000,
            l1_follow_distance: 3,
            client_checkpoint_interval: 10,
        },
    }
}

fn make_dummy_operator_pubkeys_with_seed(seed: u64) -> OperatorPubkeys {
    let mut rng = StdRng::seed_from_u64(seed);
    let sk = SecretKey::new(&mut rng);
    // Ensure the key has even parity for taproot compatibility
    let even_sk = EvenSecretKey::from(sk);
    let x_only_public_key = even_sk.x_only_public_key(SECP256K1);
    let (pk, _parity) = x_only_public_key;
    OperatorPubkeys::new(pk.into(), pk.into())
}

/// Gets the operator secret key for testing.
/// This matches the key generation in `make_dummy_operator_pubkeys_with_seed(0)`.
pub fn get_test_operator_secret_key() -> SecretKey {
    let mut rng = StdRng::seed_from_u64(0);
    SecretKey::new(&mut rng)
}

/// Gets the genesis [`Chainstate`] from consensus [`Params`] and test btc segment.
pub fn get_genesis_chainstate(params: &Params) -> (L2BlockBundle, Chainstate) {
    make_l2_genesis(params)
}

/// Generates random valid [`SignedCheckpoint`].
pub fn get_test_signed_checkpoint() -> SignedCheckpoint {
    let chstate: Chainstate = ArbitraryGenerator::new_with_size(1 << 12).generate();
    SignedCheckpoint::new(
        Checkpoint::new(
            ArbitraryGenerator::new().generate(),
            ArbitraryGenerator::new().generate(),
            ArbitraryGenerator::new().generate(),
            CheckpointSidecar::new(to_vec(&chstate).unwrap()),
        ),
        ArbitraryGenerator::new().generate(),
    )
}
