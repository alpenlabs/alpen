use std::collections::HashMap;

use anyhow::Error;
use bitcoin::{
    block::Header,
    consensus::{deserialize, serialize},
    hashes::Hash,
    Block, BlockHash, Network, Txid,
};
use bitcoind_async_client::{
    error::ClientError,
    traits::Reader,
    types::{
        GetBlockchainInfo, GetRawTransactionVerbosityOne, GetRawTransactionVerbosityZero, GetTxOut,
    },
    ClientResult,
};
use strata_btcio::reader::query::fetch_verification_state;
use strata_primitives::{
    buf::Buf32,
    l1::{HeaderVerificationState, L1BlockManifest, L1HeaderRecord},
};

#[derive(Debug)]
pub struct BtcChainSegment {
    pub headers: Vec<Header>,
    pub start: u64,
    pub end: u64,
    pub custom_blocks: HashMap<u64, Block>,
    pub custom_headers: HashMap<u64, Header>,
    pub idx_by_blockhash: HashMap<BlockHash, usize>,
}

impl BtcChainSegment {
    pub fn load_full_block() -> Block {
        let raw_block = include_bytes!(
        "../data/mainnet_block_000000000000000000000c835b2adcaedc20fdf6ee440009c249452c726dafae.raw"
    );
        let block: Block = deserialize(&raw_block[..]).unwrap();
        block
    }

    pub fn load() -> BtcChainSegment {
        let raw_headers = include_bytes!("../data/mainnet_blocks_40000-50000.raw");

        let chunk_size = Header::SIZE;
        let capacity = raw_headers.len() / chunk_size;
        let mut headers = Vec::with_capacity(capacity);

        for chunk in raw_headers.chunks(chunk_size) {
            let raw_header = chunk.to_vec();
            let header: Header = deserialize(&raw_header).unwrap();
            headers.push(header);
        }

        let custom_headers: HashMap<u64, Header> = vec![(38304, "01000000858a5c6d458833aa83f7b7e56d71c604cb71165ebb8104b82f64de8d00000000e408c11029b5fdbb92ea0eeb8dfa138ffa3acce0f69d7deebeb1400c85042e01723f6b4bc38c001d09bd8bd5")].into_iter().map(|(h, raw_block)| {
            let header_bytes = hex::decode(raw_block).unwrap();
            let header: Header = bitcoin::consensus::deserialize(&header_bytes).unwrap();
            (h, header)
        })
        .collect();

        let idx_by_blockhash = headers
            .iter()
            .enumerate()
            .map(|(idx, header)| (header.block_hash(), idx))
            .collect::<HashMap<BlockHash, usize>>();

        // This custom blocks are chose because this is where the first difficulty happened
        let custom_blocks: HashMap<u64, Block> = vec![
        (40320, "010000001a231097b6ab6279c80f24674a2c8ee5b9a848e1d45715ad89b6358100000000a822bafe6ed8600e3ffce6d61d10df1927eafe9bbf677cb44c4d209f143c6ba8db8c784b5746651cce2221180101000000010000000000000000000000000000000000000000000000000000000000000000ffffffff08045746651c02db02ffffffff0100f2052a010000004341046477f88505bef7e3c1181a7e3975c4cd2ac77ffe23ea9b28162afbb63bd71d3f7c3a07b58cf637f1ec68ed532d5b6112d57a9744010aae100e4a48cd831123b8ac00000000"),
        (40321, "0100000045720d24eae33ade0d10397a2e02989edef834701b965a9b161e864500000000993239a44a83d5c427fd3d7902789ea1a4d66a37d5848c7477a7cf47c2b071cd7690784b5746651c3af7ca030101000000010000000000000000000000000000000000000000000000000000000000000000ffffffff08045746651c02db00ffffffff0100f2052a01000000434104c9f513361104db6a84fb6d5b364ba57a27cd19bd051239bf750d8999c6b437220df8fea6b932a248df3cad1fdebb501791e02b7b893a44718d696542ba92a0acac00000000"),
        (40322, "01000000fd1133cd53d00919b0bd77dd6ca512c4d552a0777cc716c00d64c60d0000000014cf92c7edbe8a75d1e328b4fec0d6143764ecbd0f5600aba9d22116bf165058e590784b5746651c1623dbe00101000000010000000000000000000000000000000000000000000000000000000000000000ffffffff08045746651c020509ffffffff0100f2052a010000004341043eb751f57bd4839a8f2922d5bf1ed15ade9b161774658fb39801f0b9da9c881f226fbe4ee0c240915f17ce5255dd499075ab49b199a7b1f898fb20cc735bc45bac00000000"),
        (40323, "01000000c579e586b48485b6e263b54949d07dce8660316163d915a35e44eb570000000011d2b66f9794f17393bf90237f402918b61748f41f9b5a2523c482a81a44db1f4f91784b5746651c284557020101000000010000000000000000000000000000000000000000000000000000000000000000ffffffff08045746651c024502ffffffff0100f2052a01000000434104597b934f2081e7f0d7fae03ec668a9c69a090f05d4ee7c65b804390d94266ffb90442a1889aaf78b460692a43857638520baa8319cf349b0d5f086dc4d36da8eac00000000"),
        (40324, "010000001f35c6ea4a54eb0ea718a9e2e9badc3383d6598ff9b6f8acfd80e52500000000a7a6fbce300cbb5c0920164d34c36d2a8bb94586e9889749962b1be9a02bbf3b9194784b5746651c0558e1140101000000010000000000000000000000000000000000000000000000000000000000000000ffffffff08045746651c029001ffffffff0100f2052a01000000434104e5d390c21b7d221e6ba15c518444c1aae43d6fb6f721c4a5f71e590288637ca2961be07ee845a795da3fd1204f52d4faa819c167062782590f08cf717475e488ac00000000"),
        ]
        .into_iter()
        .map(|(h, raw_block)| {
            let block_bytes = hex::decode(raw_block).unwrap();
            let block: Block = bitcoin::consensus::deserialize(&block_bytes).unwrap();
            (h, block)
        })
        .collect();

        BtcChainSegment {
            headers,
            start: 40_000,
            end: 50_000,
            custom_blocks,
            custom_headers,
            idx_by_blockhash,
        }
    }
}

