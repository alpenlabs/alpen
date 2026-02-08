use strata_asm_common::{AnchorState, AsmHistoryAccumulatorState, ChainViewState};
use strata_asm_types::HeaderVerificationState;
use strata_db_types::traits::AsmDatabase;
use strata_primitives::l1::{L1BlockCommitment, L1BlockId};
use strata_state::asm_state::AsmState;

pub fn test_get_asm(db: &impl AsmDatabase) {
    let state = AsmState::new(
        AnchorState {
            chain_view: ChainViewState {
                pow_state: HeaderVerificationState::default(),
                history_accumulator: AsmHistoryAccumulatorState::new(0),
            },
            sections: vec![],
        },
        vec![],
    );

    db.put_asm_state(L1BlockCommitment::default(), state.clone())
        .expect("test insert");

    let another_block = L1BlockCommitment::from_height_u64(1, L1BlockId::default())
        .expect("height should be valid");
    db.put_asm_state(another_block, state.clone())
        .expect("test: insert");

    let update = db.get_asm_state(another_block).expect("test: get").unwrap();
    assert_eq!(update, state);
}

pub fn test_asm_state_ordering_over_256(db: &impl AsmDatabase) {
    let state = AsmState::new(
        AnchorState {
            chain_view: ChainViewState {
                pow_state: HeaderVerificationState::default(),
                history_accumulator: AsmHistoryAccumulatorState::new(0),
            },
            sections: vec![],
        },
        vec![],
    );

    let max_height = 300u64;
    for height in 0..=max_height {
        let block = L1BlockCommitment::from_height_u64(height, L1BlockId::default())
            .expect("height should be valid");
        db.put_asm_state(block, state.clone()).expect("test insert");
    }

    let (latest_block, _) = db
        .get_latest_asm_state()
        .expect("test: get latest")
        .expect("latest should exist");
    assert_eq!(latest_block.height_u64(), max_height);

    let start_height = 100u64;
    let start_block = L1BlockCommitment::from_height_u64(start_height, L1BlockId::default())
        .expect("height should be valid");
    let states = db
        .get_asm_states_from(start_block, 50)
        .expect("test: range");
    assert_eq!(states.len(), 50);
    assert_eq!(states.first().unwrap().0.height_u64(), start_height);

    let mut last_height = start_height;
    for (block, _) in states {
        let height = block.height_u64();
        assert!(height >= last_height);
        last_height = height;
    }
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
        fn test_asm_state_ordering_over_256() {
            let db = $setup_expr;
            $crate::asm_tests::test_asm_state_ordering_over_256(&db);
        }
    };
}
