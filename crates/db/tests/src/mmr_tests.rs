use strata_db_types::traits::AccountMmrDatabase;
use strata_identifiers::AccountId;

const TEST_ACCOUNT: AccountId = AccountId::zero();

pub fn test_append_single_leaf(db: &impl AccountMmrDatabase) {
    let hash = [1u8; 32];

    // Initially empty
    assert_eq!(db.num_leaves(TEST_ACCOUNT).unwrap(), 0);
    assert_eq!(db.mmr_size(TEST_ACCOUNT).unwrap(), 0);

    // Append first leaf
    let idx = db.append_leaf(TEST_ACCOUNT, hash).unwrap();
    assert_eq!(idx, 0);
    assert_eq!(db.num_leaves(TEST_ACCOUNT).unwrap(), 1);
    assert_eq!(db.mmr_size(TEST_ACCOUNT).unwrap(), 1);

    // Can retrieve the node
    let node = db.get_node(TEST_ACCOUNT, 0).unwrap();
    assert_eq!(node, hash);
}

pub fn test_append_multiple_leaves(db: &impl AccountMmrDatabase) {
    // Append 7 leaves to create a complete tree
    let hashes: Vec<[u8; 32]> = (0..7).map(|i| [i; 32]).collect();

    for (expected_idx, hash) in hashes.iter().enumerate() {
        let idx = db.append_leaf(TEST_ACCOUNT, *hash).unwrap();
        assert_eq!(idx as usize, expected_idx);
    }

    assert_eq!(db.num_leaves(TEST_ACCOUNT).unwrap(), 7);
    // 7 leaves with 3 peaks -> mmr_size = 2*7 - 3 = 11
    assert_eq!(db.mmr_size(TEST_ACCOUNT).unwrap(), 11);
}

pub fn test_get_node_positions(db: &impl AccountMmrDatabase) {
    // Append 4 leaves
    let hashes: Vec<[u8; 32]> = (0..4).map(|i| [i; 32]).collect();

    for hash in &hashes {
        db.append_leaf(TEST_ACCOUNT, *hash).unwrap();
    }

    // MMR with 4 leaves has 7 nodes:
    // Position: [0, 1, 2, 3, 4, 5, 6]
    // Height:   [0, 0, 1, 0, 0, 1, 2]
    // Leaves:   [0, 1, x, 2, 3, x, x]

    // Verify leaf positions
    assert_eq!(db.get_node(TEST_ACCOUNT, 0).unwrap(), [0u8; 32]);
    assert_eq!(db.get_node(TEST_ACCOUNT, 1).unwrap(), [1u8; 32]);
    assert_eq!(db.get_node(TEST_ACCOUNT, 3).unwrap(), [2u8; 32]);
    assert_eq!(db.get_node(TEST_ACCOUNT, 4).unwrap(), [3u8; 32]);

    // Internal nodes exist
    assert!(db.get_node(TEST_ACCOUNT, 2).is_ok());
    assert!(db.get_node(TEST_ACCOUNT, 5).is_ok());
    assert!(db.get_node(TEST_ACCOUNT, 6).is_ok());
}

pub fn test_peak_roots(db: &impl AccountMmrDatabase) {
    // Single leaf: one peak
    db.append_leaf(TEST_ACCOUNT, [1u8; 32]).unwrap();
    let peaks = db.peak_roots(TEST_ACCOUNT);
    assert_eq!(peaks.len(), 1);

    // Two leaves: one peak (merged)
    db.append_leaf(TEST_ACCOUNT, [2u8; 32]).unwrap();
    let peaks = db.peak_roots(TEST_ACCOUNT);
    assert_eq!(peaks.len(), 1);

    // Three leaves: two peaks
    db.append_leaf(TEST_ACCOUNT, [3u8; 32]).unwrap();
    let peaks = db.peak_roots(TEST_ACCOUNT);
    assert_eq!(peaks.len(), 2);

    // Four leaves: one peak (complete tree)
    db.append_leaf(TEST_ACCOUNT, [4u8; 32]).unwrap();
    let peaks = db.peak_roots(TEST_ACCOUNT);
    assert_eq!(peaks.len(), 1);
}

pub fn test_pop_leaf_empty(db: &impl AccountMmrDatabase) {
    // Popping from empty MMR returns None
    let result = db.pop_leaf(TEST_ACCOUNT).unwrap();
    assert_eq!(result, None);
}

pub fn test_pop_leaf_single(db: &impl AccountMmrDatabase) {
    let hash = [42u8; 32];
    db.append_leaf(TEST_ACCOUNT, hash).unwrap();

    assert_eq!(db.num_leaves(TEST_ACCOUNT).unwrap(), 1);

    let popped = db.pop_leaf(TEST_ACCOUNT).unwrap();
    assert_eq!(popped, Some(hash));
    assert_eq!(db.num_leaves(TEST_ACCOUNT).unwrap(), 0);
    assert_eq!(db.mmr_size(TEST_ACCOUNT).unwrap(), 0);
}

