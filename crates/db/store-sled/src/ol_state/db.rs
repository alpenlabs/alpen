use strata_db_types::DbResult;
use strata_db_types::ol_state::OLStateDatabase;
use strata_identifiers::OLBlockCommitment;
use strata_ol_state_container::OLStateContainer;
use strata_ol_state_types_v1::WriteBatch;

use super::schemas::{OLStateSchema, OLWriteBatchSchema};
use crate::define_sled_database;
use crate::utils::conv_sled_err;

define_sled_database!(
    pub struct OLStateDBSled {
        state_tree: OLStateSchema,
        write_batch_tree: OLWriteBatchSchema,
    }
);

impl OLStateDatabase for OLStateDBSled {
    fn put_toplevel_ol_state(
        &self,
        commitment: OLBlockCommitment,
        state: OLStateContainer,
    ) -> DbResult<()> {
        self.config
            .with_retry((&self.state_tree,), |(state_tree,)| {
                state_tree.insert(&commitment, &state)?;
                Ok(())
            })?;
        Ok(())
    }

    fn get_toplevel_ol_state(
        &self,
        commitment: OLBlockCommitment,
    ) -> DbResult<Option<OLStateContainer>> {
        self.state_tree.get(&commitment).map_err(conv_sled_err)
    }

    fn get_latest_toplevel_ol_state(
        &self,
    ) -> DbResult<Option<(OLBlockCommitment, OLStateContainer)>> {
        // Relying on the lexicographical order of OLBlockCommitment (slot + block ID).
        // The last entry should be the one with the highest slot.
        self.state_tree.last().map_err(conv_sled_err)
    }

    fn del_toplevel_ol_state(&self, commitment: OLBlockCommitment) -> DbResult<()> {
        self.state_tree.remove(&commitment).map_err(conv_sled_err)?;
        Ok(())
    }

    fn put_ol_write_batch(&self, commitment: OLBlockCommitment, wb: WriteBatch) -> DbResult<()> {
        self.config
            .with_retry((&self.write_batch_tree,), |(wb_tree,)| {
                wb_tree.insert(&commitment, &wb)?;
                Ok(())
            })?;
        Ok(())
    }

    fn get_ol_write_batch(&self, commitment: OLBlockCommitment) -> DbResult<Option<WriteBatch>> {
        self.write_batch_tree
            .get(&commitment)
            .map_err(conv_sled_err)
    }

    fn del_ol_write_batch(&self, commitment: OLBlockCommitment) -> DbResult<()> {
        self.write_batch_tree
            .remove(&commitment)
            .map_err(conv_sled_err)?;
        Ok(())
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use ssz::Encode;
    use strata_db_tests::ol_state_db_tests;
    use strata_identifiers::{Buf32, OLBlockId};
    use strata_ol_state_container::test_utils::create_test_container_with_staged;
    use strata_ol_state_types_v1::test_utils::create_test_genesis_state;
    use typed_sled::codec::{KeyCodec, ValueCodec};
    use typed_sled::schema::Schema;

    use super::*;
    use crate::{SledDbConfig, sled_db_test_setup};

    sled_db_test_setup!(OLStateDBSled, ol_state_db_tests);

    fn open_at(path: &Path) -> (sled::Db, OLStateDBSled) {
        let db = sled::Config::new().path(path).open().unwrap();
        let sled_db = typed_sled::SledDb::new(db.clone()).unwrap();
        let state_db = OLStateDBSled::new(Arc::new(sled_db), SledDbConfig::test()).unwrap();
        (db, state_db)
    }

    fn test_commitment() -> OLBlockCommitment {
        OLBlockCommitment::new(5, OLBlockId::from(Buf32::from([9; 32])))
    }

    #[test]
    fn test_cold_reopen_reproduces_versions_and_root() {
        let dir = tempfile::tempdir().unwrap();
        let state = create_test_container_with_staged(2);

        {
            let (db, state_db) = open_at(dir.path());
            state_db
                .put_toplevel_ol_state(test_commitment(), state.clone())
                .unwrap();
            db.flush().unwrap();
        }

        let (_db, state_db) = open_at(dir.path());
        let restored = state_db
            .get_toplevel_ol_state(test_commitment())
            .unwrap()
            .expect("test: stored snapshot");
        assert_eq!(restored.staged_spec_version(), 2);
        assert_eq!(restored.compute_state_root(), state.compute_state_root());
        assert_eq!(restored, state);
    }

    #[test]
    fn test_rejects_legacy_and_corrupt_snapshots() {
        let dir = tempfile::tempdir().unwrap();
        let (db, state_db) = open_at(dir.path());
        let key =
            <OLBlockCommitment as KeyCodec<OLStateSchema>>::encode_key(&test_commitment()).unwrap();
        let tree = db.open_tree(OLStateSchema::TREE_NAME.0).unwrap();

        // Snapshots written before the root existed are bare `OLStateV1` SSZ.
        let legacy = create_test_genesis_state().as_ssz_bytes();
        // The chainstate bytes end the encoding, so flipping the last byte
        // leaves a chainstate that no longer matches its committed root.
        let mut corrupt = create_test_container_with_staged(1).encode_value().unwrap();
        *corrupt.last_mut().unwrap() ^= 0xff;

        for (label, value) in [("legacy snapshot", legacy), ("corrupt chainstate", corrupt)] {
            tree.insert(&key, value).unwrap();
            assert!(
                state_db.get_toplevel_ol_state(test_commitment()).is_err(),
                "{label} must not decode"
            );
        }
    }
}
