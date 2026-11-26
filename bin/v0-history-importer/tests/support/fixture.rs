use sha2::{Digest, Sha256};
use strata_db_store_sled::{SLED_NAME, SledDbConfig, open_sled_backend};
use strata_db_types::{
    backend::DatabaseBackend, ol_block::OLBlockDatabase, ol_state::OLStateDatabase,
};
use strata_identifiers::{AccountId, Buf32, Buf64, OLBlockId, Slot};
use strata_ol_chain_types::{
    BlockFlags, OLBlock, OLBlockBody, OLBlockHeader, OLTransaction, OLTransactionData, OLTxSegment,
    SauTxLedgerRefs, SauTxOperationData, SauTxPayload, SauTxProofState, SauTxUpdateData,
    SignedOLBlockHeader, TransactionPayload, TxProofs,
};
use strata_ol_state_types::{OLAccountState, OLAccountTypeState, OLSnarkAccountState, WriteBatch};
use strata_predicate::PredicateKey;
use tempfile::{TempDir, tempdir};
pub(crate) fn block(
    slot: Slot,
    parent: OLBlockId,
    txs: Vec<OLTransaction>,
    terminal: bool,
) -> OLBlock {
    let body = OLBlockBody::new_common(OLTxSegment::new(txs).unwrap());
    let mut flags = BlockFlags::zero();
    flags.set_is_terminal(terminal);
    let header = OLBlockHeader::new(
        slot,
        flags,
        slot,
        u32::from(slot > 0),
        parent,
        body.compute_hash_commitment(),
        Buf32::zero(),
        Buf32::zero(),
    );
    let signature = if slot == 0 {
        Buf64::zero()
    } else {
        Buf64::from([0xa5; 64])
    };
    OLBlock::new(SignedOLBlockHeader::new(header, signature), body)
}

pub(crate) struct Fixture {
    pub(crate) dir: TempDir,
    pub(crate) genesis: OLBlockId,
    pub(crate) middle: OLBlockId,
    pub(crate) terminal: OLBlockId,
    pub(crate) txids_digest: String,
}

impl Fixture {
    pub(crate) fn new() -> Self {
        let dir = tempdir().unwrap();
        let db = open_sled_backend(dir.path(), SLED_NAME, SledDbConfig::test()).unwrap();
        let target = AccountId::new([1; 32]);
        let update = SauTxOperationData::new(
            SauTxUpdateData::new(1, SauTxProofState::new(0, Buf32::zero()), vec![7]),
            vec![],
            SauTxLedgerRefs::new_empty(),
        );
        let txs = vec![
            OLTransaction::new(
                OLTransactionData::new(
                    TransactionPayload::SnarkAccountUpdate(SauTxPayload::new(target, update)),
                    Default::default(),
                ),
                TxProofs::new_empty(),
            ),
            OLTransaction::new(
                OLTransactionData::from_gam_bytes(target, vec![4]).unwrap(),
                TxProofs::new_empty(),
            ),
        ];
        let mut digest = Sha256::new();
        for tx in &txs {
            digest.update(tx.compute_txid().as_ref());
        }
        let genesis = block(0, OLBlockId::null(), vec![], true);
        let genesis_id = genesis.header().compute_blkid();
        let middle = block(1, genesis_id, txs, false);
        let middle_id = middle.header().compute_blkid();
        let sibling = block(1, genesis_id, vec![], false);
        let terminal = block(2, middle_id, vec![], true);
        let terminal_id = terminal.header().compute_blkid();

        let mut batch = WriteBatch::default();
        let serial = 42u32.into();
        let state =
            OLAccountState::new(serial, 10u64.try_into().unwrap(), OLAccountTypeState::Empty);
        batch.ledger_mut().create_account_raw(target, state, serial);
        let updated = OLAccountState::new(
            7u32.into(),
            20u64.try_into().unwrap(),
            OLAccountTypeState::Snark(OLSnarkAccountState::new_fresh(
                PredicateKey::always_accept(),
                [3; 32].into(),
            )),
        );
        batch
            .ledger_mut()
            .update_account(AccountId::new([2; 32]), updated);
        for (block, batch) in [
            (&middle, batch),
            (&sibling, WriteBatch::default()),
            (&terminal, WriteBatch::default()),
        ] {
            db.ol_state_db()
                .put_ol_write_batch(block.header().compute_block_commitment(), batch)
                .unwrap();
        }
        for block in [genesis, sibling, middle, terminal] {
            db.ol_block_db().put_block_data(block).unwrap();
        }
        Self {
            dir,
            genesis: genesis_id,
            middle: middle_id,
            terminal: terminal_id,
            txids_digest: hex::encode(digest.finalize()),
        }
    }
}
