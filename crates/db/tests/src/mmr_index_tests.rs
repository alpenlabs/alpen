use proptest::{collection::vec, prelude::*};
use strata_db_types::{traits::MmrIndexDatabase, LeafPos, MmrBatchWrite, NodePos};
use strata_identifiers::Hash;

pub fn raw_mmr_id_strategy() -> impl Strategy<Value = Vec<u8>> {
    vec(any::<u8>(), 1..=8)
}

pub fn node_pos_strategy() -> impl Strategy<Value = NodePos> {
    (0u8..=8u8, 0u64..1024).prop_map(|(height, index)| NodePos::new(height, index))
}

pub fn leaf_pos_strategy() -> impl Strategy<Value = LeafPos> {
    (0u64..1024).prop_map(LeafPos::new)
}

pub fn hash_strategy() -> impl Strategy<Value = Hash> {
    prop::array::uniform32(any::<u8>()).prop_map(Hash::from)
}

pub fn payload_strategy() -> impl Strategy<Value = Vec<u8>> {
    vec(any::<u8>(), 0..128)
}

pub fn proptest_mmr_index_batch_roundtrip(
    db: &impl MmrIndexDatabase,
    mmr_id: Vec<u8>,
    pos: NodePos,
    leaf: LeafPos,
    hash: Hash,
    payload: Vec<u8>,
) {
    let mut batch = MmrBatchWrite::default();
    batch.entry(mmr_id.clone()).put_node(pos, hash);
    batch
        .entry(mmr_id.clone())
        .put_preimage(leaf, payload.clone());

    db.apply_update(batch).expect("apply batch");

    assert_eq!(
        db.get_node(mmr_id.clone(), pos).expect("read node"),
        Some(hash)
    );
    assert_eq!(
        db.get_preimage(mmr_id, leaf).expect("read payload"),
        Some(payload)
    );
}

pub fn proptest_mmr_index_precondition_conflict_rolls_back(
    db: &impl MmrIndexDatabase,
    mmr_id: Vec<u8>,
    pos: NodePos,
    hash_a: Hash,
) {
    let mut setup = MmrBatchWrite::default();
    setup.entry(mmr_id.clone()).put_node(pos, hash_a);
    db.apply_update(setup).expect("setup batch");

    let mut conflict = MmrBatchWrite::default();
    {
        let mmr_batch = conflict.entry(mmr_id.clone());
        mmr_batch.add_node_precond(pos, None);
        mmr_batch.put_node(pos, hash_a);
    }

    assert!(db.apply_update(conflict).is_err());
    assert_eq!(db.get_node(mmr_id, pos).expect("read node"), Some(hash_a));
}

pub fn proptest_mmr_index_payload_precondition_conflict_rolls_back(
    db: &impl MmrIndexDatabase,
    mmr_id: Vec<u8>,
    leaf: LeafPos,
    payload_a: Vec<u8>,
) {
    let mut setup = MmrBatchWrite::default();
    setup
        .entry(mmr_id.clone())
        .put_preimage(leaf, payload_a.clone());
    db.apply_update(setup).expect("setup batch");

    let mut conflict = MmrBatchWrite::default();
    {
        let mmr_batch = conflict.entry(mmr_id.clone());
        mmr_batch.add_preimage_precond(leaf, None);
        mmr_batch.put_preimage(leaf, payload_a.clone());
    }

    assert!(db.apply_update(conflict).is_err());
    assert_eq!(
        db.get_preimage(mmr_id, leaf).expect("read payload"),
        Some(payload_a)
    );
}

