//! Snark account diff types.

use strata_acct_types::Hash;
use strata_codec::{Codec, CodecError, Decoder, Encoder};
use strata_da_framework::{
    BitSeqReader, BitSeqWriter, CompoundMember, DaCounter, DaLinacc, DaRegister, DaWrite,
    counter_schemes::{CtrU64ByU16, CtrU64ByUnsignedVarInt},
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

/// Diff for proof state (inner state root + next inbox read index).
#[derive(Clone, Debug)]
pub struct DaProofStateDiff {
    pub inner_state: DaRegister<Hash>,
    pub next_inbox_msg_idx: DaCounter<CtrU64ByUnsignedVarInt>,
}

impl DaProofStateDiff {
    pub fn new(
        inner_state: DaRegister<Hash>,
        next_inbox_msg_idx: DaCounter<CtrU64ByUnsignedVarInt>,
    ) -> Self {
        Self {
            inner_state,
            next_inbox_msg_idx,
        }
    }
}

impl Default for DaProofStateDiff {
    fn default() -> Self {
        Self {
            inner_state: DaRegister::new_unset(),
            next_inbox_msg_idx: DaCounter::new_unchanged(),
        }
    }
}

impl Codec for DaProofStateDiff {
    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let mask = u8::decode(dec)?;
        let mut bitr = BitSeqReader::from_mask(mask);

        let inner_state = bitr.decode_next_member::<DaRegister<Hash>>(dec)?;
        let next_inbox_msg_idx =
            bitr.decode_next_member::<DaCounter<CtrU64ByUnsignedVarInt>>(dec)?;

        Ok(Self {
            inner_state,
            next_inbox_msg_idx,
        })
    }

    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        let mut bitw = BitSeqWriter::<u8>::new();
        bitw.prepare_member(&self.inner_state);
        bitw.prepare_member(&self.next_inbox_msg_idx);

        bitw.mask().encode(enc)?;

        if !CompoundMember::is_default(&self.inner_state) {
            CompoundMember::encode_set(&self.inner_state, enc)?;
        }
        if !CompoundMember::is_default(&self.next_inbox_msg_idx) {
            CompoundMember::encode_set(&self.next_inbox_msg_idx, enc)?;
        }

        Ok(())
    }
}

impl DaWrite for DaProofStateDiff {
    type Target = DaProofState;
    type Context = ();
    type Error = crate::DaError;

    fn is_default(&self) -> bool {
        DaWrite::is_default(&self.inner_state) && DaWrite::is_default(&self.next_inbox_msg_idx)
    }

    fn apply(
        &self,
        target: &mut Self::Target,
        _context: &Self::Context,
    ) -> Result<(), Self::Error> {
        let mut inner_state = target.inner().inner_state();
        if let Some(new_inner_state) = self.inner_state.new_value() {
            inner_state = *new_inner_state;
        }

        let mut next_inbox_msg_idx = target.inner().next_inbox_msg_idx();
        self.next_inbox_msg_idx
            .apply(&mut next_inbox_msg_idx, &())?;

        *target = DaProofState::new(inner_state, next_inbox_msg_idx);
        Ok(())
    }
}

impl CompoundMember for DaProofStateDiff {
    fn default() -> Self {
        <DaProofStateDiff as Default>::default()
    }

    fn is_default(&self) -> bool {
        DaWrite::is_default(self)
    }

    fn decode_set(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        Self::decode(dec)
    }

    fn encode_set(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        if DaWrite::is_default(self) {
            return Err(CodecError::InvalidVariant("proof_state_diff"));
        }
        self.encode(enc)
    }
}

use super::inbox::InboxBuffer;

/// Diff for snark account state.
#[derive(Debug)]
pub struct SnarkAccountDiff {
    /// Sequence number counter diff.
    pub seq_no: DaCounter<CtrU64ByU16>,

    /// Proof state diff.
    pub proof_state: DaProofStateDiff,

    /// Inbox append-only diff.
    pub inbox: DaLinacc<InboxBuffer>,
}

impl Default for SnarkAccountDiff {
    fn default() -> Self {
        Self {
            seq_no: DaCounter::new_unchanged(),
            proof_state: <DaProofStateDiff as Default>::default(),
            inbox: DaLinacc::new(),
        }
    }
}

impl SnarkAccountDiff {
    /// Creates a new [`SnarkAccountDiff`] from a sequence number, proof state, and inbox diff.
    pub fn new(
        seq_no: DaCounter<CtrU64ByU16>,
        proof_state: DaProofStateDiff,
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
    SnarkAccountDiff < (), crate::DaError > u8 => SnarkAccountTarget {
        seq_no: counter (CtrU64ByU16),
        proof_state: compound (DaProofStateDiff),
        inbox: compound (DaLinacc<InboxBuffer>),
    }
}

/// Target state for applying a [`SnarkAccountDiff`].
///
/// This struct is the `DaWrite::Target` for snark diffs and is used by
/// higher-level account diff targets during DA application.
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
