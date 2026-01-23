//! Snark account diff types.

use strata_acct_types::Hash;
use strata_codec::{Codec, CodecError, Decoder, Encoder};
use strata_da_framework::{
    CompoundMember, DaCounter, DaLinacc, DaRegister,
    counter_schemes::{self, CtrU64ByU16},
    make_compound_impl,
};

use super::inbox::InboxAccumulator;

/// Diff for snark account state.
#[derive(Debug)]
pub struct SnarkAccountDiff {
    /// Sequence number counter diff.
    pub seq_no: DaCounter<CtrU64ByU16>,

    /// Proof state register diff.
    pub proof_state: DaRegister<DaProofState>,

    /// Inbox append-only diff.
    pub inbox: DaLinacc<InboxAccumulator>,
}

impl Default for SnarkAccountDiff {
    fn default() -> Self {
        Self {
            seq_no: DaCounter::new_unchanged(),
            proof_state: DaRegister::new_unset(),
            inbox: DaLinacc::new(),
        }
    }
}

impl SnarkAccountDiff {
    /// Creates a new [`SnarkAccountDiff`] from a sequence number, proof state, and inbox MMR.
    pub fn new(
        seq_no: DaCounter<counter_schemes::CtrU64ByU16>,
        proof_state: DaRegister<DaProofState>,
        inbox: DaLinacc<InboxAccumulator>,
    ) -> Self {
        Self {
            seq_no,
            proof_state,
            inbox,
        }
    }
}

make_compound_impl! {
    SnarkAccountDiff u8 => SnarkAccountTarget {
        seq_no: counter (counter_schemes::CtrU64ByU16),
        proof_state: register (DaProofState),
        inbox: compound (DaLinacc<InboxAccumulator>),
    }
}

impl CompoundMember for SnarkAccountDiff {
    fn default() -> Self {
        <SnarkAccountDiff as Default>::default()
    }

    fn is_default(&self) -> bool {
        CompoundMember::is_default(&self.seq_no)
            && CompoundMember::is_default(&self.proof_state)
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

/// Proof state snapshot used in DA diffs.
#[derive(Clone, Debug, Default, Eq, PartialEq, Codec)]
pub struct DaProofState {
    /// Inner state root commitment.
    pub inner_state_root: Hash,

    /// Next message read index.
    pub next_msg_read_idx: u64,
}

impl DaProofState {
    /// Creates a new [`DaProofState`] from a inner state root and next message read index.
    pub fn new(inner_state_root: Hash, next_msg_read_idx: u64) -> Self {
        Self {
            inner_state_root,
            next_msg_read_idx,
        }
    }
}

/// Target for applying snark account diffs.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SnarkAccountTarget {
    /// Current sequence number.
    pub seq_no: u64,

    /// Current proof state.
    pub proof_state: DaProofState,

    /// Current inbox accumulator.
    pub inbox: InboxAccumulator,
}
