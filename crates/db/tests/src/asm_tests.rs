use bitcoin::Network;
use strata_asm_common::{AnchorState, AsmHistoryAccumulatorState, AuxData, ChainViewState};
use strata_btc_verification::L1Anchor;
use strata_db_types::asm::{AsmDatabase, AsmExecOutput};
use strata_l1_txfmt::MagicBytes;
use strata_primitives::l1::{L1BlockCommitment, L1BlockId};

pub fn test_get_asm(db: &impl AsmDatabase) {
    let state = AsmExecOutput::new(make_anchor_state(), vec![]);

    db.put_asm_state(L1BlockCommitment::default(), state.clone())
        .expect("test insert");

    let another_block = L1BlockCommitment::new(1, L1BlockId::default());
    db.put_asm_state(another_block, state.clone())
        .expect("test: insert");

    let update = db.get_asm_state(another_block).expect("test: get").unwrap();
    assert_eq!(update, state);
}

/// An anchor state written on its own must be readable on its own.
///
/// This is the genesis shape: the ASM worker stores the genesis anchor state
/// directly, and no manifest ever carries logs for it, so the block has a state
/// entry and no log entry. Reads that demand both halves must not see it, and
/// reads that want only the anchor must.
pub fn test_get_anchor_state_without_logs(db: &impl AsmDatabase) {
    let anchor = make_anchor_state();
    let block = L1BlockCommitment::new(110, L1BlockId::default());

    db.put_anchor_state(block, anchor.clone())
        .expect("test: put anchor state");

    let fetched = db
        .get_anchor_state(block)
        .expect("test: get anchor state")
        .expect("test: anchor state missing");
    assert_eq!(fetched, anchor);

    let (latest_block, latest) = db
        .get_latest_anchor_state()
        .expect("test: get latest anchor state")
        .expect("test: latest anchor state missing");
    assert_eq!(latest_block, block);
    assert_eq!(latest, anchor);

    // The combined read needs both halves, so it must not yield this block.
    assert!(
        db.get_asm_state(block)
            .expect("test: get asm state")
            .is_none(),
        "combined read must not yield a block whose logs are absent"
    );

    // Once the logs land, the combined read completes.
    db.put_asm_logs(block, vec![]).expect("test: put asm logs");
    let combined = db
        .get_asm_state(block)
        .expect("test: get asm state")
        .expect("test: asm state missing after logs");
    assert_eq!(combined, AsmExecOutput::new(anchor, vec![]));
}

/// Minimal [`AsmExecOutput`] for tests that only need a persistable value.
pub fn make_test_asm_state() -> AsmExecOutput {
    AsmExecOutput::new(make_anchor_state(), vec![])
}

fn make_anchor_state() -> AnchorState {
    let anchor = L1Anchor {
        block: L1BlockCommitment::default(),
        next_target: 0,
        epoch_start_timestamp: 0,
        network: Network::Bitcoin,
    };

    AnchorState {
        magic: AnchorState::magic_ssz(MagicBytes::from(*b"ALPN")),
        chain_view: ChainViewState {
            pow_state: strata_asm_common::HeaderVerificationState::init(anchor),
            history_accumulator: AsmHistoryAccumulatorState::new(0),
        },
        sections: Default::default(),
    }
}

pub fn test_put_get_aux_data(db: &impl AsmDatabase) {
    let block = L1BlockCommitment::new(1, L1BlockId::default());

    // Initially no aux data.
    let result = db.get_aux_data(block).expect("test: get empty");
    assert!(result.is_none());

    // Store and retrieve.
    let aux_data = AuxData::default();
    db.put_aux_data(block, aux_data.clone())
        .expect("test: put aux_data");

    let retrieved = db.get_aux_data(block).expect("test: get aux_data").unwrap();
    assert_eq!(retrieved, aux_data);
}

// TODO(STR-2653): add more tests.
#[macro_export]
macro_rules! asm_state_db_tests {
    ($setup_expr:expr) => {
        #[test]
        fn test_get_asm() {
            let db = $setup_expr;
            $crate::asm_tests::test_get_asm(&db);
        }

        #[test]
        fn test_put_get_aux_data() {
            let db = $setup_expr;
            $crate::asm_tests::test_put_get_aux_data(&db);
        }

        #[test]
        fn test_get_anchor_state_without_logs() {
            let db = $setup_expr;
            $crate::asm_tests::test_get_anchor_state_without_logs(&db);
        }
    };
}