pub fn proptest_mmr_index_multi_namespace_atomicity(
    db: &impl MmrIndexDatabase,
    mmr_a: Vec<u8>,
    mmr_b: Vec<u8>,
    pos: NodePos,
    existing_hash: Hash,
    new_hash: Hash,
) {
    let mut setup = MmrBatchWrite::default();
    setup.entry(mmr_b.clone()).put_node(pos, existing_hash);
    db.apply_update(setup).expect("setup batch");

    let mut conflict = MmrBatchWrite::default();
    conflict.entry(mmr_b.clone()).add_node_precond(pos, None);
    conflict.entry(mmr_a.clone()).put_node(pos, new_hash);
    conflict.entry(mmr_b.clone()).put_node(pos, new_hash);

    assert!(db.apply_update(conflict).is_err());
    assert_eq!(db.get_node(mmr_a, pos).expect("read mmr_a"), None);
    assert_eq!(
        db.get_node(mmr_b, pos).expect("read mmr_b"),
        Some(existing_hash)
    );
}

#[derive(Debug)]
pub struct AtomicRollbackPayloadConflictInput {
    pub mmr_a: Vec<u8>,
    pub mmr_b: Vec<u8>,
    pub pos_a: NodePos,
    pub pos_b: NodePos,
    pub leaf_a: LeafPos,
    pub leaf_b: LeafPos,
    pub hash_a: Hash,
    pub hash_b: Hash,
    pub payload_a: Vec<u8>,
    pub payload_b_existing: Vec<u8>,
    pub payload_b_new: Vec<u8>,
}

pub fn proptest_mmr_index_atomic_rollback_on_payload_precondition_failure(
    db: &impl MmrIndexDatabase,
    input: AtomicRollbackPayloadConflictInput,
) {
    let mut setup = MmrBatchWrite::default();
    setup
        .entry(input.mmr_b.clone())
        .put_preimage(input.leaf_b, input.payload_b_existing.clone());
    db.apply_update(setup).expect("setup batch");

    let mut conflict = MmrBatchWrite::default();
    conflict
        .entry(input.mmr_b.clone())
        .add_preimage_precond(input.leaf_b, None);
    {
        let mmr_a_batch = conflict.entry(input.mmr_a.clone());
        mmr_a_batch.put_node(input.pos_a, input.hash_a);
        mmr_a_batch.put_preimage(input.leaf_a, input.payload_a);
    }
    {
        let mmr_b_batch = conflict.entry(input.mmr_b.clone());
        mmr_b_batch.put_node(input.pos_b, input.hash_b);
        mmr_b_batch.put_preimage(input.leaf_b, input.payload_b_new);
    }

    assert!(db.apply_update(conflict).is_err());

    // No writes from the failed transaction should be visible.
    assert_eq!(
        db.get_node(input.mmr_a.clone(), input.pos_a)
            .expect("read mmr_a node"),
        None
    );
    assert_eq!(
        db.get_preimage(input.mmr_a, input.leaf_a)
            .expect("read mmr_a payload"),
        None
    );
    assert_eq!(
        db.get_preimage(input.mmr_b, input.leaf_b)
            .expect("read mmr_b payload"),
        Some(input.payload_b_existing)
    );
}

pub fn proptest_mmr_index_delete_node_and_payload(
    db: &impl MmrIndexDatabase,
    mmr_id: Vec<u8>,
    pos: NodePos,
    leaf: LeafPos,
    hash: Hash,
    payload: Vec<u8>,
) {
    let mut setup = MmrBatchWrite::default();
    setup.entry(mmr_id.clone()).put_node(pos, hash);
    setup
        .entry(mmr_id.clone())
        .put_preimage(leaf, payload.clone());
    db.apply_update(setup).expect("setup batch");

    assert_eq!(
        db.get_node(mmr_id.clone(), pos).expect("read node"),
        Some(hash)
    );
    assert_eq!(
        db.get_preimage(mmr_id.clone(), leaf).expect("read payload"),
        Some(payload.clone())
    );

    let mut del = MmrBatchWrite::default();
    {
        let mmr_batch = del.entry(mmr_id.clone());
        mmr_batch.add_node_precond(pos, Some(hash));
        mmr_batch.add_preimage_precond(leaf, Some(payload));
        mmr_batch.del_node(pos);
        mmr_batch.del_preimage(leaf);
    }
    db.apply_update(del).expect("delete batch");

    assert_eq!(db.get_node(mmr_id.clone(), pos).expect("read node"), None);
    assert_eq!(db.get_preimage(mmr_id, leaf).expect("read payload"), None);
}

