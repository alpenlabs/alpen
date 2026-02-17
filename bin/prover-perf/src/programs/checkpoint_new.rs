use strata_acct_types::BitcoinAmount;
use strata_identifiers::{Buf32, Buf64, L1BlockId, WtxidsRoot};
use strata_ledger_types::{AccountTypeState, IStateAccessor, NewAccountData};
use strata_ol_chain_types_new::{
    GamTxPayload, OLBlock, OLL1ManifestContainer, OLTransaction, SignedOLBlockHeader,
    TransactionAttachment, TransactionPayload,
};
use strata_ol_state_types::OLSnarkAccountState;
use strata_ol_stf::{
    test_utils::{
        SnarkUpdateBuilder, create_test_genesis_state, execute_block,
        get_snark_state_expect, get_test_snark_account_id, get_test_state_root, test_account_id,
    },
    BlockComponents, BlockInfo,
};
use strata_predicate::PredicateKey;
use strata_proofimpl_checkpoint_new::program::{CheckpointProgram, CheckpointProverInput};
use tracing::info;
use zkaleido::{PerformanceReport, ZkVmHostPerf, ZkVmProgramPerf};

const SLOTS_PER_EPOCH: u64 = 100;
const NUM_BLOCKS: usize = 10;
const SNARK_INITIAL_BALANCE: u64 = 100_000_000;

fn create_snark_account(state: &mut strata_ol_state_types::OLState) {
    let snark_id = get_test_snark_account_id();
    let update_vk = PredicateKey::always_accept();
    let initial_state_root = get_test_state_root(1);
    let snark_state = OLSnarkAccountState::new_fresh(update_vk, initial_state_root);
    let balance = BitcoinAmount::from_sat(SNARK_INITIAL_BALANCE);
    let new_acct_data = NewAccountData::new(balance, AccountTypeState::Snark(snark_state));
    state
        .create_new_account(snark_id, new_acct_data)
        .expect("should create snark account");
}

fn build_chain_with_transactions(
    state: &mut strata_ol_state_types::OLState,
    num_blocks: usize,
    slots_per_epoch: u64,
) -> Vec<strata_ol_stf::CompletedBlock> {
    use strata_asm_common::AsmManifest;

    let mut blocks = Vec::with_capacity(num_blocks);

    let gam_target = test_account_id(1);
    let snark_id = get_test_snark_account_id();

    // Create snark account before genesis
    create_snark_account(state);

    // Terminal genesis (with manifest) so epoch advances from 0 to 1
    let genesis_manifest = AsmManifest::new(
        0,
        L1BlockId::from(Buf32::from([0u8; 32])),
        WtxidsRoot::from(Buf32::from([0u8; 32])),
        vec![],
    );
    let genesis_info = BlockInfo::new_genesis(1_000_000);
    let genesis_components = BlockComponents::new_manifests(vec![genesis_manifest]);
    let genesis =
        execute_block(state, &genesis_info, None, genesis_components).expect("genesis should work");
    blocks.push(genesis);

    // Track state root variant for snark updates
    let mut state_root_counter: u8 = 2;

    // Subsequent blocks: cycle through GAM, SnarkAccountUpdate, and empty blocks
    for i in 1..num_blocks {
        let slot = i as u64;
        let epoch = ((slot - 1) / slots_per_epoch + 1) as u32;
        let parent = blocks[i - 1].header();
        let timestamp = 1_000_000 + (i as u64 * 1000);
        let block_info = BlockInfo::new(timestamp, slot, epoch);

        let is_terminal = slot.is_multiple_of(slots_per_epoch);

        let components = if is_terminal {
            let dummy_manifest = AsmManifest::new(
                0,
                L1BlockId::from(Buf32::from([0u8; 32])),
                WtxidsRoot::from(Buf32::from([0u8; 32])),
                vec![],
            );
            let tx = TransactionPayload::GenericAccountMessage(
                GamTxPayload::new(gam_target, format!("terminal block {i}").into_bytes())
                    .expect("GamTxPayload creation should succeed"),
            );
            BlockComponents::new(
                strata_ol_chain_types_new::OLTxSegment::new(vec![OLTransaction::new(
                    tx,
                    TransactionAttachment::default(),
                )])
                .expect("tx segment should be within limits"),
                Some(
                    OLL1ManifestContainer::new(vec![dummy_manifest])
                        .expect("single manifest should succeed"),
                ),
            )
        } else if i % 3 == 0 {
            // SnarkAccountUpdate transaction
            let (_, snark_state) = get_snark_state_expect(state, snark_id);
            let builder = SnarkUpdateBuilder::from_snark_state(snark_state.clone());
            let new_state_root = get_test_state_root(state_root_counter);
            state_root_counter = state_root_counter.wrapping_add(1);
            let tx = builder.build(snark_id, new_state_root, vec![0u8; 32]);
            BlockComponents::new_txs(vec![tx])
        } else if i % 3 == 1 {
            // GAM transaction
            let tx = TransactionPayload::GenericAccountMessage(
                GamTxPayload::new(gam_target, format!("message at slot {i}").into_bytes())
                    .expect("GamTxPayload creation should succeed"),
            );
            BlockComponents::new_txs(vec![tx])
        } else {
            // Empty block
            BlockComponents::new_empty()
        };

        let block = execute_block(state, &block_info, Some(parent), components)
            .expect("block execution should succeed");
        blocks.push(block);
    }

    blocks
}

pub(crate) fn prepare_input() -> CheckpointProverInput {
    info!("Preparing input for Checkpoint New");

    // Build a chain with transactions to get realistic cycle counts
    let mut state = create_test_genesis_state();
    let mut blocks = build_chain_with_transactions(&mut state, NUM_BLOCKS, SLOTS_PER_EPOCH);

    // First block is the parent (genesis); remaining blocks are the proving batch
    let parent = blocks.remove(0).into_header();

    // Rebuild start_state: execute just the genesis block to get state after genesis
    let mut start_state = create_test_genesis_state();
    let _ = build_chain_with_transactions(&mut start_state, 1, SLOTS_PER_EPOCH);

    // Wrap completed blocks into OLBlock (with dummy signatures)
    let blocks = blocks
        .into_iter()
        .map(|b| {
            OLBlock::new(
                SignedOLBlockHeader::new(b.header().clone(), Buf64::zero()),
                b.body().clone(),
            )
        })
        .collect();

    CheckpointProverInput {
        start_state,
        blocks,
        parent,
    }
}

pub(crate) fn gen_perf_report(host: &impl ZkVmHostPerf) -> PerformanceReport {
    info!("Generating performance report for Checkpoint New");
    let input = prepare_input();
    CheckpointProgram::perf_report(&input, host).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_checkpoint_new_native_execution() {
        let input = prepare_input();
        let output = CheckpointProgram::execute(&input).unwrap();
        dbg!(output);
    }
}
