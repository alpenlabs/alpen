use strata_asm_common::{AnchorState, ChainViewState};
use strata_asm_types::HeaderVerificationState;
use strata_db::traits::AsmDatabase;
use strata_primitives::l1::L1BlockCommitment;
use strata_state::{asm_state::AsmState, l1::L1BlockId};

pub fn test_get_asm(db: &impl AsmDatabase) {
    let state = AsmState::new(
        AnchorState {
            chain_view: ChainViewState {
                pow_state: HeaderVerificationState::default(),
            },
            sections: vec![],
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

// TODO(QQ): add more tests.
#[macro_export]
macro_rules! asm_state_db_tests {
    ($setup_expr:expr) => {
        #[test]
        fn test_get_asm() {
            let db = $setup_expr;
            $crate::asm_tests::test_get_asm(&db);
        }
    };
}
