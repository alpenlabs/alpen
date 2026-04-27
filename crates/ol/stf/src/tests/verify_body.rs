//! Body commitment and round-trip verification tests for the OL STF implementation.

use strata_acct_types::{AccountId, BitcoinAmount, MAX_MESSAGES, TxEffects};
use strata_asm_common::AsmManifest;
use strata_identifiers::{Buf32, L1BlockId, WtxidsRoot};
use strata_ledger_types::{IAccountState, ISnarkAccountState, IStateAccessor};
use strata_ol_chain_types_new::{
    MAX_LOGS_PER_BLOCK, OLL1ManifestContainer, OLTransaction, OLTxSegment,
};

use crate::{
    BRIDGE_GATEWAY_ACCT_ID, assembly::BlockComponents, context::BlockInfo, errors::ExecError,
    test_utils::*,
};

const WITHDRAWAL_LOG_AMOUNT: u64 = 100_000_000;

// ===== ROUND-TRIP VERIFICATION TESTS =====

#[test]
fn test_verify_valid_block_succeeds() {
    let mut state = create_test_genesis_state();

    let genesis_info = BlockInfo::new_genesis(1000000);
    let genesis = execute_block(&mut state, &genesis_info, None, genesis_block_components())
        .expect("Genesis block assembly should succeed");

    let mut verify_state = create_test_genesis_state();
    assert_verification_succeeds(&mut verify_state, genesis.header(), None, genesis.body());
}

#[test]
fn test_assemble_then_verify_roundtrip() {
    let mut state = create_test_genesis_state();

    let genesis_info = BlockInfo::new_genesis(1000000);
    let genesis = execute_block(&mut state, &genesis_info, None, genesis_block_components())
        .expect("test: Genesis block assembly should succeed");

    let block1_info = BlockInfo::new(1001000, 1, 1);
    let block1 = execute_block(
        &mut state,
        &block1_info,
        Some(genesis.header()),
        BlockComponents::new_empty(),
    )
    .expect("test: Block 1 assembly should succeed");

    let block2_info = BlockInfo::new(1002000, 2, 1);
    let block2 = execute_block(
        &mut state,
        &block2_info,
        Some(block1.header()),
        BlockComponents::new_empty(),
    )
    .expect("test: Block 2 assembly should succeed");

    let mut verify_state = create_test_genesis_state();

    assert_verification_succeeds(&mut verify_state, genesis.header(), None, genesis.body());
    assert_verification_succeeds(
        &mut verify_state,
        block1.header(),
        Some(genesis.header().clone()),
        block1.body(),
    );
    assert_verification_succeeds(
        &mut verify_state,
        block2.header(),
        Some(block1.header().clone()),
        block2.body(),
    );
}

#[test]
fn test_multi_block_chain_verification() {
    let mut state = create_test_genesis_state();
    const SLOTS_PER_EPOCH: u64 = 10;

    let mut blocks = Vec::new();
    let mut headers = Vec::new();

    for i in 0..15 {
        let slot = i as u64;
        let epoch = if i == 0 {
            0
        } else {
            ((slot - 1) / SLOTS_PER_EPOCH + 1) as u32
        };
        let timestamp = 1000000 + (i as u64 * 1000);
        let block_info = if i == 0 {
            BlockInfo::new_genesis(timestamp)
        } else {
            BlockInfo::new(timestamp, slot, epoch)
        };

        let parent_header = if i == 0 { None } else { Some(&headers[i - 1]) };

        let is_terminal = if i == 0 {
            true
        } else {
            slot.is_multiple_of(SLOTS_PER_EPOCH)
        };

        let components = if is_terminal {
            let dummy_manifest = AsmManifest::new(
                state.last_l1_height() + 1,
                L1BlockId::from(Buf32::from([0u8; 32])),
                WtxidsRoot::from(Buf32::from([0u8; 32])),
                vec![],
            )
            .expect("test manifest should be valid");
            BlockComponents::new_manifests(vec![dummy_manifest])
        } else {
            BlockComponents::new_empty()
        };

        let Ok(block) = execute_block(&mut state, &block_info, parent_header, components) else {
            panic!("test: block {i} assembly should succeed");
        };

        headers.push(block.header().clone());
        blocks.push(block);
    }

    let mut verify_state = create_test_genesis_state();

    for (i, block) in blocks.iter().enumerate() {
        let parent_header = if i == 0 {
            None
        } else {
            Some(headers[i - 1].clone())
        };

        assert_verification_succeeds(
            &mut verify_state,
            block.header(),
            parent_header,
            block.body(),
        );
    }

    assert_state_updated(&mut verify_state, 2, 14);
}

