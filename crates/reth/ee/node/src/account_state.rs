use alloy_primitives::FixedBytes;
use alpen_ee_primitives::{BitcoinAmount, L1BlockCommitment, OlBlockCommitment};

/// Representation of inner account state of EE
/// In this implementation, this equals the blockhash of the final ee block of the update
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AccountStateCommitment(FixedBytes<32>);

impl From<FixedBytes<32>> for AccountStateCommitment {
    fn from(value: FixedBytes<32>) -> Self {
        Self(value)
    }
}

/// Representation of state of EE Account in OL state as needed by EE
#[derive(Debug, Clone)]
pub struct Account {
    sequence_no: u64,
    balance: BitcoinAmount,
    state: AccountStateCommitment,
}

impl Account {
    pub fn new(sequence_no: u64, balance: BitcoinAmount, state: AccountStateCommitment) -> Self {
        Self {
            sequence_no,
            balance,
            state,
        }
    }

    pub fn sequence_no(&self) -> u64 {
        self.sequence_no
    }

    pub fn balance(&self) -> &BitcoinAmount {
        &self.balance
    }

    pub fn state(&self) -> &AccountStateCommitment {
        &self.state
    }
}

/// Representation of OL state at a particular block
#[derive(Debug, Clone)]
pub struct OlState {
    /// state of own ee account
    account_state: Account,
    /// OL block corresponding to this state
    block: OlBlockCommitment,
    /// Last L1 block whose data (ASM log/messages) were included in the ol block.
    /// Corresponds to the last L1 block of the range whose ASM Logs were included in the previous
    /// OL Epoch.
    checkedin_l1: L1BlockCommitment,
    /// L1 block containing the checkpoint where this block's corresponding epoch is included.
    /// `None` only for latest blocks pending checkpoint
    checkpointed_l1: Option<L1BlockCommitment>,
}

impl OlState {
    pub fn new(
        account_state: Account,
        block: OlBlockCommitment,
        checkedin_l1: L1BlockCommitment,
        checkpointed_l1: Option<L1BlockCommitment>,
    ) -> Self {
        Self {
            account_state,
            block,
            checkedin_l1,
            checkpointed_l1,
        }
    }

    pub fn account_state(&self) -> &Account {
        &self.account_state
    }

    pub fn block(&self) -> &OlBlockCommitment {
        &self.block
    }

    pub fn checkedin_l1(&self) -> &L1BlockCommitment {
        &self.checkedin_l1
    }

    pub fn checkpointed_l1(&self) -> &Option<L1BlockCommitment> {
        &self.checkpointed_l1
    }
}
