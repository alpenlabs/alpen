use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_primitives::l1::{HeaderVerificationState, L1BlockCommitment, L1BlockId};

/// Describes state relating to the CL's view of L1.  Updated by entries in the
/// L1 segment of CL blocks.
#[derive(Clone, Debug, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct L1ViewState {
    /// The first block we decide we're able to look at.  This probably won't
    /// change unless we want to do Bitcoin history expiry or something.
    pub(crate) horizon_height: u64,

    /// The actual first block we ever looked at.
    pub(crate) genesis_height: u64,

    /// State against which the new L1 block header are verified
    pub(crate) header_vs: HeaderVerificationState,
}

impl L1ViewState {
    /// Creates a new instance with the genesis trigger L1 block already ingested.
    pub fn new_at_genesis(
        horizon_height: u64,
        genesis_height: u64,
        header_vs: HeaderVerificationState,
    ) -> Self {
        Self {
            horizon_height,
            genesis_height,
            header_vs,
        }
    }

    pub fn safe_blkid(&self) -> &L1BlockId {
        self.header_vs.last_verified_block.blkid()
    }

    pub fn safe_height(&self) -> u64 {
        self.header_vs.last_verified_block.height()
    }

    pub fn header_vs(&self) -> &HeaderVerificationState {
        &self.header_vs
    }

    pub fn header_vs_mut(&mut self) -> &mut HeaderVerificationState {
        &mut self.header_vs
    }

    /// Gets the safe block as a [`L1BlockCommitment`].
    pub fn get_safe_block(&self) -> L1BlockCommitment {
        L1BlockCommitment::new(self.safe_height(), *self.safe_blkid())
    }

    /// The height of the next block we expect to be added.
    pub fn next_expected_height(&self) -> u64 {
        self.safe_height() + 1
    }
}

impl<'a> Arbitrary<'a> for L1ViewState {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let hh = u8::arbitrary(u)? as u64;
        let gh = hh + u16::arbitrary(u)? as u64;
        let header_vs = HeaderVerificationState::arbitrary(u)?;
        Ok(Self::new_at_genesis(hh, gh, header_vs))
    }
}