#[test]
fn test_verify_block_with_transactions() {
    let mut state = create_test_genesis_state();

    let target = test_account_id(1);
    let gam_tx = make_gam_tx(target);

    let dummy_manifest = AsmManifest::new(
        1,
        L1BlockId::from(Buf32::from([0u8; 32])),
        WtxidsRoot::from(Buf32::from([0u8; 32])),
        vec![],
    )
    .expect("test manifest should be valid");
    let genesis_components = BlockComponents::new(
        OLTxSegment::new(vec![gam_tx]).expect("tx segment should be within limits"),
        Some(
            OLL1ManifestContainer::new(vec![dummy_manifest])
                .expect("single manifest should succeed"),
        ),
    );

    let genesis_info = BlockInfo::new_genesis(1000000);
    let genesis = execute_block(&mut state, &genesis_info, None, genesis_components)
        .expect("Genesis with tx should assemble");

    let mut verify_state = create_test_genesis_state();
    assert_verification_succeeds(&mut verify_state, genesis.header(), None, genesis.body());

    assert_eq!(
        genesis
            .body()
            .tx_segment()
            .expect("genesis should have tx_segment")
            .txs()
            .len(),
        1
    );
}

// ===== BODY/STATE/LOG COMMITMENT VALIDATION TESTS =====

#[test]
fn test_verify_rejects_mismatched_state_root() {
    let mut state = create_test_genesis_state();

    let genesis_info = BlockInfo::new_genesis(1000000);
    let genesis = execute_block(&mut state, &genesis_info, None, genesis_block_components())
        .expect("Genesis assembly should succeed");

    let wrong_root = Buf32::from([99u8; 32]);
    let tampered_header = tamper_state_root(genesis.header(), wrong_root);

    let mut positive_verify_state = create_test_genesis_state();
    assert_verification_succeeds(
        &mut positive_verify_state,
        genesis.header(),
        None,
        genesis.body(),
    );

    let mut verify_state = create_test_genesis_state();
    assert_verification_fails_with(
        &mut verify_state,
        &tampered_header,
        None,
        genesis.body(),
        |e| matches!(e, ExecError::ChainIntegrity),
    );
}

#[test]
fn test_verify_rejects_mismatched_logs_root() {
    let mut state = create_test_genesis_state();

    let target = test_account_id(1);
    let gam_tx = make_gam_tx(target);

    let dummy_manifest = AsmManifest::new(
        1,
        L1BlockId::from(Buf32::from([0u8; 32])),
        WtxidsRoot::from(Buf32::from([0u8; 32])),
        vec![],
    )
    .expect("test manifest should be valid");
    let genesis_components = BlockComponents::new(
        OLTxSegment::new(vec![gam_tx]).expect("tx segment should be within limits"),
        Some(
            OLL1ManifestContainer::new(vec![dummy_manifest])
                .expect("single manifest should succeed"),
        ),
    );

    let genesis_info = BlockInfo::new_genesis(1000000);
    let genesis = execute_block(&mut state, &genesis_info, None, genesis_components)
        .expect("Genesis assembly should succeed");

    let wrong_root = Buf32::from([88u8; 32]);
    let tampered_header = tamper_logs_root(genesis.header(), wrong_root);

    let mut positive_verify_state = create_test_genesis_state();
    assert_verification_succeeds(
        &mut positive_verify_state,
        genesis.header(),
        None,
        genesis.body(),
    );

    let mut verify_state = create_test_genesis_state();
    assert_verification_fails_with(
        &mut verify_state,
        &tampered_header,
        None,
        genesis.body(),
        |e| matches!(e, ExecError::ChainIntegrity),
    );
}

