use bitcoin::Network;
use strata_asm_common::{AnchorState, AsmHistoryAccumulatorState, AuxData, ChainViewState};
use strata_btc_verification::L1Anchor;
use strata_db_types::asm::AsmDatabase;
use strata_identifiers::Buf32;
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

pub fn test_del_asm_entries_after(db: &impl AsmDatabase) {
    let state = AsmState::new(make_anchor_state(), vec![]);
    let pivot = L1BlockCommitment::new(10, L1BlockId::from(Buf32::from([10; 32])));
    let same_height_orphan = L1BlockCommitment::new(10, L1BlockId::from(Buf32::from([250; 32])));
    let higher = L1BlockCommitment::new(11, L1BlockId::from(Buf32::from([11; 32])));
    let aux_only = L1BlockCommitment::new(12, L1BlockId::from(Buf32::from([12; 32])));

    for block in [pivot, same_height_orphan, higher] {
        db.put_asm_state(block, state.clone())
            .expect("test: insert ASM state");
        db.put_aux_data(block, AuxData::default())
            .expect("test: insert aux data");
    }
    db.put_aux_data(aux_only, AuxData::default())
        .expect("test: insert aux-only data");

    let deleted = db
        .del_asm_entries_after(pivot)
        .expect("test: delete ASM suffix");

    assert_eq!(deleted, vec![same_height_orphan, higher, aux_only]);
    assert!(db
        .get_asm_state(pivot)
        .expect("test: get kept state")
        .is_some());
    assert!(db
        .get_aux_data(pivot)
        .expect("test: get kept aux data")
        .is_some());
    for block in [same_height_orphan, higher, aux_only] {
        assert!(db
            .get_asm_state(block)
            .expect("test: get deleted state")
            .is_none());
        assert!(db
            .get_aux_data(block)
            .expect("test: get deleted aux data")
            .is_none());
    }

    let (latest, _) = db
        .get_latest_asm_state()
        .expect("test: get latest state")
        .expect("test: kept state remains");
    assert_eq!(latest, pivot);
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
        fn test_del_asm_entries_after() {
            let db = $setup_expr;
            $crate::asm_tests::test_del_asm_entries_after(&db);
        }
    };
}
