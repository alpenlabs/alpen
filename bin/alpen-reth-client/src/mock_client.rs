use std::{collections::HashMap, hash::Hash, sync::Arc};

use alloy_primitives::FixedBytes;
use async_trait::async_trait;
use eyre::eyre;
use tokio::sync::RwLock;

use crate::{
    traits::{ELSequencerClient, L1Client, OlClient},
    types::{
        AccountId, AccountStateCommitment, EEAccount, L1BlockCommitment, L1BlockId,
        OlBlockCommitment, OlBlockId,
    },
};

#[derive(Debug, Clone)]
struct MockL1Block {
    commitment: L1BlockCommitment,
}

impl MockL1Block {
    fn new_block(height: u64) -> Self {
        let mut hash_bytes = [0u8; 32];
        hash_bytes[0] = 1;
        hash_bytes[24..].copy_from_slice(&height.to_le_bytes());
        let blockid = FixedBytes::from_slice(&hash_bytes).into();
        let commitment = L1BlockCommitment::new(blockid, height);
        Self { commitment }
    }
}

#[derive(Debug, Clone)]
struct AccountState {
    hash: AccountStateCommitment,
}

#[derive(Debug, Clone)]
struct MockOlBlock {
    commitment: OlBlockCommitment,
    account: AccountState,
}

impl MockOlBlock {
    fn new_block(
        slot: u64,
        account_commitment: AccountStateCommitment,
        l1_commitment: L1BlockCommitment,
    ) -> Self {
        let mut hash_bytes = [0u8; 32];
        hash_bytes[0] = 2;
        hash_bytes[24..].copy_from_slice(&slot.to_le_bytes());
        let blockhash = FixedBytes::from_slice(&hash_bytes).into();

        Self {
            commitment: OlBlockCommitment::new(blockhash, slot, l1_commitment),
            account: AccountState {
                hash: account_commitment,
            },
        }
    }
}

trait MockBlock {
    type ID: Clone + PartialEq + Eq + Hash;

    fn hash(&self) -> &Self::ID;
    fn height(&self) -> u64;
}

impl MockBlock for MockL1Block {
    type ID = L1BlockId;

    fn hash(&self) -> &L1BlockId {
        self.commitment.blockhash()
    }

    fn height(&self) -> u64 {
        self.commitment.height()
    }
}

impl MockBlock for MockOlBlock {
    type ID = OlBlockId;

    fn hash(&self) -> &OlBlockId {
        self.commitment.blockhash()
    }

    fn height(&self) -> u64 {
        self.commitment.slot()
    }
}

#[derive(Debug, Clone)]
struct MockChain<Block: MockBlock> {
    by_blockhash: HashMap<Block::ID, Block>,
    by_height: HashMap<u64, Block::ID>,
    tip_height: u64,
}

impl<Block: MockBlock + Clone> FromIterator<Block> for MockChain<Block> {
    fn from_iter<T: IntoIterator<Item = Block>>(iter: T) -> Self {
        let mut by_blockhash = HashMap::new();
        let mut by_height = HashMap::new();

        let mut max_height: Option<u64> = None;

        for block in iter {
            let height = block.height();
            max_height = max_height.map(|max| max.max(height)).or(Some(height));
            by_height.insert(block.height(), block.hash().clone());
            by_blockhash.insert(block.hash().clone(), block);
        }

        let tip_height = max_height.expect("blocks must exist in chain");

        Self {
            by_blockhash,
            by_height,
            tip_height,
        }
    }
}

impl<Block: MockBlock + Clone> MockChain<Block> {
    fn insert_new_block(&mut self, block: Block) -> eyre::Result<()> {
        if block.height() != self.tip_height + 1 {
            return Err(eyre::eyre!("block does not extend chain"));
        }

        self.by_blockhash
            .insert(block.hash().clone(), block.clone());
        self.by_height.insert(block.height(), block.hash().clone());
        self.tip_height += 1;

        Ok(())
    }

    fn tip(&self) -> &Block {
        let id = self.by_height.get(&self.tip_height).expect("should exist");

        self.by_blockhash.get(id).expect("should exist")
    }
}