impl BtcChainSegment {
    /// Retrieve a block at a given height.
    pub fn get_block_at(&self, height: u64) -> ClientResult<Block> {
        if let Some(block) = self.custom_blocks.get(&height) {
            Ok(block.clone())
        } else {
            Err(ClientError::Body(format!(
                "Block at height {height} not available"
            )))
        }
    }

    /// Retrieve a block at a given height.
    pub fn get_block_header_at(&self, height: u64) -> ClientResult<Header> {
        if let Some(header) = self.custom_headers.get(&height) {
            return Ok(*header);
        }

        if !(self.start..self.end).contains(&height) {
            return Err(ClientError::Body(format!(
                "Block header at height {height} not available"
            )));
        }
        let idx = height - self.start;
        Ok(self.headers[idx as usize])
    }

    /// Retrieve a block at a given height.
    pub fn get_block_header(&self, blockhash: &BlockHash) -> ClientResult<Header> {
        let Some(idx) = self.idx_by_blockhash.get(blockhash) else {
            return Err(ClientError::Body(format!(
                "Block header for blockhash {} not available",
                *blockhash
            )));
        };
        Ok(self.headers[*idx])
    }

    pub fn get_block_manifest(&self, height: u64) -> L1BlockManifest {
        let rec = self.get_header_record(height).unwrap();
        let header_vs = self.get_verification_state(height, 0).unwrap();
        L1BlockManifest::new(rec, Some(header_vs), Vec::new(), 1, height)
    }
}

/// Implement the [`Reader`] trait for our chain segment.
impl Reader for BtcChainSegment {
    /// Return a default fee estimate.
    async fn estimate_smart_fee(&self, _conf_target: u16) -> ClientResult<u64> {
        // Return a default fee (e.g., 1000 satoshis per kB)
        Ok(1000)
    }

    /// Look up a block by its hash in our custom blocks.
    async fn get_block(&self, hash: &BlockHash) -> ClientResult<Block> {
        // Search our custom_blocks for a block matching the given hash.
        for block in self.custom_blocks.values() {
            if &block.block_hash() == hash {
                return Ok(block.clone());
            }
        }
        Err(ClientError::Body(format!(
            "Block with hash {hash:?} not found"
        )))
    }

    async fn get_block_header(&self, _hash: &BlockHash) -> ClientResult<Header> {
        unimplemented!()
    }

    /// Return the block height corresponding to the given block hash.
    async fn get_block_height(&self, hash: &BlockHash) -> ClientResult<u64> {
        for (height, block) in &self.custom_blocks {
            if &block.block_hash() == hash {
                return Ok(*height);
            }
        }
        Err(ClientError::Body(format!(
            "Block with hash {hash:?} not found"
        )))
    }

    /// Retrieve a block at a given height.
    async fn get_block_at(&self, height: u64) -> ClientResult<Block> {
        self.get_block_at(height)
    }

    /// Retrieve a block at a given height.
    async fn get_block_header_at(&self, height: u64) -> ClientResult<Header> {
        self.get_block_header_at(height)
    }

    /// Return the height of the best (most-work) block.
    async fn get_block_count(&self) -> ClientResult<u64> {
        // In this segment, we assume the tip is at `end - 1`.
        Ok(self.end - 1)
    }

