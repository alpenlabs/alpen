//! Simple execution environment types.

use std::collections::BTreeMap;

use digest::Digest;
use sha2::Sha256;
use strata_acct_types::{AccountId, BitcoinAmount, MsgPayload, SentMessage, SubjectId};
use strata_codec::{Codec, CodecError};
use strata_ee_acct_types::{
    EnvError, EnvResult, ExecBlock, ExecBlockBody, ExecHeader, ExecPartialState,
};
use strata_ee_chain_types::BlockOutputs;

pub(crate) type Hash = [u8; 32];

/// Write batch containing the updated account state.
#[derive(Clone, Debug)]
pub struct SimpleWriteBatch {
    accounts: BTreeMap<SubjectId, u64>,
}

impl SimpleWriteBatch {
    pub fn new(accounts: BTreeMap<SubjectId, u64>) -> Self {
        Self { accounts }
    }

    pub fn accounts(&self) -> &BTreeMap<SubjectId, u64> {
        &self.accounts
    }
}

/// Intrinsics for a simple header (fields that are known before execution).
#[derive(Clone, Debug)]
pub struct SimpleHeaderIntrinsics {
    pub parent_blkid: Hash,
    pub index: u64,
}

/// Partial state representing accounts as a simple mapping.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SimplePartialState {
    accounts: BTreeMap<SubjectId, u64>,
}

impl SimplePartialState {
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

impl ExecPartialState for SimplePartialState {
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

impl Codec for SimplePartialState {
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

/// Simple block header.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SimpleHeader {
    /// Parent block ID
    pub parent_blkid: Hash,

    /// State root after applying this block
    pub state_root: Hash,

    /// Block index/height
    pub index: u64,
}

impl SimpleHeader {
    pub fn new(parent_blkid: Hash, state_root: Hash, index: u64) -> Self {
        Self {
            parent_blkid,
            state_root,
            index,
        }
    }

    pub fn genesis() -> Self {
        Self {
            parent_blkid: [0; 32],
            state_root: SimplePartialState::new_empty()
                .compute_state_root()
                .expect("genesis state root"),
            index: 0,
        }
    }

    pub fn index(&self) -> u64 {
        self.index
    }
}

impl ExecHeader for SimpleHeader {
    type Intrinsics = SimpleHeaderIntrinsics;

    fn get_intrinsics(&self) -> Self::Intrinsics {
        SimpleHeaderIntrinsics {
            parent_blkid: self.parent_blkid,
            index: self.index,
        }
    }

    fn get_state_root(&self) -> Hash {
        self.state_root
    }

    fn compute_block_id(&self) -> Hash {
        let mut hasher = Sha256::new();
        hasher.update(&self.parent_blkid);
        hasher.update(&self.state_root);
        hasher.update(&self.index.to_le_bytes());
        hasher.finalize().into()
    }
}

impl Codec for SimpleHeader {
    fn encode(&self, enc: &mut impl strata_codec::Encoder) -> Result<(), CodecError> {
        self.parent_blkid.encode(enc)?;
        self.state_root.encode(enc)?;
        self.index.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl strata_codec::Decoder) -> Result<Self, CodecError> {
        Ok(Self {
            parent_blkid: Hash::decode(dec)?,
            state_root: Hash::decode(dec)?,
            index: u64::decode(dec)?,
        })
    }
}

/// Simple block body containing transactions.
#[derive(Clone, Debug)]
pub struct SimpleBlockBody {
    transactions: Vec<SimpleTransaction>,
}

impl SimpleBlockBody {
    pub fn new(transactions: Vec<SimpleTransaction>) -> Self {
        Self { transactions }
    }

    pub fn transactions(&self) -> &[SimpleTransaction] {
        &self.transactions
    }
}

impl ExecBlockBody for SimpleBlockBody {}

impl Codec for SimpleBlockBody {
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
            transactions.push(SimpleTransaction::decode(dec)?);
        }

        Ok(Self { transactions })
    }
}

/// Simple block containing header and body.
#[derive(Clone, Debug)]
pub struct SimpleBlock {
    header: SimpleHeader,
    body: SimpleBlockBody,
}

impl SimpleBlock {
    pub fn new(header: SimpleHeader, body: SimpleBlockBody) -> Self {
        Self { header, body }
    }

    pub fn transactions(&self) -> &[SimpleTransaction] {
        self.body.transactions()
    }
}

impl ExecBlock for SimpleBlock {
    type Header = SimpleHeader;
    type Body = SimpleBlockBody;

    fn from_parts(header: Self::Header, body: Self::Body) -> Self {
        Self { header, body }
    }

    fn check_header_matches_body(_header: &Self::Header, _body: &Self::Body) -> bool {
        // For the simple implementation, headers always match bodies
        true
    }

