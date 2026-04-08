use strata_asm_common::{
    AnchorState, AsmHistoryAccumulatorState, AuxData, ChainViewState, HeaderVerificationState,
};
use strata_db_types::traits::AsmDatabase;
use strata_l1_txfmt::MagicBytes;
use strata_primitives::l1::{L1BlockCommitment, L1BlockId};
use strata_state::asm_state::AsmState;

pub fn test_get_asm(db: &impl AsmDatabase) {
    // `AnchorState.chain_view.pow_state` is the SSZ-backed ASM-local
    // `HeaderVerificationState` (re-exported from `strata_asm_common`), not the
    // native `strata_btc_verification` one — convert via `from_native`.
    let native_hvs = strata_btc_verification::HeaderVerificationState::default();
    let state = AsmState::new(
        AnchorState {
            magic: AnchorState::magic_ssz(MagicBytes::from([0u8; 4])),
            chain_view: ChainViewState {
                pow_state: HeaderVerificationState::from_native(native_hvs),
                history_accumulator: AsmHistoryAccumulatorState::new(0),
            },
            sections: vec![].into(),
        },
        vec![],
    );

    db.put_asm_state(L1BlockCommitment::default(), state.clone())
        .expect("test insert");

    let another_block = L1BlockCommitment::new(1, L1BlockId::default());
    db.put_asm_state(another_block, state.clone())
        .expect("test: insert");

    let update = db.get_asm_state(another_block).expect("test: get").unwrap();
    assert_eq!(update, state);
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
