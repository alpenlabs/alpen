//! Cross-release test using the v0.3.5 importer's three-block fixture.
//! Prepare a fresh promoted node, run the importer against it, then check the registered RPCs.
use std::{env, fs, path::PathBuf, sync::Arc};

use jsonrpsee::rpc_params;
use serde_json::Value;
use strata_db_store_sled::{
    SLED_NAME, SledDbConfig, init_core_dbs, open_sled_database_with_handle,
};
use strata_db_types::{backend::DatabaseBackend, ol_block::OLBlockDatabase};
use strata_identifiers::{EpochCommitment, OLBlockId};
use strata_storage::{create_node_storage, test_runtime_handle};

use super::RpcFixture;
use crate::{
    rpc::{build_public_rpc_module, history::HistoryArchive},
    run_context::NodeRole,
};

fn fixture() -> (PathBuf, Vec<Value>, OLBlockId, EpochCommitment) {
    let path =
        PathBuf::from(env::var_os("STRATA_V0_FIXTURE_OUT").expect("path to importer fixture"));
    let rows: Vec<Value> =
        serde_json::from_slice(&fs::read(path.join("responses.json")).unwrap()).unwrap();
    let genesis = serde_json::from_value(rows[0]["header"]["blkid"].clone()).unwrap();
    let terminal = serde_json::from_value(rows[2]["header"]["blkid"].clone()).unwrap();
    (
        path.join("promoted"),
        rows,
        genesis,
        EpochCommitment::new(1, 2, terminal),
    )
}

#[test]
#[ignore = "reads the promoted database after the v0.3.5 importer has exited"]
fn explorer_reads_imported_database() {
    let (path, source, genesis, expected) = fixture();
    let (typed, raw) = open_sled_database_with_handle(&path, SLED_NAME).unwrap();
    let backend = init_core_dbs(typed, SledDbConfig::test()).unwrap();
    let base = backend.ol_block_db().get_history_base().unwrap().unwrap();
    assert_eq!(base, expected);
    let mut deps = RpcFixture::new().make_deps(NodeRole::Sequencer);
    deps.storage = Arc::new(create_node_storage(backend, test_runtime_handle()).unwrap());
    deps.history_archive = HistoryArchive::discover(&raw, || Ok((genesis, base))).unwrap();
    let rpc = build_public_rpc_module(&deps).unwrap();
    test_runtime_handle().block_on(async {
        let headers: Vec<Value> = rpc
            .call("strata_getHeadersInRange", rpc_params![0, 4])
            .await
            .unwrap();
        let blocks: Vec<Value> = rpc
            .call("strata_getRawBlocksRange", rpc_params![0, 4])
            .await
            .unwrap();
        assert_eq!((headers.len(), blocks.len()), (5, 5));
        for (slot, row) in source.iter().enumerate() {
            assert_eq!(headers[slot], row["header"]);
            assert_eq!(blocks[slot], row["block"]);
            let raw: Value = rpc
                .call(
                    "strata_getRawBlockById",
                    rpc_params![&row["header"]["blkid"]],
                )
                .await
                .unwrap();
            assert_eq!(raw, row["block"]["raw_block"]);
            let txs: Value = rpc
                .call("strata_getBlockTransactions", rpc_params![slot])
                .await
                .unwrap();
            assert_eq!(txs, row["transactions"]);
            let changes = rpc
                .call::<_, Value>("strata_getBlockAccountChanges", rpc_params![slot])
                .await;
            if slot == 0 {
                assert!(
                    changes
                        .unwrap_err()
                        .to_string()
                        .contains("No OL write batch found for block 0")
                );
            } else {
                assert_eq!(changes.unwrap(), row["changes"]);
            }
        }
        for slot in [3, 4] {
            assert_eq!(headers[slot]["parent_blkid"], headers[slot - 1]["blkid"]);
            assert_eq!(blocks[slot]["blkid"], headers[slot]["blkid"]);
        }
    });
}
