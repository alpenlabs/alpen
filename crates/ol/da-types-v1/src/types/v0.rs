//! Decoding of V0 DA payloads.
//!
//! The V1 layout is the V0 layout with `update_vk` appended to the snark
//! account diff. A compound decoder reads one presence bit per member it knows
//! and ignores the rest of its bitmap, so V0 reads a snark diff from the
//! first three bits whatever the others hold. The V1 decoder reads the fourth
//! bit as `update_vk`, so bytes V0 accepts with that bit set fail to
//! decode, or decode differently, under V1. A V0 checkpoint proves the DA bytes
//! it carries as V0 decodes them, so replaying a V0 epoch must decode its
//! payload by the V0 rules. Every other type decodes the same under both.
//!
//! The types here only decode; they convert into the V1 types, with
//! `update_vk` unset.

use strata_codec::{Codec, CodecError, Decoder, Encoder, decode_buf_exact};
use strata_da_framework::{BitSeqReader, CompoundMember, DaRegister};
use strata_identifiers::AccountSerial;
use strata_ol_da_common::U16LenList;

use super::{
    AccountDiffEntryV1, AccountDiffV1, GlobalStateDiffV1, LedgerDiffV1, NewAccountEntryV1,
    OLDaPayloadV1, OLStateDiffV1, SnarkAccountDiffV1,
};

/// Decodes an OL DA payload by the V0 rules, which a V0 epoch's checkpoint
/// proves, into the V1 types.
///
/// It differs from [`decode_ol_da_payload_bytes`](super::decode_ol_da_payload_bytes)
/// only in the snark account diff, whose presence bits past the V0 members
/// it ignores, leaving `update_vk` unset.
pub fn decode_v0_payload_bytes(bytes: &[u8]) -> Result<OLDaPayloadV1, CodecError> {
    let payload: V0Payload = decode_buf_exact(bytes)?;
    Ok(payload.into_v1())
}

/// [`OLDaPayloadV1`] as V0 decodes it: the [`OLStateDiffV1`] members.
#[derive(Codec)]
struct V0Payload {
    global: GlobalStateDiffV1,
    ledger: V0LedgerDiff,
}

impl V0Payload {
    fn into_v1(self) -> OLDaPayloadV1 {
        let account_diffs = self
            .ledger
            .account_diffs
            .into_entries()
            .into_iter()
            .map(|entry| AccountDiffEntryV1::new(entry.account_serial, entry.diff.0))
            .collect();
        OLDaPayloadV1::new(OLStateDiffV1::new(
            self.global,
            LedgerDiffV1::new(self.ledger.new_accounts, U16LenList::new(account_diffs)),
        ))
    }
}

/// [`LedgerDiffV1`] as V0 decodes it.
#[derive(Codec)]
struct V0LedgerDiff {
    new_accounts: U16LenList<NewAccountEntryV1>,
    account_diffs: U16LenList<V0AccountDiffEntry>,
}

/// [`AccountDiffEntryV1`] as V0 decodes it.
#[derive(Codec)]
struct V0AccountDiffEntry {
    account_serial: AccountSerial,
    diff: V0AccountDiff,
}

/// [`AccountDiffV1`] as V0 decodes it, with its snark diff read by the
/// V0 rules.
struct V0AccountDiff(AccountDiffV1);

impl Codec for V0AccountDiff {
    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let mask = u8::decode(dec)?;
        let mut bits = BitSeqReader::from_mask(mask);
        let balance = bits.decode_next_member(dec)?;
        let V0SnarkAccountDiff(snark) = bits.decode_next_member(dec)?;
        Ok(Self(AccountDiffV1::new(balance, snark)))
    }

    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        // `update_vk` is unset, so this is the V0 encoding.
        self.0.encode(enc)
    }
}

/// [`SnarkAccountDiffV1`] as V0 decodes it: `seq_no`, `proof_state` and
/// `inbox`, with any further presence bits ignored and `update_vk` unset.
struct V0SnarkAccountDiff(SnarkAccountDiffV1);

impl CompoundMember for V0SnarkAccountDiff {
    fn default() -> Self {
        Self(CompoundMember::default())
    }

    fn is_default(&self) -> bool {
        CompoundMember::is_default(&self.0)
    }

    fn decode_set(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let mask = u8::decode(dec)?;
        let mut bits = BitSeqReader::from_mask(mask);
        let seq_no = bits.decode_next_member(dec)?;
        let proof_state = bits.decode_next_member(dec)?;
        let inbox = bits.decode_next_member(dec)?;
        Ok(Self(SnarkAccountDiffV1::new(
            seq_no,
            proof_state,
            inbox,
            DaRegister::new_unset(),
        )))
    }

    fn encode_set(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        // `update_vk` is unset, so this is the V0 encoding.
        self.0.encode_set(enc)
    }
}
