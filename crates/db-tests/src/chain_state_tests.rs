use strata_db::chainstate::ChainstateDatabase;
use strata_primitives::buf::Buf32;
use strata_state::{
    chain_state::Chainstate,
    state_op::WriteBatch,
};
use strata_test_utils::ArbitraryGenerator;

pub fn test_create_and_get_state_instance(db: &impl ChainstateDatabase) {
    let mut generator = ArbitraryGenerator::new();
    let state: Chainstate = generator.generate();

    // Test creating a new state instance
    let inst_id = db.create_new_inst(state.clone()).unwrap();
    
    // Test retrieving the state
    let retrieved_state = db.get_inst_toplevel_state(inst_id).unwrap();
    assert_eq!(state, retrieved_state);
    
    // Test getting the state root
    let expected_root = state.compute_state_root();
    let retrieved_root = db.get_inst_root(inst_id).unwrap();
    assert_eq!(expected_root, retrieved_root);
}

pub fn test_clone_state_instance(db: &impl ChainstateDatabase) {
    let mut generator = ArbitraryGenerator::new();
    let state: Chainstate = generator.generate();

    // Create original instance
    let original_id = db.create_new_inst(state.clone()).unwrap();
    
    // Clone the instance
    let cloned_id = db.clone_inst(original_id).unwrap();
    
    // Verify both instances have the same state
    let original_state = db.get_inst_toplevel_state(original_id).unwrap();
    let cloned_state = db.get_inst_toplevel_state(cloned_id).unwrap();
    assert_eq!(original_state, cloned_state);
    assert_eq!(state, original_state);
    assert_eq!(state, cloned_state);
}

pub fn test_write_batch_operations(db: &impl ChainstateDatabase) {
    let mut generator = ArbitraryGenerator::new();
    let state: Chainstate = generator.generate();
    let wb = WriteBatch::new_replace_toplevel(state);
    let wb_id: Buf32 = generator.generate();
    
    // Test putting and getting write batch
    db.put_write_batch(wb_id, wb.clone()).unwrap();
    let retrieved_wb = db.get_write_batch(wb_id).unwrap();
    assert_eq!(Some(wb), retrieved_wb);
    
    // Test deleting write batch
    db.del_write_batch(wb_id).unwrap();
    let deleted_wb = db.get_write_batch(wb_id).unwrap();
    assert_eq!(None, deleted_wb);
}

pub fn test_delete_state_instance(db: &impl ChainstateDatabase) {
    let mut generator = ArbitraryGenerator::new();
    let state: Chainstate = generator.generate();

    // Create and verify instance exists
    let inst_id = db.create_new_inst(state.clone()).unwrap();
    let retrieved_state = db.get_inst_toplevel_state(inst_id).unwrap();
    assert_eq!(state, retrieved_state);
    
    // Delete instance
    db.del_inst(inst_id).unwrap();
    
    // Verify instance no longer exists
    let result = db.get_inst_toplevel_state(inst_id);
    assert!(result.is_err());
}

pub fn test_get_all_instances(db: &impl ChainstateDatabase) {
    let mut generator = ArbitraryGenerator::new();
    
    // Initially no instances
    let instances = db.get_insts().unwrap();
    assert!(instances.is_empty());
    
    // Create multiple instances
    let state1: Chainstate = generator.generate();
    let state2: Chainstate = generator.generate();
    
    let id1 = db.create_new_inst(state1).unwrap();
    let id2 = db.create_new_inst(state2).unwrap();
    
    // Verify all instances are returned
    let instances = db.get_insts().unwrap();
    assert_eq!(instances.len(), 2);
    assert!(instances.contains(&id1));
    assert!(instances.contains(&id2));
}

pub fn test_merge_write_batches(db: &impl ChainstateDatabase) {
    let mut generator = ArbitraryGenerator::new();
    let initial_state: Chainstate = generator.generate();
    let final_state: Chainstate = generator.generate();
    
    // Create a state instance
    let inst_id = db.create_new_inst(initial_state.clone()).unwrap();
    
    // Create and store write batches
    let wb1 = WriteBatch::new_replace_toplevel(final_state.clone());
    let wb_id1: Buf32 = generator.generate();
    let wb_id2: Buf32 = generator.generate();
    
    db.put_write_batch(wb_id1, wb1.clone()).unwrap();
    db.put_write_batch(wb_id2, wb1.clone()).unwrap();
    
    // Merge write batches
    db.merge_write_batches(inst_id, vec![wb_id1, wb_id2]).unwrap();
    
    // Verify the final state
    let merged_state = db.get_inst_toplevel_state(inst_id).unwrap();
    assert_eq!(final_state, merged_state);
}

#[macro_export]
macro_rules! chain_state_db_tests {
    ($setup_expr:expr) => {
        #[test]
        fn test_create_and_get_state_instance() {
            let db = $setup_expr;
            $crate::chain_state_tests::test_create_and_get_state_instance(&db);
        }

        #[test]
        fn test_clone_state_instance() {
            let db = $setup_expr;
            $crate::chain_state_tests::test_clone_state_instance(&db);
        }

        #[test]
        fn test_write_batch_operations() {
            let db = $setup_expr;
            $crate::chain_state_tests::test_write_batch_operations(&db);
        }

        #[test]
        fn test_delete_state_instance() {
            let db = $setup_expr;
            $crate::chain_state_tests::test_delete_state_instance(&db);
        }

        #[test]
        fn test_get_all_instances() {
            let db = $setup_expr;
            $crate::chain_state_tests::test_get_all_instances(&db);
        }

        #[test]
        fn test_merge_write_batches() {
            let db = $setup_expr;
            $crate::chain_state_tests::test_merge_write_batches(&db);
        }
    };
}
