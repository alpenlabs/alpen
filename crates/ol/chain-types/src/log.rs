use strata_acct_types::{AccountId, BitcoinAmount};
use strata_primitives::{Buf32, EpochCommitment};

/// Log emitted during OL block execution.
#[derive(Clone, Debug)]
pub struct OLLog {
    /// Account this log is related to.
    // TODO: maybe use account serial,
    account_id: AccountId,

    /// Opaque log payload.
    log_type: LogType,
    // TODO: add more concrete fields.
}

impl OLLog {
    pub fn new(account_id: AccountId, log_type: LogType) -> Self {
        Self {
            account_id,
            log_type,
        }
    }

    pub fn account_id(&self) -> AccountId {
        self.account_id
    }

    pub fn log_type(&self) -> &LogType {
        &self.log_type
    }

    /// Create a withdrawal intent log.
    pub fn withdrawal_intent(account_id: AccountId, amount: BitcoinAmount, dest: Vec<u8>) -> Self {
        Self::new(
            account_id,
            LogType::WithdrawalIntent(WithdrawalIntentLog::new(amount, dest)),
        )
    }

    /// Create a snark account update log.
    pub fn snark_account_update(
        account_id: AccountId,
        from_msg_idx: u64,
        to_msg_idx: u64,
        new_proof_state: Buf32,
        extra_data: Vec<u8>,
    ) -> Self {
        Self::new(
            account_id,
            LogType::SnarkAccountUpdate(SnarkAccountUpdateLog::new(
                from_msg_idx,
                to_msg_idx,
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
            LogType::DepositAck(DepositAckLog::new(subject_addr, amount)),
        )
    }

    /// Create a checkpoint acknowledgment log.
    pub fn checkpoint_ack(account_id: AccountId, epoch: EpochCommitment) -> Self {
        Self::new(
            account_id,
            LogType::CheckpointAck(CheckpointAckLog::new(epoch)),
        )
    }
}

pub fn compute_logs_root(_logs: &[OLLog]) -> Buf32 {
    // TODO:
    todo!()
}

/// Structured representation of the type of log.
#[derive(Clone, Debug)]
pub enum LogType {
    WithdrawalIntent(WithdrawalIntentLog),
    SnarkAccountUpdate(SnarkAccountUpdateLog),
    DepositAck(DepositAckLog),
    CheckpointAck(CheckpointAckLog),
}

/// Log representing intent to withdraw amount by an account. Account is implied from the log
/// context.
#[derive(Clone, Debug)]
pub struct WithdrawalIntentLog {
    /// Amount intended to withdraw.
    amount: BitcoinAmount,

    /// Withdrawal destination.
    dest: Vec<u8>, // TODO: use bosd?
}

impl WithdrawalIntentLog {
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
pub struct SnarkAccountUpdateLog {
    /// Start of the message index that was processed.
    from_msg_idx: u64,

    /// End of the message index that was processed.
    to_msg_idx: u64,

    /// New proof state after the update.
    new_proof_state: Buf32,

    /// Any extra data
    extra_data: Vec<u8>,
}

impl SnarkAccountUpdateLog {
    pub fn new(
        from_msg_idx: u64,
        to_msg_idx: u64,
        new_proof_state: Buf32,
        extra_data: Vec<u8>,
    ) -> Self {
        Self {
            from_msg_idx,
            to_msg_idx,
            new_proof_state,
            extra_data,
        }
    }

    pub fn from_msg_idx(&self) -> u64 {
        self.from_msg_idx
    }

    pub fn to_msg_idx(&self) -> u64 {
        self.to_msg_idx
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
pub struct DepositAckLog {
    /// Subject within the account to deposit to.
    subject_addr: Vec<u8>,
    /// Deposit amount.
    amount: BitcoinAmount,
}

impl DepositAckLog {
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
pub struct CheckpointAckLog {
    epoch: EpochCommitment,
}

impl CheckpointAckLog {
    pub fn new(epoch: EpochCommitment) -> Self {
        Self { epoch }
    }

    pub fn epoch(&self) -> EpochCommitment {
        self.epoch
    }
}
