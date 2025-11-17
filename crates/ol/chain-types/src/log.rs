use sha2::{Digest, Sha256};
use strata_acct_types::{AccountId, BitcoinAmount};
use strata_primitives::{Buf32, EpochCommitment};

/// Log emitted during OL block execution.
#[derive(Clone, Debug)]
pub struct OLLog {
    /// Account this log is related to.
    // TODO: maybe use account serial,
    account_id: AccountId,

    /// Inner log data.
    log_data: LogData,
    // TODO: add more concrete fields.
}

impl OLLog {
    pub fn new(account_id: AccountId, log_data: LogData) -> Self {
        Self {
            account_id,
            log_data,
        }
    }

    pub fn account_id(&self) -> AccountId {
        self.account_id
    }

    pub fn log_data(&self) -> &LogData {
        &self.log_data
    }

    /// Create a withdrawal intent log.
    pub fn withdrawal_intent(account_id: AccountId, amount: BitcoinAmount, dest: Vec<u8>) -> Self {
        Self::new(
            account_id,
            LogData::WithdrawalIntent(WithdrawalIntentLogData::new(amount, dest)),
        )
    }

    /// Create a snark account update log.
    pub fn snark_account_update(
        account_id: AccountId,
        new_next_msg_idx: u64,
        new_proof_state: Buf32,
        extra_data: Vec<u8>,
    ) -> Self {
        Self::new(
            account_id,
            LogData::SnarkAccountUpdate(SnarkAccountUpdateLogData::new(
                new_next_msg_idx,
                new_proof_state,
                extra_data,
            )),
        )
    }

    /// Create a deposit acknowledgment log.
    pub fn deposit_ack(
        account_id: AccountId,
        subject_addr: Vec<u8>,
        amount: BitcoinAmount,
    ) -> Self {
        Self::new(
            account_id,
            LogData::DepositAck(DepositAckLogData::new(subject_addr, amount)),
        )
    }

    /// Create a checkpoint acknowledgment log.
    pub fn checkpoint_ack(account_id: AccountId, epoch: EpochCommitment) -> Self {
        Self::new(
            account_id,
            LogData::CheckpointAck(CheckpointAckLogData::new(epoch)),
        )
    }

    pub fn compute_root(&self) -> [u8; 32] {
        // TODO: figure out and implement this correctly
        [0; 32]
    }
}

pub fn compute_logs_root(logs: &[OLLog]) -> Buf32 {
    let mut hasher = Sha256::new();
    for log in logs {
        hasher.update(log.compute_root());
    }
    let res: [u8; 32] = hasher.finalize().into();
    res.into()
}

/// Structured representation of the type of log.
#[derive(Clone, Debug)]
pub enum LogData {
    WithdrawalIntent(WithdrawalIntentLogData),
    SnarkAccountUpdate(SnarkAccountUpdateLogData),
    DepositAck(DepositAckLogData),
    CheckpointAck(CheckpointAckLogData),
}

/// Log representing intent to withdraw amount by an account. Account is implied from the log
/// context.
#[derive(Clone, Debug)]
pub struct WithdrawalIntentLogData {
    /// Amount intended to withdraw.
    amount: BitcoinAmount,

    /// Withdrawal destination.
    dest: Vec<u8>, // TODO: use bosd?
}

impl WithdrawalIntentLogData {
    pub fn new(amount: BitcoinAmount, dest: Vec<u8>) -> Self {
        Self { amount, dest }
    }

    pub fn amount(&self) -> BitcoinAmount {
        self.amount
    }

    pub fn dest(&self) -> &[u8] {
        &self.dest
    }
}

/// Log representing snark account update.
#[derive(Clone, Debug)]
pub struct SnarkAccountUpdateLogData {
    /// End of the message index that was processed.
    new_next_msg_idx: u64,

    /// New proof state after the update.
    new_proof_state: Buf32,

    /// Any extra data.
    extra_data: Vec<u8>,
}

impl SnarkAccountUpdateLogData {
    pub fn new(new_next_msg_idx: u64, new_proof_state: Buf32, extra_data: Vec<u8>) -> Self {
        Self {
            new_next_msg_idx,
            new_proof_state,
            extra_data,
        }
    }

    pub fn to_msg_idx(&self) -> u64 {
        self.new_next_msg_idx
    }

    pub fn new_proof_state(&self) -> Buf32 {
        self.new_proof_state
    }

    pub fn extra_data(&self) -> &[u8] {
        &self.extra_data
    }
}

/// Log that acknowledges deposit. The account this deposits to is in the log context.
#[derive(Clone, Debug)]
pub struct DepositAckLogData {
    /// Subject within the account to deposit to.
    subject_addr: Vec<u8>,
    /// Deposit amount.
    amount: BitcoinAmount,
}

impl DepositAckLogData {
    pub fn new(subject_addr: Vec<u8>, amount: BitcoinAmount) -> Self {
        Self {
            subject_addr,
            amount,
        }
    }

    pub fn subject_addr(&self) -> &[u8] {
        &self.subject_addr
    }

    pub fn amount(&self) -> BitcoinAmount {
        self.amount
    }
}

/// Log that acknowledges checkpoint processing.
#[derive(Clone, Debug)]
pub struct CheckpointAckLogData {
    epoch: EpochCommitment,
}

impl CheckpointAckLogData {
    pub fn new(epoch: EpochCommitment) -> Self {
        Self { epoch }
    }

    pub fn epoch(&self) -> EpochCommitment {
        self.epoch
    }
}

/// OL log emitter.
pub trait LogEmitter {
    /// Emits a single log.
    fn emit_log(&self, log: OLLog);

    /// Emits multiple logs.
    fn emit_logs(&self, logs: impl IntoIterator<Item = OLLog>);
}