    fn get_header(&self) -> &Self::Header {
        &self.header
    }

    fn get_body(&self) -> &Self::Body {
        &self.body
    }
}

impl Codec for SimpleBlock {
    fn encode(&self, enc: &mut impl strata_codec::Encoder) -> Result<(), CodecError> {
        self.header.encode(enc)?;
        self.body.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl strata_codec::Decoder) -> Result<Self, CodecError> {
        let header = SimpleHeader::decode(dec)?;
        let body = SimpleBlockBody::decode(dec)?;
        Ok(Self { header, body })
    }
}

/// Simple transaction that can transfer value or emit outputs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SimpleTransaction {
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
    /// Emit a message output to a subject in another EE
    EmitMessage {
        from: SubjectId,
        dest_account: AccountId,
        dest_subject: SubjectId,
        value: u64,
        data: Vec<u8>,
    },
}

impl SimpleTransaction {
    /// Apply this transaction to the account state.
    pub fn apply(
        &self,
        accounts: &mut BTreeMap<SubjectId, u64>,
        outputs: &mut BlockOutputs,
    ) -> EnvResult<()> {
        match self {
            SimpleTransaction::Transfer { from, to, value } => {
                // Deduct from source
                let from_bal = accounts.get_mut(from).ok_or(EnvError::InvalidBlockTx)?;
                *from_bal = from_bal
                    .checked_sub(*value)
                    .ok_or(EnvError::InvalidBlockTx)?;

                // Add to destination
                let to_bal = accounts.entry(*to).or_insert(0);
                *to_bal = to_bal.checked_add(*value).ok_or(EnvError::InvalidBlockTx)?;
            }
            SimpleTransaction::EmitTransfer { from, dest, value } => {
                // Deduct from source
                let from_bal = accounts.get_mut(from).ok_or(EnvError::InvalidBlockTx)?;
                *from_bal = from_bal
                    .checked_sub(*value)
                    .ok_or(EnvError::InvalidBlockTx)?;

                // Emit output
                use strata_ee_chain_types::OutputTransfer;
                outputs.add_transfer(OutputTransfer::new(*dest, BitcoinAmount::from(*value)));
            }
            SimpleTransaction::EmitMessage {
                from,
                dest_account,
                dest_subject,
                value,
                data,
            } => {
                // Deduct from source
                let from_bal = accounts.get_mut(from).ok_or(EnvError::InvalidBlockTx)?;
                *from_bal = from_bal
                    .checked_sub(*value)
                    .ok_or(EnvError::InvalidBlockTx)?;

                // Encode message data: dest_subject + user data
                let mut msg_data = Vec::new();
                msg_data.extend_from_slice(dest_subject.inner());
                msg_data.extend_from_slice(data);

                // Emit message output
                let payload = MsgPayload::new(BitcoinAmount::from(*value), msg_data);
                let message = SentMessage::new(*dest_account, payload);
                outputs.add_message(message);
            }
        }

        Ok(())
    }
}

impl Codec for SimpleTransaction {
    fn encode(&self, enc: &mut impl strata_codec::Encoder) -> Result<(), CodecError> {
        match self {
            SimpleTransaction::Transfer { from, to, value } => {
                0u8.encode(enc)?;
                from.encode(enc)?;
                to.encode(enc)?;
                value.encode(enc)?;
            }
            SimpleTransaction::EmitTransfer { from, dest, value } => {
                1u8.encode(enc)?;
                from.encode(enc)?;
                dest.encode(enc)?;
                value.encode(enc)?;
            }
            SimpleTransaction::EmitMessage {
                from,
                dest_account,
                dest_subject,
                value,
                data,
            } => {
                2u8.encode(enc)?;
                from.encode(enc)?;
                dest_account.encode(enc)?;
                dest_subject.encode(enc)?;
                value.encode(enc)?;
                (data.len() as u32).encode(enc)?;
                enc.write_buf(data)?;
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
                Ok(SimpleTransaction::Transfer { from, to, value })
            }
            1 => {
                let from = SubjectId::decode(dec)?;
                let dest = AccountId::decode(dec)?;
                let value = u64::decode(dec)?;
                Ok(SimpleTransaction::EmitTransfer { from, dest, value })
            }
            2 => {
                let from = SubjectId::decode(dec)?;
                let dest_account = AccountId::decode(dec)?;
                let dest_subject = SubjectId::decode(dec)?;
                let value = u64::decode(dec)?;
                let len = u32::decode(dec)? as usize;
                let mut data = vec![0u8; len];
                dec.read_buf(&mut data)?;
                Ok(SimpleTransaction::EmitMessage {
                    from,
                    dest_account,
                    dest_subject,
                    value,
                    data,
                })
            }
            _ => Err(CodecError::InvalidVariant("SimpleTransaction")),
        }
    }
}
