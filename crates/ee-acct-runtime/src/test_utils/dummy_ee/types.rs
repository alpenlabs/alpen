//! Types for the dummy execution environment.

use std::collections::BTreeMap;

use digest::Digest;
use sha2::Sha256;
use strata_acct_types::{AccountId, BitcoinAmount, SubjectId, VarVec};
use strata_codec::{Codec, CodecError};
use strata_ee_acct_types::{
    EnvError, EnvResult, ExecBlock, ExecBlockBody, ExecHeader, ExecPartialState,
};
use strata_ee_chain_types::{BlockOutputs, OutputTransfer};

type Hash = [u8; 32];

/// Partial state representing accounts as a simple mapping.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DummyPartialState {
    accounts: BTreeMap<SubjectId, u64>,
}

impl DummyPartialState {
    pub fn new(accounts: BTreeMap<SubjectId, u64>) -> Self {
        Self { accounts }
    }

    pub fn new_empty() -> Self {
        Self::new(BTreeMap::new())
    }

    pub fn accounts(&self) -> &BTreeMap<SubjectId, u64> {
        &self.accounts
    }

    pub fn set_balance(&mut self, subject: SubjectId, balance: u64) {
        if balance == 0 {
            self.accounts.remove(&subject);
        } else {
            self.accounts.insert(subject, balance);
        }
    }
}

impl ExecPartialState for DummyPartialState {
    fn compute_state_root(&self) -> EnvResult<Hash> {
        // Hash the account state by encoding it as a sorted list
        let mut hasher = Sha256::new();

        for (subject, balance) in &self.accounts {
            hasher.update(subject.inner());
            hasher.update(&balance.to_le_bytes());
        }

        Ok(hasher.finalize().into())
    }
}

impl Codec for DummyPartialState {
    fn encode(&self, enc: &mut impl strata_codec::Encoder) -> Result<(), CodecError> {
        // Encode as (subject_id, balance) pairs
        let entries: Vec<_> = self.accounts.iter().collect();
        (entries.len() as u32).encode(enc)?;

        for (subject, balance) in entries {
            subject.encode(enc)?;
            balance.encode(enc)?;
        }

        Ok(())
    }

    fn decode(dec: &mut impl strata_codec::Decoder) -> Result<Self, CodecError> {
        let len = u32::decode(dec)? as usize;
        let mut accounts = BTreeMap::new();

        for _ in 0..len {
            let subject = SubjectId::decode(dec)?;
            let balance = u64::decode(dec)?;
            accounts.insert(subject, balance);
        }

        Ok(Self { accounts })
    }
}

/// Intrinsic data for a dummy block header.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DummyHeaderIntrinsics {
    /// Parent block ID
    pub parent_blkid: Hash,

    /// Block index/height
    pub index: u64,
}

/// Dummy block header.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DummyHeader {
    /// Intrinsic data
    intrinsics: DummyHeaderIntrinsics,

    /// State root after applying this block
    pub state_root: Hash,
}

impl DummyHeader {
    pub fn new(parent_blkid: Hash, state_root: Hash, index: u64) -> Self {
        Self {
            intrinsics: DummyHeaderIntrinsics {
                parent_blkid,
                index,
            },
            state_root,
        }
    }

    pub fn genesis() -> Self {
        Self {
            intrinsics: DummyHeaderIntrinsics {
                parent_blkid: [0; 32],
                index: 0,
            },
            state_root: DummyPartialState::new_empty()
                .compute_state_root()
                .expect("genesis state root"),
        }
    }

    pub fn parent_blkid(&self) -> Hash {
        self.intrinsics.parent_blkid
    }

    pub fn index(&self) -> u64 {
        self.intrinsics.index
    }
}

impl ExecHeader for DummyHeader {
    type Intrinsics = DummyHeaderIntrinsics;

    fn get_intrinsics(&self) -> Self::Intrinsics {
        self.intrinsics.clone()
    }

    fn get_state_root(&self) -> Hash {
        self.state_root
    }

    fn compute_block_id(&self) -> Hash {
        let mut hasher = Sha256::new();
        hasher.update(&self.intrinsics.parent_blkid);
        hasher.update(&self.state_root);
        hasher.update(&self.intrinsics.index.to_le_bytes());
        hasher.finalize().into()
    }
}

impl Codec for DummyHeader {
    fn encode(&self, enc: &mut impl strata_codec::Encoder) -> Result<(), CodecError> {
        self.intrinsics.parent_blkid.encode(enc)?;
        self.state_root.encode(enc)?;
        self.intrinsics.index.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl strata_codec::Decoder) -> Result<Self, CodecError> {
        let parent_blkid = Hash::decode(dec)?;
        let state_root = Hash::decode(dec)?;
        let index = u64::decode(dec)?;
        Ok(Self {
            intrinsics: DummyHeaderIntrinsics {
                parent_blkid,
                index,
            },
            state_root,
        })
    }
}

/// Dummy block body containing transactions.
#[derive(Clone, Debug)]
pub struct DummyBlockBody {
    transactions: Vec<DummyTransaction>,
}

impl DummyBlockBody {
    pub fn new(transactions: Vec<DummyTransaction>) -> Self {
        Self { transactions }
    }