#[test]
fn test_verify_empty_block_logs_root() {
    let mut state = create_test_genesis_state();

    let genesis_info = BlockInfo::new_genesis(1000000);
    let genesis = execute_block(&mut state, &genesis_info, None, genesis_block_components())
        .expect("Genesis assembly should succeed");

    assert_eq!(
        *genesis.header().logs_root(),
        Buf32::zero(),
        "Empty block should have zero logs root"
    );

    let mut verify_state = create_test_genesis_state();
    assert_verification_succeeds(&mut verify_state, genesis.header(), None, genesis.body());
}

#[test]
fn test_verify_block_allows_max_withdrawal_logs() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let initial_balance = WITHDRAWAL_LOG_AMOUNT * MAX_LOGS_PER_BLOCK;

    let genesis = setup_genesis_with_snark_account(&mut state, snark_id, initial_balance);
    let txs = withdrawal_log_txs(snark_id, MAX_LOGS_PER_BLOCK as usize);

    let block_info = BlockInfo::new(1_001_000, 1, 1);
    let output = execute_block_with_outputs(
        &mut state,
        &block_info,
        Some(genesis.header()),
        BlockComponents::new_txs_from_ol_transactions(txs),
    )
    .expect("Block emitting the maximum log count should execute");
    let block = output.completed_block();

    assert_eq!(
        output.outputs().logs().len(),
        MAX_LOGS_PER_BLOCK as usize,
        "Block should emit the maximum allowed log count"
    );

    let mut verify_state = create_test_genesis_state();
    create_snark_account_with_balance(&mut verify_state, snark_id, initial_balance);
    assert_verification_succeeds(&mut verify_state, genesis.header(), None, genesis.body());
    assert_verification_succeeds(
        &mut verify_state,
        block.header(),
        Some(genesis.header().clone()),
        block.body(),
    );

    let (ol_account_state, snark_account_state) =
        lookup_snark_account_states(&verify_state, snark_id);
    assert_eq!(
        ol_account_state.balance(),
        BitcoinAmount::from_sat(0),
        "Verified state should spend every withdrawal output"
    );
    assert_eq!(
        *snark_account_state.seqno().inner(),
        withdrawal_log_tx_count(MAX_LOGS_PER_BLOCK as usize),
        "Verified state should increment sequence number once per SAU"
    );
}

#[test]
fn test_assemble_rejects_withdrawal_logs_over_limit() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let log_count = MAX_LOGS_PER_BLOCK as usize + 1;
    let initial_balance = WITHDRAWAL_LOG_AMOUNT * log_count as u64;

    let genesis = setup_genesis_with_snark_account(&mut state, snark_id, initial_balance);
    let txs = withdrawal_log_txs(snark_id, log_count);

    let block_info = BlockInfo::new(1_001_000, 1, 1);
    let result = execute_block(
        &mut state,
        &block_info,
        Some(genesis.header()),
        BlockComponents::new_txs_from_ol_transactions(txs),
    );

    match result {
        Err(e) => match e.into_base() {
            ExecError::LogsOverflow { count, max } => {
                assert_eq!(count, log_count);
                assert_eq!(max, MAX_LOGS_PER_BLOCK as usize);
            }
            err => panic!("Expected LogsOverflow, got: {err:?}"),
        },
        Ok(_) => panic!("Block emitting too many logs should fail"),
    }
}