pub fn proptest_mmr_index_last_write_wins_db_boundary(
    db: &impl MmrIndexDatabase,
    mmr_id: Vec<u8>,
    pos: NodePos,
    leaf: LeafPos,
    hash: Hash,
    payload: Vec<u8>,
) {
    let mut put_then_del = MmrBatchWrite::default();
    {
        let batch = put_then_del.entry(mmr_id.clone());
        batch.put_node(pos, hash);
        batch.del_node(pos);
        batch.put_preimage(leaf, payload.clone());
        batch.del_preimage(leaf);
    }
    db.apply_update(put_then_del).expect("put-then-del batch");
    assert_eq!(db.get_node(mmr_id.clone(), pos).expect("read node"), None);
    assert_eq!(
        db.get_preimage(mmr_id.clone(), leaf).expect("read payload"),
        None
    );

    let mut del_then_put = MmrBatchWrite::default();
    {
        let batch = del_then_put.entry(mmr_id.clone());
        batch.del_node(pos);
        batch.put_node(pos, hash);
        batch.del_preimage(leaf);
        batch.put_preimage(leaf, payload.clone());
    }
    db.apply_update(del_then_put).expect("del-then-put batch");
    assert_eq!(
        db.get_node(mmr_id.clone(), pos).expect("read node"),
        Some(hash)
    );
    assert_eq!(
        db.get_preimage(mmr_id, leaf).expect("read payload"),
        Some(payload)
    );
}

pub fn proptest_mmr_index_idempotent_delete_absent_keys(
    db: &impl MmrIndexDatabase,
    mmr_id: Vec<u8>,
    pos: NodePos,
    leaf: LeafPos,
) {
    let mut delete_once = MmrBatchWrite::default();
    delete_once.entry(mmr_id.clone()).del_node(pos);
    delete_once.entry(mmr_id.clone()).del_preimage(leaf);
    db.apply_update(delete_once).expect("first delete");

    let mut delete_twice = MmrBatchWrite::default();
    delete_twice.entry(mmr_id.clone()).del_node(pos);
    delete_twice.entry(mmr_id.clone()).del_preimage(leaf);
    db.apply_update(delete_twice).expect("second delete");

    assert_eq!(db.get_node(mmr_id.clone(), pos).expect("read node"), None);
    assert_eq!(db.get_preimage(mmr_id, leaf).expect("read payload"), None);
}

pub fn proptest_mmr_index_namespace_isolation_positive(
    db: &impl MmrIndexDatabase,
    mmr_a: Vec<u8>,
    mmr_b: Vec<u8>,
    pos: NodePos,
    leaf: LeafPos,
    hash: Hash,
) {
    let mut batch = MmrBatchWrite::default();
    batch.entry(mmr_a.clone()).put_node(pos, hash);
    db.apply_update(batch).expect("apply batch");

    assert_eq!(db.get_node(mmr_a, pos).expect("read node"), Some(hash));
    assert_eq!(db.get_node(mmr_b.clone(), pos).expect("read node"), None);
    assert_eq!(db.get_preimage(mmr_b, leaf).expect("read payload"), None);
}

pub fn test_mmr_index_leaf_count_default_zero(db: &impl MmrIndexDatabase, mmr_id: Vec<u8>) {
    assert_eq!(
        db.get_leaf_count(mmr_id).expect("read initial leaf count"),
        0
    );
}

pub fn test_mmr_index_leaf_count_cas_conflict_rolls_back(
    db: &impl MmrIndexDatabase,
    mmr_id: Vec<u8>,
    pos: NodePos,
    hash: Hash,
) {
    let mut setup = MmrBatchWrite::default();
    setup.entry(mmr_id.clone()).set_leaf_count(2);
    db.apply_update(setup).expect("setup leaf count");

    let mut stale = MmrBatchWrite::default();
    {
        let mmr_batch = stale.entry(mmr_id.clone());
        mmr_batch.set_expected_leaf_count(1);
        mmr_batch.put_node(pos, hash);
        mmr_batch.set_leaf_count(3);
    }

    assert!(db.apply_update(stale).is_err());
    assert_eq!(
        db.get_leaf_count(mmr_id.clone())
            .expect("leaf count after conflict"),
        2
    );
    assert_eq!(db.get_node(mmr_id, pos).expect("node after conflict"), None);
}

