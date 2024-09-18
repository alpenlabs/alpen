use alpen_express_primitives::{buf::Buf32, l1::L1BlockManifest};
use bitcoin::{
    block::Header, consensus::deserialize, hashes::Hash, params::Params, Block, Transaction,
};

use crate::ArbitraryGenerator;

pub fn get_test_bitcoin_txns() -> Vec<Transaction> {
    let t1 = "0200000000010176f29f18c5fc677ad6dd6c9309f6b9112f83cb95889af21da4be7fbfe22d1d220000000000fdffffff0300e1f505000000002200203946555814a18ccc94ef4991fb6af45278425e6a0a2cfc2bf4cf9c47515c56ff0000000000000000176a1500e0e78c8201d91f362c2ad3bb6f8e6f31349454663b1010240100000022512012d77c9ae5fdca5a3ab0b17a29b683fd2690f5ad56f6057a000ec42081ac89dc0247304402205de15fbfb413505a3563608dad6a73eb271b4006a4156eeb62d1eacca5efa10b02201eb71b975304f3cbdc664c6dd1c07b93ac826603309b3258cb92cfd201bb8792012102f55f96fd587a706a7b5e7312c4e9d755a65b3dad9945d65598bca34c9e961db400000000";
    let t2 = "02000000000101f4f2e8830d2948b5e980e739e61b23f048d03d4af81588bf5da4618406c495aa0000000000fdffffff02969e0700000000002200203946555814a18ccc94ef4991fb6af45278425e6a0a2cfc2bf4cf9c47515c56ff60f59000000000001600148d0499ec043b1921a608d24690b061196e57c927040047304402203875f7b610f8783d5f5c163118eeec1a23473dd33b53c8ea584c7d28a82b209b022034b6814344b79826a348e23cc19ff06ed2df23850b889557552e376bf9e32c560147304402200f647dad3c137ff98d7da7a302345c82a57116a3d0e6a3719293bbb421cb0abe02201c04a1e808f5bab3595f77985af91aeaf61e9e042c9ac97d696e0f4b020cb54b0169522102dba8352965522ff44538dde37d793b3b4ece54e07759ade5f648aa396165d2962103c0683712773b725e7fe4809cbc90c9e0b890c45e5e24a852a4c472d1b6e9fd482103bf56f172d0631a7f8ae3ef648ad43a816ad01de4137ba89ebc33a2da8c48531553ae00000000";
    let t3 = "02000000000101f4f2e8830d2948b5e980e739e61b23f048d03d4af81588bf5da4618406c495aa0200000000ffffffff0380969800000000002200203946555814a18ccc94ef4991fb6af45278425e6a0a2cfc2bf4cf9c47515c56ff0000000000000000176a15006e1a916a60b93a545f2370f2a36d2f807fb3d675588b693a000000001600149fafc79c72d1c4d917a360f32bdc68755402ef670247304402203c813ad8918366ce872642368b57b78e78e03b1a1eafe16ec8f3c9268b4fc050022018affe880963f18bfc0338f1e54c970185aa90f8c36a52ac935fe76cb885d726012102fa9b81d082a98a46d0857d62e6c9afe9e1bf40f9f0cbf361b96241c9d6fb064b00000000";
    let t4 = "02000000000101d8acf0a647b7d5d1d0ee83360158d5bf01146d3762c442defd7985476b02aa6b0100000000fdffffff030065cd1d000000002200203946555814a18ccc94ef4991fb6af45278425e6a0a2cfc2bf4cf9c47515c56ff0000000000000000176a1500e0e78c8201d91f362c2ad3bb6f8e6f3134945466aec19dd00000000022512040718748dbca6dea8ac6b6f0b177014f0826478f1613c2b489e738db7ecdf3610247304402207cfc5cd87ec83687c9ac2bd921e96b8a58710f15d77bc7624da4fb29fe589dab0220437b74ed8e8f9d3084269edfb8641bf27246b0e5476667918beba73025c7a2c501210249a34cfbb6163b1b6ca2fff63fd1f8a802fb1999fa7930b2febe5a711f713dd900000000";
    let t5 = "0200000000010176f29f18c5fc677ad6dd6c9309f6b9112f83cb95889af21da4be7fbfe22d1d220000000000fdffffff0300e1f505000000002200203946555814a18ccc94ef4991fb6af45278425e6a0a2cfc2bf4cf9c47515c56ff0000000000000000176a1500e0e78c8201d91f362c2ad3bb6f8e6f31349454663b1010240100000022512012d77c9ae5fdca5a3ab0b17a29b683fd2690f5ad56f6057a000ec42081ac89dc0247304402205de15fbfb413505a3563608dad6a73eb271b4006a4156eeb62d1eacca5efa10b02201eb71b975304f3cbdc664c6dd1c07b93ac826603309b3258cb92cfd201bb8792012102f55f96fd587a706a7b5e7312c4e9d755a65b3dad9945d65598bca34c9e961db400000000";
    [t1, t2, t3, t4, t5]
        .iter()
        .map(|x| deserialize(&hex::decode(x).unwrap()).unwrap())
        .collect()
}

