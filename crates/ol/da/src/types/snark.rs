//! Snark account diff types.

use strata_acct_types::Hash;
use strata_codec::{Codec, CodecError, Decoder, Encoder};
use strata_da_framework::{
    CompoundMember, DaCounter, DaLinacc, DaRegister,
    counter_schemes::{self, CtrU64ByU16},
    make_compound_impl,
};
use strata_snark_acct_types::ProofState;

/// DA-encoded proof state (inner state root + next inbox read index).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DaProofState {
    inner: ProofState,
}

impl DaProofState {
    pub fn new(inner_state_root: Hash, next_msg_read_idx: u64) -> Self {
        Self {
            inner: ProofState::new(inner_state_root, next_msg_read_idx),
        }
    }

    pub fn inner(&self) -> &ProofState {
        &self.inner
    }

    pub fn into_inner(self) -> ProofState {
        self.inner
    }
}

impl Default for DaProofState {
    fn default() -> Self {
        Self::new([0u8; 32].into(), 0)
    }
}

impl From<ProofState> for DaProofState {
    fn from(inner: ProofState) -> Self {
        Self { inner }
    }
}

impl From<DaProofState> for ProofState {
    fn from(value: DaProofState) -> Self {
        value.inner
    }
}

impl Codec for DaProofState {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.inner.inner_state().encode(enc)?;
        self.inner.next_inbox_msg_idx().encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let inner_state_root = Hash::decode(dec)?;
        let next_msg_read_idx = u64::decode(dec)?;
        Ok(Self::new(inner_state_root, next_msg_read_idx))
    }
}

use super::inbox::InboxBuffer;

/// Diff for snark account state.
#[derive(Debug)]
pub struct SnarkAccountDiff {
    /// Sequence number counter diff.
    pub seq_no: DaCounter<CtrU64ByU16>,

    /// Proof state register diff.
    pub proof_state: DaRegister<DaProofState>,

    /// Inbox append-only diff.
    pub inbox: DaLinacc<InboxBuffer>,
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
    /// Creates a new [`SnarkAccountDiff`] from a sequence number, proof state, and inbox diff.
    pub fn new(
        seq_no: DaCounter<counter_schemes::CtrU64ByU16>,
        proof_state: DaRegister<DaProofState>,
        inbox: DaLinacc<InboxBuffer>,
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
        inbox: compound (DaLinacc<InboxBuffer>),
    }
}

/// Target for applying a snark account diff.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SnarkAccountTarget {
    pub seq_no: u64,
    pub proof_state: DaProofState,
    pub inbox: InboxBuffer,
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
