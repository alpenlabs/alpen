use bitcoin::Network;
use strata_asm_common::{AnchorState, AsmHistoryAccumulatorState, AuxData, ChainViewState};
use strata_btc_verification::L1Anchor;
use strata_db_types::traits::AsmDatabase;
use strata_l1_txfmt::MagicBytes;
use strata_primitives::l1::{L1BlockCommitment, L1BlockId};
use strata_state::asm_state::AsmState;

pub fn test_get_asm(db: &impl AsmDatabase) {
    let state = AsmState::new(make_anchor_state(), vec![]);

    db.put_asm_state(L1BlockCommitment::default(), state.clone())
        .expect("test insert");

    let another_block = L1BlockCommitment::new(1, L1BlockId::default());
    db.put_asm_state(another_block, state.clone())
        .expect("test: insert");

    let update = db.get_asm_state(another_block).expect("test: get").unwrap();
    assert_eq!(update, state);
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

// TODO(QQ): add more tests.
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
    };
}
