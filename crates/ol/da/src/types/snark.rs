//! Snark account diff types.

use strata_acct_types::Hash;
use strata_codec::{Codec, CodecError, Decoder, Encoder};
use strata_da_framework::{
    BitSeqReader, BitSeqWriter, CompoundMember, DaCounter, DaLinacc, DaRegister,
    counter_schemes::{self, CtrU64ByU16},
};

use super::inbox::InboxBuffer;

/// Diff for snark account state.
#[derive(Debug)]
pub struct SnarkAccountDiff {
    /// Sequence number counter diff.
    pub seq_no: DaCounter<CtrU64ByU16>,

    /// Inner state root register diff.
    pub inner_state_root: DaRegister<Hash>,

    /// Next message read index counter diff.
    pub next_msg_read_idx: DaCounter<CtrU64ByU16>,

    /// Inbox append-only diff.
    pub inbox: DaLinacc<InboxBuffer>,
}

impl Default for SnarkAccountDiff {
    fn default() -> Self {
        Self {
            seq_no: DaCounter::new_unchanged(),
            inner_state_root: DaRegister::new_unset(),
            next_msg_read_idx: DaCounter::new_unchanged(),
            inbox: DaLinacc::new(),
        }
    }
}

impl SnarkAccountDiff {
    /// Creates a new [`SnarkAccountDiff`] from a sequence number, state root, next-read index,
    /// and inbox diff.
    pub fn new(
        seq_no: DaCounter<counter_schemes::CtrU64ByU16>,
        inner_state_root: DaRegister<Hash>,
        next_msg_read_idx: DaCounter<counter_schemes::CtrU64ByU16>,
        inbox: DaLinacc<InboxBuffer>,
    ) -> Self {
        Self {
            seq_no,
            inner_state_root,
            next_msg_read_idx,
            inbox,
        }
    }
}

impl Codec for SnarkAccountDiff {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        let mut bitw = BitSeqWriter::<u8>::new();
        bitw.prepare_member(&self.seq_no);
        bitw.prepare_member(&self.inner_state_root);
        bitw.prepare_member(&self.next_msg_read_idx);
        bitw.prepare_member(&self.inbox);
        bitw.mask().encode(enc)?;

        if !CompoundMember::is_default(&self.seq_no) {
            CompoundMember::encode_set(&self.seq_no, enc)?;
        }
        if !CompoundMember::is_default(&self.inner_state_root) {
            CompoundMember::encode_set(&self.inner_state_root, enc)?;
        }
        if !CompoundMember::is_default(&self.next_msg_read_idx) {
            CompoundMember::encode_set(&self.next_msg_read_idx, enc)?;
        }
        if !CompoundMember::is_default(&self.inbox) {
            CompoundMember::encode_set(&self.inbox, enc)?;
        }

        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let mask = u8::decode(dec)?;
        let mut bitr = BitSeqReader::from_mask(mask);
        let seq_no = bitr.decode_next_member::<DaCounter<CtrU64ByU16>>(dec)?;
        let inner_state_root = bitr.decode_next_member::<DaRegister<Hash>>(dec)?;
        let next_msg_read_idx = bitr.decode_next_member::<DaCounter<CtrU64ByU16>>(dec)?;
        let inbox = bitr.decode_next_member::<DaLinacc<InboxBuffer>>(dec)?;
        Ok(Self {
            seq_no,
            inner_state_root,
            next_msg_read_idx,
            inbox,
        })
    }
}

impl CompoundMember for SnarkAccountDiff {
    fn default() -> Self {
        <SnarkAccountDiff as Default>::default()
    }

    fn is_default(&self) -> bool {
        CompoundMember::is_default(&self.seq_no)
            && CompoundMember::is_default(&self.inner_state_root)
            && CompoundMember::is_default(&self.next_msg_read_idx)
            && CompoundMember::is_default(&self.inbox)
    }

    fn decode_set(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        Self::decode(dec)
    }

    fn encode_set(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        if CompoundMember::is_default(self) {
            return Err(CodecError::InvalidVariant("snark_account_diff"));
        }
        self.encode(enc)
    }
}