#[derive(Debug, Clone)]
pub(crate) struct MockClientInner {
    l1_chain: MockChain<MockL1Block>,
    ol_chain: MockChain<MockOlBlock>,
    latest_account_commitment: AccountStateCommitment,
    commitment_counter: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct MockClient {
    inner: Arc<RwLock<MockClientInner>>,
}

impl MockClient {
    pub(crate) async fn set_latest_account_commitment(&self, commitment: AccountStateCommitment) {
        let mut state = self.inner.write().await;
        state.latest_account_commitment = commitment;
        state.commitment_counter += 1;

        if state.commitment_counter % 8 == 0 {
            // create new l1 block
            let block = MockL1Block::new_block(state.l1_chain.tip_height + 1);
            state.l1_chain.insert_new_block(block).unwrap();
        }

        if state.commitment_counter % 4 == 0 {
            // create new OL block
            let block = MockOlBlock::new_block(
                state.ol_chain.tip_height + 1,
                state.latest_account_commitment.clone(),
                state.l1_chain.tip().commitment.clone(),
            );

            state.ol_chain.insert_new_block(block).unwrap()
        }
    }
}

#[async_trait]
impl OlClient for MockClient {
    async fn account_state_commitment_at(
        &self,
        _account_id: &AccountId,
        blockid: &OlBlockId,
    ) -> eyre::Result<AccountStateCommitment> {
        let state = self.inner.read().await;
        Ok(state
            .ol_chain
            .by_blockhash
            .get(blockid)
            .unwrap()
            .account
            .hash
            .clone())
    }

    async fn best_ol_block(&self) -> eyre::Result<OlBlockCommitment> {
        let state = self.inner.read().await;
        Ok(state.ol_chain.tip().commitment.clone())
    }

    async fn ol_block_for_l1(
        &self,
        l1block: &L1BlockCommitment,
    ) -> eyre::Result<OlBlockCommitment> {
        let state = self.inner.read().await;

        state
            .ol_chain
            .by_blockhash
            .iter()
            .find(|(_, block)| block.commitment.l1_commitment() == l1block)
            .map(|(_, block)| block.commitment.clone())
            .ok_or(eyre::eyre!("missing block"))
    }

    async fn submit_account_update(
        &self,
        _account_id: &AccountId,
        update: &EEAccount,
    ) -> eyre::Result<()> {
        let mut state = self.inner.write().await;

        let new_l1 = MockL1Block::new_block(state.l1_chain.tip_height + 1);
        let new_ol = MockOlBlock::new_block(
            state.ol_chain.tip_height + 1,
            update.state_commitment.clone(),
            new_l1.commitment.clone(),
        );

        state.l1_chain.insert_new_block(new_l1)?;
        state.ol_chain.insert_new_block(new_ol)?;

        Ok(())
    }
}

#[async_trait]
impl L1Client for MockClient {
    async fn get_l1_commitment(&self, blockhash: L1BlockId) -> eyre::Result<L1BlockCommitment> {
        let state = self.inner.read().await;
        state
            .l1_chain
            .by_blockhash
            .get(&blockhash)
            .map(|block| block.commitment.clone())
            .ok_or(eyre::eyre!("unknown blockhash"))
    }

    async fn get_l1_commitment_by_height(&self, height: u64) -> eyre::Result<L1BlockCommitment> {
        let state = self.inner.read().await;

        let blockhash = state
            .l1_chain
            .by_height
            .get(&height)
            .ok_or(eyre!("unknown block height"))?;

        state
            .l1_chain
            .by_blockhash
            .get(&blockhash)
            .map(|block| block.commitment.clone())
            .ok_or(eyre::eyre!("unknown blockhash"))
    }
}

#[async_trait]
impl ELSequencerClient for MockClient {
    async fn get_latest_state_commitment(&self) -> eyre::Result<AccountStateCommitment> {
        let state = self.inner.read().await;

        Ok(state.latest_account_commitment.clone())
    }
}

pub(crate) fn get_mocked_client() -> MockClient {
    let l1_gen_block = MockL1Block::new_block(0);
    let l1_commitment = l1_gen_block.commitment.clone();

    let l1_chain = MockChain::<MockL1Block>::from_iter(vec![l1_gen_block].into_iter());
    let ol_gen_block = MockOlBlock::new_block(0, AccountStateCommitment::zero(), l1_commitment);
    let ol_chain = MockChain::<MockOlBlock>::from_iter(vec![ol_gen_block].into_iter());

    MockClient {
        inner: Arc::new(RwLock::new(MockClientInner {
            l1_chain,
            ol_chain,
            latest_account_commitment: AccountStateCommitment::zero(),
            commitment_counter: 0,
        })),
    }
}