#[test]
fn test_verify_rejects_mismatched_body_root() {
    let mut state = create_test_genesis_state();

    let genesis_info = BlockInfo::new_genesis(1000000);
    let genesis = execute_block(&mut state, &genesis_info, None, genesis_block_components())
        .expect("Genesis assembly should succeed");

    let target = test_account_id(1);
    let block1_info = BlockInfo::new(1001000, 1, 1);
    let block1 = execute_block(
        &mut state,
        &block1_info,
        Some(genesis.header()),
        BlockComponents::new_txs_from_ol_transactions(vec![make_gam_tx(target)]),
    )
    .expect("Block 1 assembly should succeed");

    let wrong_root = Buf32::from([77u8; 32]);
    let tampered_header = tamper_body_root(block1.header(), wrong_root);

    let mut positive_verify_state = create_test_genesis_state();
    assert_verification_succeeds(
        &mut positive_verify_state,
        genesis.header(),
        None,
        genesis.body(),
    );
    assert_verification_succeeds(
        &mut positive_verify_state,
        block1.header(),
        Some(genesis.header().clone()),
        block1.body(),
    );

    let mut verify_state = create_test_genesis_state();
    assert_verification_succeeds(&mut verify_state, genesis.header(), None, genesis.body());

    assert_verification_fails_with(
        &mut verify_state,
        &tampered_header,
        Some(genesis.header().clone()),
        block1.body(),
        |e| matches!(e, ExecError::BlockStructureMismatch),
    );
}

#[test]
fn test_verify_state_root_changes_with_state() {
    let mut state = create_test_genesis_state();

    let genesis_info = BlockInfo::new_genesis(1000000);
    let genesis = execute_block(&mut state, &genesis_info, None, genesis_block_components())
        .expect("Genesis should execute");

    let block1_info = BlockInfo::new(1001000, 1, 1);
    let block1 = execute_block(
        &mut state,
        &block1_info,
        Some(genesis.header()),
        BlockComponents::new_empty(),
    )
    .expect("Block 1 should execute");

    assert_ne!(
        genesis.header().state_root(),
        block1.header().state_root(),
        "State root should change when state changes"
    );

    let mut verify_state = create_test_genesis_state();

    assert_verification_succeeds(&mut verify_state, genesis.header(), None, genesis.body());
    let verified_genesis_state_root = verify_state
        .compute_state_root()
        .expect("verified genesis state root should compute");
    assert_eq!(
        genesis.header().state_root(),
        &verified_genesis_state_root,
        "verified genesis state root should match the block header"
    );

    assert_verification_succeeds(
        &mut verify_state,
        block1.header(),
        Some(genesis.header().clone()),
        block1.body(),
    );
    let verified_block1_state_root = verify_state
        .compute_state_root()
        .expect("verified block 1 state root should compute");
    assert_eq!(
        block1.header().state_root(),
        &verified_block1_state_root,
        "verified block 1 state root should match the block header"
    );
}

fn withdrawal_log_txs(sender_id: AccountId, log_count: usize) -> Vec<OLTransaction> {
    let mut remaining_logs = log_count;
    let mut seq_no = 0;
    let mut txs = Vec::new();

    while remaining_logs > 0 {
        let tx_log_count = remaining_logs.min(MAX_MESSAGES as usize);
        let mut effects = TxEffects::default();

        for _ in 0..tx_log_count {
            let added = effects.push_message(
                BRIDGE_GATEWAY_ACCT_ID,
                WITHDRAWAL_LOG_AMOUNT,
                encode_withdrawal_payload(b"bc1qlogcap", u32::MAX),
            );
            assert!(added, "test: withdrawal log tx should fit effect list");
        }

        txs.push(create_unchecked_snark_update(
            sender_id,
            seq_no,
            get_test_state_root(seq_no as u8 + 2),
            0,
            effects,
        ));

        remaining_logs -= tx_log_count;
        seq_no += 1;
    }

    txs
}

fn withdrawal_log_tx_count(log_count: usize) -> u64 {
    log_count.div_ceil(MAX_MESSAGES as usize) as u64
}