pub fn test_pop_leaf_multiple(db: &impl AccountMmrDatabase) {
    // Append several leaves
    let hashes: Vec<[u8; 32]> = (0..5).map(|i| [i; 32]).collect();
    for hash in &hashes {
        db.append_leaf(TEST_ACCOUNT, *hash).unwrap();
    }

    assert_eq!(db.num_leaves(TEST_ACCOUNT).unwrap(), 5);

    // Pop last leaf
    let popped = db.pop_leaf(TEST_ACCOUNT).unwrap();
    assert_eq!(popped, Some([4u8; 32]));
    assert_eq!(db.num_leaves(TEST_ACCOUNT).unwrap(), 4);

    // Pop another
    let popped = db.pop_leaf(TEST_ACCOUNT).unwrap();
    assert_eq!(popped, Some([3u8; 32]));
    assert_eq!(db.num_leaves(TEST_ACCOUNT).unwrap(), 3);
}

pub fn test_append_after_pop(db: &impl AccountMmrDatabase) {
    // Append 3 leaves
    db.append_leaf(TEST_ACCOUNT, [1u8; 32]).unwrap();
    db.append_leaf(TEST_ACCOUNT, [2u8; 32]).unwrap();
    db.append_leaf(TEST_ACCOUNT, [3u8; 32]).unwrap();

    // Pop one
    db.pop_leaf(TEST_ACCOUNT).unwrap();
    assert_eq!(db.num_leaves(TEST_ACCOUNT).unwrap(), 2);

    // Append again
    let idx = db.append_leaf(TEST_ACCOUNT, [4u8; 32]).unwrap();
    assert_eq!(idx, 2);
    assert_eq!(db.num_leaves(TEST_ACCOUNT).unwrap(), 3);
}

pub fn test_to_compact(db: &impl AccountMmrDatabase) {
    // Empty MMR
    let compact = db.to_compact(TEST_ACCOUNT);
    assert_eq!(compact.entries, 0);

    // Add some leaves
    for i in 0..4 {
        db.append_leaf(TEST_ACCOUNT, [i; 32]).unwrap();
    }

    let compact = db.to_compact(TEST_ACCOUNT);
    assert_eq!(compact.entries, 4);
    assert_eq!(compact.cap_log2, 64);
    assert!(!compact.roots.is_empty());
}

pub fn test_mmr_size_formula(db: &impl AccountMmrDatabase) {
    // Test the MMR size formula: size = 2 * leaves - peaks
    // where peaks = number of set bits in binary representation of leaves

    let test_cases = vec![
        (1, 1),  // 1 leaf, 1 peak -> 2*1 - 1 = 1
        (2, 3),  // 2 leaves, 1 peak -> 2*2 - 1 = 3
        (3, 4),  // 3 leaves, 2 peaks -> 2*3 - 2 = 4
        (4, 7),  // 4 leaves, 1 peak -> 2*4 - 1 = 7
        (7, 11), // 7 leaves, 3 peaks -> 2*7 - 3 = 11
    ];

    for (num_leaves, expected_size) in test_cases {
        // Clear and rebuild
        while db.num_leaves(TEST_ACCOUNT).unwrap() > 0 {
            db.pop_leaf(TEST_ACCOUNT).unwrap();
        }

        for i in 0..num_leaves {
            db.append_leaf(TEST_ACCOUNT, [i; 32]).unwrap();
        }

        assert_eq!(
            db.mmr_size(TEST_ACCOUNT).unwrap(),
            expected_size,
            "MMR size mismatch for {} leaves",
            num_leaves
        );
    }
}

#[macro_export]
macro_rules! mmr_db_tests {
    ($setup_expr:expr) => {
        #[test]
        fn test_append_single_leaf() {
            let db = $setup_expr;
            $crate::mmr_tests::test_append_single_leaf(&db);
        }

        #[test]
        fn test_append_multiple_leaves() {
            let db = $setup_expr;
            $crate::mmr_tests::test_append_multiple_leaves(&db);
        }

        #[test]
        fn test_get_node_positions() {
            let db = $setup_expr;
            $crate::mmr_tests::test_get_node_positions(&db);
        }

        #[test]
        fn test_peak_roots() {
            let db = $setup_expr;
            $crate::mmr_tests::test_peak_roots(&db);
        }

        #[test]
        fn test_pop_leaf_empty() {
            let db = $setup_expr;
            $crate::mmr_tests::test_pop_leaf_empty(&db);
        }

        #[test]
        fn test_pop_leaf_single() {
            let db = $setup_expr;
            $crate::mmr_tests::test_pop_leaf_single(&db);
        }

        #[test]
        fn test_pop_leaf_multiple() {
            let db = $setup_expr;
            $crate::mmr_tests::test_pop_leaf_multiple(&db);
        }

        #[test]
        fn test_append_after_pop() {
            let db = $setup_expr;
            $crate::mmr_tests::test_append_after_pop(&db);
        }

        #[test]
        fn test_to_compact() {
            let db = $setup_expr;
            $crate::mmr_tests::test_to_compact(&db);
        }

        #[test]
        fn test_mmr_size_formula() {
            let db = $setup_expr;
            $crate::mmr_tests::test_mmr_size_formula(&db);
        }
    };
}