    /// Retrieve the block hash for the block at the given height.
    async fn get_block_hash(&self, height: u64) -> ClientResult<BlockHash> {
        let header = self.get_block_header_at(height)?;
        Ok(header.block_hash())
    }

    /// Return some blockchain info using default values.
    async fn get_blockchain_info(&self) -> ClientResult<GetBlockchainInfo> {
        unimplemented!()
    }

    /// Return the timestamp of the current best block.
    async fn get_current_timestamp(&self) -> ClientResult<u32> {
        unimplemented!()
    }

    /// Return an empty mempool.
    async fn get_raw_mempool(&self) -> ClientResult<Vec<Txid>> {
        // For our in-memory segment, we assume there are no unconfirmed transactions.
        Ok(vec![])
    }

    /// Gets a raw transaction by its [`Txid`].
    async fn get_raw_transaction_verbosity_zero(
        &self,
        _txid: &Txid,
    ) -> ClientResult<GetRawTransactionVerbosityZero> {
        unimplemented!()
    }

    /// Gets a raw transaction by its [`Txid`].
    async fn get_raw_transaction_verbosity_one(
        &self,
        _txid: &Txid,
    ) -> ClientResult<GetRawTransactionVerbosityOne> {
        unimplemented!()
    }

    /// Return an error as this functionality is not implemented.
    async fn get_tx_out(
        &self,
        _txid: &Txid,
        _vout: u32,
        _include_mempool: bool,
    ) -> ClientResult<GetTxOut> {
        unimplemented!()
    }

    /// Return the underlying network (mainnet).
    async fn network(&self) -> ClientResult<Network> {
        Ok(Network::Bitcoin)
    }
}

impl BtcChainSegment {
    pub fn get_block_manifest_by_blockhash(
        &self,
        blockhash: &BlockHash,
    ) -> Result<L1BlockManifest, Error> {
        let Some(idx) = self.idx_by_blockhash.get(blockhash) else {
            return Err(ClientError::Body(format!(
                "Block header for blockhash {} not available",
                *blockhash
            )))?;
        };
        let height = self.start + *idx as u64;

        let header = self.headers[*idx];
        let header_record = L1HeaderRecord::new(
            header.block_hash().into(),
            serialize(&header),
            Buf32::from(header.merkle_root.as_raw_hash().to_byte_array()),
        );
        let header_vs = self.get_verification_state(height, 0).unwrap();
        Ok(L1BlockManifest::new(
            header_record,
            Some(header_vs),
            Vec::new(),
            0,
            height,
        ))
    }

    pub fn get_block_manifests(
        &self,
        from_height: u64,
        len: usize,
    ) -> Result<Vec<L1BlockManifest>, Error> {
        let mut manifests = Vec::with_capacity(len);
        for i in 0..len {
            let height = from_height + i as u64;
            let manifest = self.get_block_manifest(height);
            manifests.push(manifest);
        }
        Ok(manifests)
    }

    pub fn get_header_record(&self, height: u64) -> Result<L1HeaderRecord, Error> {
        let header = self.get_block_header_at(height)?;
        Ok(L1HeaderRecord::new(
            header.block_hash().into(),
            serialize(&header),
            Buf32::from(header.merkle_root.as_raw_hash().to_byte_array()),
        ))
    }

    pub fn get_header_records(
        &self,
        from_height: u64,
        len: usize,
    ) -> Result<Vec<L1HeaderRecord>, Error> {
        let mut blocks = Vec::with_capacity(len);
        for i in 0..len {
            let block = self.get_header_record(from_height + i as u64)?;
            blocks.push(block);
        }
        Ok(blocks)
    }

    pub fn get_blocks(&self, from_height: u64, len: usize) -> Result<Vec<Block>, Error> {
        let mut blocks = Vec::with_capacity(len);
        for i in 0..len {
            let block = self.get_block_at(from_height + i as u64)?;
            blocks.push(block);
        }
        Ok(blocks)
    }

    pub fn get_verification_state(
        &self,
        height: u64,
        l1_reorg_safe_depth: u32,
    ) -> Result<HeaderVerificationState, Error> {
        block_on(fetch_verification_state(self, height, l1_reorg_safe_depth))
    }
}

/// If we're already in a tokio runtime, we'll block in place. Otherwise, we'll create a new
/// runtime.
pub fn block_on<T>(fut: impl std::future::Future<Output = T>) -> T {
    use tokio::task::block_in_place;

    // Handle case if we're already in an tokio runtime.
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        block_in_place(|| handle.block_on(fut))
    } else {
        // Otherwise create a new runtime.
        let rt = tokio::runtime::Runtime::new().expect("Failed to create a new runtime");
        rt.block_on(fut)
    }
}
