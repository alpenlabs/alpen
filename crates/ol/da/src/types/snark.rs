//! Snark account diff types.

use strata_acct_types::Hash;
use strata_codec::{Codec, CodecError, Decoder, Encoder};
use strata_da_framework::{
    CompoundMember, DaCounter, DaLinacc, DaRegister,
    counter_schemes::{self, CtrU64ByU16},
    make_compound_impl,
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

make_compound_impl! {
    SnarkAccountDiff u8 => SnarkAccountTarget {
        seq_no: counter (counter_schemes::CtrU64ByU16),
        inner_state_root: register (Hash),
        next_msg_read_idx: counter (counter_schemes::CtrU64ByU16),
        inbox: compound (DaLinacc<InboxBuffer>),
    }
}

/// Target for applying a snark account diff.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SnarkAccountTarget {
    pub seq_no: u64,
    pub inner_state_root: Hash,
    pub next_msg_read_idx: u64,
    pub inbox: InboxBuffer,
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
