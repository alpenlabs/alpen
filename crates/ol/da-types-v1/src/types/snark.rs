//! Snark account diff types.

use strata_acct_types::Hash;
use strata_codec::{Codec, CodecError, Decoder, Encoder};
use strata_da_framework::counter_schemes::{CtrU64ByU16, CtrU64ByUnsignedVarInt};
use strata_da_framework::{
    BitSeqReader, BitSeqWriter, CompoundMember, DaCounter, DaLinacc, DaRegister, DaWrite,
    make_compound_impl,
};
use strata_ol_da_common::{DaError, U16LenBytes};
use strata_snark_acct_types::ProofState;

use super::inbox::InboxBufferV1;

/// DA-encoded proof state (inner state root + next inbox read index).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DaProofStateV1 {
    inner: ProofState,
}

impl DaProofStateV1 {
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

impl Default for DaProofStateV1 {
    fn default() -> Self {
        Self::new([0u8; 32].into(), 0)
    }
}

impl From<ProofState> for DaProofStateV1 {
    fn from(inner: ProofState) -> Self {
        Self { inner }
    }
}

impl From<DaProofStateV1> for ProofState {
    fn from(value: DaProofStateV1) -> Self {
        value.inner
    }
}

impl Codec for DaProofStateV1 {
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
pub struct DaProofStateDiffV1 {
    pub inner_state: DaRegister<Hash>,
    pub next_inbox_msg_idx: DaCounter<CtrU64ByUnsignedVarInt>,
}

impl DaProofStateDiffV1 {
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

impl Default for DaProofStateDiffV1 {
    fn default() -> Self {
        Self {
            inner_state: DaRegister::new_unset(),
            next_inbox_msg_idx: DaCounter::new_unchanged(),
        }
    }
}

impl Codec for DaProofStateDiffV1 {
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

impl DaWrite for DaProofStateDiffV1 {
    type Target = DaProofStateV1;
    type Context = ();
    type Error = DaError;

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

        *target = DaProofStateV1::new(inner_state, next_inbox_msg_idx);
        Ok(())
    }
}

impl CompoundMember for DaProofStateDiffV1 {
    fn default() -> Self {
        <DaProofStateDiffV1 as Default>::default()
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

/// Diff for snark account state.
///
/// Field order mirrors `SnarkAccountState` for consistency.
#[derive(Debug)]
pub struct SnarkAccountDiffV1 {
    /// Update predicate key (VK) register, set when an update declares a
    /// rotation. Carries the serialized key bytes, as in
    /// [`SnarkAccountInitV1`](super::ledger::SnarkAccountInitV1).
    pub update_vk: DaRegister<U16LenBytes>,

    /// Proof state diff.
    pub proof_state: DaProofStateDiffV1,

    /// Sequence number counter diff.
    pub seq_no: DaCounter<CtrU64ByU16>,

    /// Inbox append-only diff.
    pub inbox: DaLinacc<InboxBufferV1>,
}

impl Default for SnarkAccountDiffV1 {
    fn default() -> Self {
        Self {
            update_vk: DaRegister::new_unset(),
            proof_state: <DaProofStateDiffV1 as Default>::default(),
            seq_no: DaCounter::new_unchanged(),
            inbox: DaLinacc::new(),
        }
    }
}

impl SnarkAccountDiffV1 {
    /// Creates a new [`SnarkAccountDiffV1`] from an update VK register, proof
    /// state, sequence number, and inbox diff.
    pub fn new(
        update_vk: DaRegister<U16LenBytes>,
        proof_state: DaProofStateDiffV1,
        seq_no: DaCounter<CtrU64ByU16>,
        inbox: DaLinacc<InboxBufferV1>,
    ) -> Self {
        Self {
            update_vk,
            proof_state,
            seq_no,
            inbox,
        }
    }
}

make_compound_impl! {
    SnarkAccountDiffV1 < (), DaError > u8 => SnarkAccountTargetV1 {
        update_vk: register (U16LenBytes),
        proof_state: compound (DaProofStateDiffV1),
        seq_no: counter (CtrU64ByU16),
        inbox: compound (DaLinacc<InboxBufferV1>),
    }
}

/// Target state for applying a [`SnarkAccountDiffV1`].
///
/// This struct is the `DaWrite::Target` for snark diffs and is used by
/// higher-level account diff targets during DA application.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SnarkAccountTargetV1 {
    pub update_vk: U16LenBytes,
    pub proof_state: DaProofStateV1,
    pub seq_no: u64,
    pub inbox: InboxBufferV1,
}

impl CompoundMember for SnarkAccountDiffV1 {
    fn default() -> Self {
        <SnarkAccountDiffV1 as Default>::default()
    }

    fn is_default(&self) -> bool {
        CompoundMember::is_default(&self.update_vk)
            && CompoundMember::is_default(&self.proof_state)
            && CompoundMember::is_default(&self.seq_no)
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