#[macro_export]
macro_rules! mmr_index_db_tests {
    ($setup_expr:expr) => {
        proptest::proptest! {
            #[test]
            fn proptest_mmr_index_batch_roundtrip_contract(
                mmr_id in $crate::mmr_index_tests::raw_mmr_id_strategy(),
                pos in $crate::mmr_index_tests::node_pos_strategy(),
                leaf in $crate::mmr_index_tests::leaf_pos_strategy(),
                hash in $crate::mmr_index_tests::hash_strategy(),
                payload in $crate::mmr_index_tests::payload_strategy(),
            ) {
                let db = $setup_expr;
                $crate::mmr_index_tests::proptest_mmr_index_batch_roundtrip(
                    &db, mmr_id, pos, leaf, hash, payload
                );
            }

            #[test]
            fn proptest_mmr_index_precondition_conflict_contract(
                mmr_id in $crate::mmr_index_tests::raw_mmr_id_strategy(),
                pos in $crate::mmr_index_tests::node_pos_strategy(),
                hash_a in $crate::mmr_index_tests::hash_strategy(),
            ) {
                let db = $setup_expr;
                $crate::mmr_index_tests::proptest_mmr_index_precondition_conflict_rolls_back(
                    &db, mmr_id, pos, hash_a
                );
            }

            #[test]
            fn proptest_mmr_index_payload_precondition_conflict_contract(
                mmr_id in $crate::mmr_index_tests::raw_mmr_id_strategy(),
                leaf in $crate::mmr_index_tests::leaf_pos_strategy(),
                payload_a in $crate::mmr_index_tests::payload_strategy(),
            ) {
                let db = $setup_expr;
                $crate::mmr_index_tests::proptest_mmr_index_payload_precondition_conflict_rolls_back(
                    &db, mmr_id, leaf, payload_a
                );
            }

            #[test]
            fn proptest_mmr_index_multi_namespace_atomicity_contract(
                mmr_a in $crate::mmr_index_tests::raw_mmr_id_strategy(),
                mmr_b in $crate::mmr_index_tests::raw_mmr_id_strategy(),
                pos in $crate::mmr_index_tests::node_pos_strategy(),
                existing_hash in $crate::mmr_index_tests::hash_strategy(),
                new_hash in $crate::mmr_index_tests::hash_strategy(),
            ) {
                proptest::prop_assume!(mmr_a != mmr_b);
                let db = $setup_expr;
                $crate::mmr_index_tests::proptest_mmr_index_multi_namespace_atomicity(
                    &db, mmr_a, mmr_b, pos, existing_hash, new_hash
                );
            }

            #[test]
            fn proptest_mmr_index_atomic_rollback_on_payload_precondition_failure_contract(
                mmr_a in $crate::mmr_index_tests::raw_mmr_id_strategy(),
                mmr_b in $crate::mmr_index_tests::raw_mmr_id_strategy(),
                pos_a in $crate::mmr_index_tests::node_pos_strategy(),
                pos_b in $crate::mmr_index_tests::node_pos_strategy(),
                leaf_a in $crate::mmr_index_tests::leaf_pos_strategy(),
                leaf_b in $crate::mmr_index_tests::leaf_pos_strategy(),
                hash_a in $crate::mmr_index_tests::hash_strategy(),
                hash_b in $crate::mmr_index_tests::hash_strategy(),
                payload_a in $crate::mmr_index_tests::payload_strategy(),
                payload_b_existing in $crate::mmr_index_tests::payload_strategy(),
                payload_b_new in $crate::mmr_index_tests::payload_strategy(),
            ) {
                proptest::prop_assume!(mmr_a != mmr_b);
                let db = $setup_expr;
                $crate::mmr_index_tests::proptest_mmr_index_atomic_rollback_on_payload_precondition_failure(
                    &db,
                    $crate::mmr_index_tests::AtomicRollbackPayloadConflictInput {
                        mmr_a,
                        mmr_b,
                        pos_a,
                        pos_b,
                        leaf_a,
                        leaf_b,
                        hash_a,
                        hash_b,
                        payload_a,
                        payload_b_existing,
                        payload_b_new,
                    },
                );
            }

            #[test]
            fn proptest_mmr_index_delete_node_and_payload_contract(
                mmr_id in $crate::mmr_index_tests::raw_mmr_id_strategy(),
                pos in $crate::mmr_index_tests::node_pos_strategy(),
                leaf in $crate::mmr_index_tests::leaf_pos_strategy(),
                hash in $crate::mmr_index_tests::hash_strategy(),
                payload in $crate::mmr_index_tests::payload_strategy(),
            ) {
                let db = $setup_expr;
                $crate::mmr_index_tests::proptest_mmr_index_delete_node_and_payload(
                    &db, mmr_id, pos, leaf, hash, payload
                );
            }

            #[test]
            fn proptest_mmr_index_last_write_wins_db_boundary_contract(
                mmr_id in $crate::mmr_index_tests::raw_mmr_id_strategy(),
                pos in $crate::mmr_index_tests::node_pos_strategy(),
                leaf in $crate::mmr_index_tests::leaf_pos_strategy(),
                hash in $crate::mmr_index_tests::hash_strategy(),
                payload in $crate::mmr_index_tests::payload_strategy(),
            ) {
                let db = $setup_expr;
                $crate::mmr_index_tests::proptest_mmr_index_last_write_wins_db_boundary(
                    &db, mmr_id, pos, leaf, hash, payload
                );
            }

            #[test]
            fn proptest_mmr_index_idempotent_delete_absent_keys_contract(
                mmr_id in $crate::mmr_index_tests::raw_mmr_id_strategy(),
                pos in $crate::mmr_index_tests::node_pos_strategy(),
                leaf in $crate::mmr_index_tests::leaf_pos_strategy(),
            ) {
                let db = $setup_expr;
                $crate::mmr_index_tests::proptest_mmr_index_idempotent_delete_absent_keys(
                    &db, mmr_id, pos, leaf
                );
            }

            #[test]
            fn proptest_mmr_index_namespace_isolation_positive_contract(
                mmr_a in $crate::mmr_index_tests::raw_mmr_id_strategy(),
                mmr_b in $crate::mmr_index_tests::raw_mmr_id_strategy(),
                pos in $crate::mmr_index_tests::node_pos_strategy(),
                leaf in $crate::mmr_index_tests::leaf_pos_strategy(),
                hash in $crate::mmr_index_tests::hash_strategy(),
            ) {
                proptest::prop_assume!(mmr_a != mmr_b);
                let db = $setup_expr;
                $crate::mmr_index_tests::proptest_mmr_index_namespace_isolation_positive(
                    &db, mmr_a, mmr_b, pos, leaf, hash
                );
            }

            #[test]
            fn test_mmr_index_leaf_count_default_zero_contract(
                mmr_id in $crate::mmr_index_tests::raw_mmr_id_strategy(),
            ) {
                let db = $setup_expr;
                $crate::mmr_index_tests::test_mmr_index_leaf_count_default_zero(&db, mmr_id);
            }

            #[test]
            fn test_mmr_index_leaf_count_cas_conflict_rolls_back_contract(
                mmr_id in $crate::mmr_index_tests::raw_mmr_id_strategy(),
                pos in $crate::mmr_index_tests::node_pos_strategy(),
                hash in $crate::mmr_index_tests::hash_strategy(),
            ) {
                let db = $setup_expr;
                $crate::mmr_index_tests::test_mmr_index_leaf_count_cas_conflict_rolls_back(
                    &db, mmr_id, pos, hash
                );
            }
        }
    };
}
