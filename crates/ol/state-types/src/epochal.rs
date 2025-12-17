//! Epoch-level state that is changed during sealing/checkin.
//!
//! This can be completely omitted from DA.

use strata_acct_types::{BitcoinAmount, CompactMmr64, Mmr64};
use strata_asm_manifest_types::AsmManifest;
use strata_codec::{Codec, CodecError, Decoder, Encoder};
use strata_codec_utils::CodecSsz;
use strata_identifiers::{EpochCommitment, L1BlockCommitment, L1BlockId, L1Height};

#[derive(Clone, Debug)]
pub struct EpochalState {
    total_ledger_funds: BitcoinAmount,
    cur_epoch: u32,
    last_l1_block: L1BlockCommitment,
    checkpointed_epoch: EpochCommitment,
    manifest_mmr: Mmr64,
}

impl EpochalState {
    /// Create a new epochal state for testing.
    pub fn new(
        total_ledger_funds: BitcoinAmount,
        cur_epoch: u32,
        last_l1_block: L1BlockCommitment,
        checkpointed_epoch: EpochCommitment,
        manifest_mmr: Mmr64,
    ) -> Self {
        Self {
            total_ledger_funds,
            cur_epoch,
            last_l1_block,
            checkpointed_epoch,
            manifest_mmr,
        }
    }

    /// Gets the current epoch.
    pub fn cur_epoch(&self) -> u32 {
        self.cur_epoch
    }

    /// Sets the current epoch.
    pub fn set_cur_epoch(&mut self, epoch: u32) {
        self.cur_epoch = epoch;
    }

    /// Last L1 block ID.
    pub fn last_l1_blkid(&self) -> &L1BlockId {
        self.last_l1_block.blkid()
    }

    /// Last L1 block height.
    pub fn last_l1_height(&self) -> L1Height {
        // FIXME this conversion is weird
        self.last_l1_block.height_u64() as u32
    }

    /// Appends a new ASM manifest to the accumulator, also updating the last L1
    /// block height and other fields.
    pub fn append_manifest(&mut self, height: L1Height, mf: AsmManifest) {
        let manifest_hash = mf.blkid().as_ref();
        self.manifest_mmr
            .add_leaf(*manifest_hash)
            .expect("MMR capacity exceeded");
        // FIXME make this conversion less weird
        self.last_l1_block = L1BlockCommitment::from_height_u64(height as u64, *mf.blkid())
            .expect("state: weird conversion")
    }

    /// Gets the field for the epoch that the ASM considers to be valid.
    ///
    /// This is our perspective of the perspective of the last block's ASM
    /// manifest we've accepted.
    pub fn asm_recorded_epoch(&self) -> &EpochCommitment {
        &self.checkpointed_epoch
    }

    /// Sets the field for the epoch that the ASM considers to be finalized.
    ///
    /// This is our perspective of the perspective of the last block's ASM
    /// manifest we've accepted.
    pub fn set_asm_recorded_epoch(&mut self, epoch: EpochCommitment) {
        self.checkpointed_epoch = epoch;
    }

    /// Gets the total OL ledger balance.
    pub fn total_ledger_balance(&self) -> BitcoinAmount {
        self.total_ledger_funds
    }

    /// Sets the total OL ledger balance.
    pub fn set_total_ledger_balance(&mut self, amt: BitcoinAmount) {
        self.total_ledger_funds = amt;
    }

    /// Gets the ASM manifests MMR.
    pub fn asm_manifests_mmr(&self) -> &Mmr64 {
        &self.manifest_mmr
    }
}

impl Codec for EpochalState {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.total_ledger_funds.encode(enc)?;
        self.cur_epoch.encode(enc)?;
        self.last_l1_block.encode(enc)?;
        self.checkpointed_epoch.encode(enc)?;

        let compact_mmr = self.manifest_mmr.to_compact();
        let wrapped_mmr = CodecSsz::new(compact_mmr);
        wrapped_mmr.encode(enc)?;

        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let total_ledger_funds = BitcoinAmount::decode(dec)?;
        let cur_epoch = u32::decode(dec)?;
        let last_l1_block = L1BlockCommitment::decode(dec)?;
        let checkpointed_epoch = EpochCommitment::decode(dec)?;

        let wrapped_mmr: CodecSsz<CompactMmr64> = CodecSsz::decode(dec)?;
        let compact_mmr = wrapped_mmr.inner();
        let manifest_mmr = Mmr64::from_compact(compact_mmr);

        Ok(Self {
            total_ledger_funds,
            cur_epoch,
            last_l1_block,
            checkpointed_epoch,
            manifest_mmr,
        })
    }
}