pub fn gen_l1_chain(len: usize) -> Vec<L1BlockManifest> {
    // FIXME this is bad, the blocks generated are nonsensical
    let mut blocks = vec![];
    for _ in 0..len {
        let block: L1BlockManifest = ArbitraryGenerator::new().generate();
        blocks.push(block);
    }
    blocks
}

pub fn get_btc_mainnet_block() -> Block {
    let raw_block = include_bytes!(
        "../data/mainnet_block_000000000000000000000c835b2adcaedc20fdf6ee440009c249452c726dafae.raw"
    );
    let block: Block = deserialize(&raw_block[..]).unwrap();
    block
}

/// Calculates the height at which a specific difficulty adjustment occurs relative to a
/// starting height.
///
/// # Arguments
///
/// * `idx` - The index of the difficulty adjustment (1-based). 1 for the first adjustment, 2 for
///   the second, and so on.
/// * `start` - The starting height from which to calculate.
/// * `params` - [PowParams](PowParams) of the network
pub fn get_difficulty_adjustment_height(
    idx: u32,
    start: u32,
    difficulty_adjustment_interval: u32,
) -> u32 {
    ((start / difficulty_adjustment_interval) + idx) * difficulty_adjustment_interval
}

pub struct BtcChainSegment {
    pub params: Params,
    pub headers: Vec<Header>,
    pub start: u32,
    pub end: u32,
}

impl BtcChainSegment {
    /// Retrieves the block header at the specified height.
    pub fn get_header(&self, height: u32) -> Header {
        assert!(
            (self.start..self.end).contains(&height),
            "height must be in the range [{}..{})",
            self.start,
            self.end
        );
        let idx = height - self.start;
        self.headers[idx as usize]
    }

    /// Retrieves the timestamps of a specified number of blocks from a given height in a
    /// descending order.
    pub fn get_last_timestamps(&self, from: u32, count: u32) -> Vec<u32> {
        let mut timestamps = Vec::with_capacity(count as usize);
        for i in (0..count).rev() {
            let h = self.get_header(from - i);
            timestamps.push(h.time)
        }
        timestamps
    }

    pub fn get_first_difficulty_adjustment_height(&self) -> u32 {
        get_difficulty_adjustment_height(
            1,
            self.start,
            self.params.difficulty_adjustment_interval() as u32,
        )
    }

    pub fn get_header_verification_info(&self, height: u32) -> (Buf32, u32, [u32; 11], u32) {
        let difficulty_adjustment_interval = self.params.difficulty_adjustment_interval() as u32;

        // Consider the block before `height` to be the last verified block
        let vh = height - 1; // verified_height

        let vb = self.get_header(vh); // verified block

        // Fetch the previous timestamps of block from `vh`
        // This fetches timestamps of `vh`, `vh-1`, `vh-2`, ...
        let recent_block_timestamps: [u32; 11] =
            self.get_last_timestamps(vh, 11).try_into().unwrap();

        // Get the difficulty adjustment height just before the block
        let h1 = get_difficulty_adjustment_height(0, height, difficulty_adjustment_interval);

        (
            vb.block_hash().as_raw_hash().to_byte_array().into(),
            vb.target().to_compact_lossy().to_consensus(),
            recent_block_timestamps,
            self.get_header(h1).time,
        )
    }
}

pub fn get_btc_chain(params: Params) -> BtcChainSegment {
    let buffer = include_bytes!("../data/mainnet_blocks_40000-50000.raw");

    let chunk_size = Header::SIZE;
    let capacity = buffer.len() / chunk_size;
    let mut headers = Vec::with_capacity(capacity);

    for chunk in buffer.chunks(chunk_size) {
        let raw_header = chunk.to_vec();
        let header: Header = deserialize(&raw_header).unwrap();
        headers.push(header);
    }

    BtcChainSegment {
        params,
        headers,
        start: 40_000,
        end: 50_000,
    }
}