    pub fn transactions(&self) -> &[DummyTransaction] {
        &self.transactions
    }
}

impl ExecBlockBody for DummyBlockBody {}

impl Codec for DummyBlockBody {
    fn encode(&self, enc: &mut impl strata_codec::Encoder) -> Result<(), CodecError> {
        (self.transactions.len() as u32).encode(enc)?;

        for tx in &self.transactions {
            tx.encode(enc)?;
        }

        Ok(())
    }

    fn decode(dec: &mut impl strata_codec::Decoder) -> Result<Self, CodecError> {
        let len = u32::decode(dec)? as usize;
        let mut transactions = Vec::with_capacity(len);

        for _ in 0..len {
            transactions.push(DummyTransaction::decode(dec)?);
        }

        Ok(Self { transactions })
    }
}

/// Dummy block containing header and body.
#[derive(Clone, Debug)]
pub struct DummyBlock {
    header: DummyHeader,
    body: DummyBlockBody,
}

impl DummyBlock {
    pub fn new(header: DummyHeader, body: DummyBlockBody) -> Self {
        Self { header, body }
    }

    pub fn transactions(&self) -> &[DummyTransaction] {
        self.body.transactions()
    }
}

impl ExecBlock for DummyBlock {
    type Header = DummyHeader;
    type Body = DummyBlockBody;

    fn from_parts(header: Self::Header, body: Self::Body) -> Self {
        Self { header, body }
    }

    fn check_header_matches_body(_header: &Self::Header, _body: &Self::Body) -> bool {
        // For the dummy implementation, headers always match bodies
        true
    }

    fn get_header(&self) -> &Self::Header {
        &self.header
    }

    fn get_body(&self) -> &Self::Body {
        &self.body
    }
}

impl Codec for DummyBlock {
    fn encode(&self, enc: &mut impl strata_codec::Encoder) -> Result<(), CodecError> {
        self.header.encode(enc)?;
        self.body.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl strata_codec::Decoder) -> Result<Self, CodecError> {
        let header = DummyHeader::decode(dec)?;
        let body = DummyBlockBody::decode(dec)?;
        Ok(Self { header, body })
    }
}

/// Simple transaction that can transfer value or emit outputs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DummyTransaction {
    /// Transfer value from one subject to another
    Transfer {
        from: SubjectId,
        to: SubjectId,
        value: u64,
    },
    /// Emit an output transfer to the orchestration layer
    EmitTransfer {
        from: SubjectId,
        dest: AccountId,
        value: u64,
    },
}

impl DummyTransaction {
    /// Apply this transaction to the account state.
    pub fn apply(
        &self,
        accounts: &mut BTreeMap<SubjectId, u64>,
        outputs: &mut BlockOutputs,
    ) -> EnvResult<()> {
        match self {
            DummyTransaction::Transfer { from, to, value } => {
                // Deduct from source
                let from_bal = accounts
                    .get_mut(from)
                    .ok_or(EnvError::ConflictingPublicState)?;
                *from_bal = from_bal
                    .checked_sub(*value)
                    .ok_or(EnvError::ConflictingPublicState)?;

                // Add to destination
                let to_bal = accounts.entry(*to).or_insert(0);
                *to_bal = to_bal
                    .checked_add(*value)
                    .ok_or(EnvError::ConflictingPublicState)?;
            }
            DummyTransaction::EmitTransfer { from, dest, value } => {
                // Deduct from source
                let from_bal = accounts
                    .get_mut(from)
                    .ok_or(EnvError::ConflictingPublicState)?;
                *from_bal = from_bal
                    .checked_sub(*value)
                    .ok_or(EnvError::ConflictingPublicState)?;

                // Emit output
                outputs.add_transfer(OutputTransfer::new(*dest, BitcoinAmount::from(*value)));
            }
        }

        Ok(())
    }
}

impl Codec for DummyTransaction {
    fn encode(&self, enc: &mut impl strata_codec::Encoder) -> Result<(), CodecError> {
        match self {
            DummyTransaction::Transfer { from, to, value } => {
                0u8.encode(enc)?;
                from.encode(enc)?;
                to.encode(enc)?;
                value.encode(enc)?;
            }
            DummyTransaction::EmitTransfer { from, dest, value } => {
                1u8.encode(enc)?;
                from.encode(enc)?;
                dest.encode(enc)?;
                value.encode(enc)?;
            }
        }
        Ok(())
    }

    fn decode(dec: &mut impl strata_codec::Decoder) -> Result<Self, CodecError> {
        let tag = u8::decode(dec)?;
        match tag {
            0 => {
                let from = SubjectId::decode(dec)?;
                let to = SubjectId::decode(dec)?;
                let value = u64::decode(dec)?;
                Ok(DummyTransaction::Transfer { from, to, value })
            }
            1 => {
                let from = SubjectId::decode(dec)?;
                let dest = AccountId::decode(dec)?;
                let value = u64::decode(dec)?;
                Ok(DummyTransaction::EmitTransfer { from, dest, value })
            }
            _ => Err(CodecError::InvalidVariant("DummyTransaction")),
        }
    }
}
