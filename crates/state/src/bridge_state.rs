//! Bridge state types.
//!
//! This just implements a very simple n-of-n multisig bridge.  It will be
//! extended to a more sophisticated design when we have that specced out.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_primitives::{
    bitcoin_bosd::Descriptor,
    bridge::{BitcoinBlockHeight, OperatorIdx},
    buf::Buf32,
    l1::{BitcoinAmount, OutputRef},
    operator::{OperatorKeyProvider, OperatorPubkeys},
};

/// Entry for an operator.
///
/// Each operator has:
///
/// * an `idx` which is used to identify operators uniquely.
/// * a `signing_pk` which is a [`Buf32`] key used to sign messages sent among each other.
/// * a `wallet_pk` which is a [`Buf32`] [`XOnlyPublickey`](bitcoin::secp256k1::XOnlyPublicKey) used
///   to sign bridge transactions.
///
/// # Note
///
/// The separation between the two keys is so that we can use a different signing mechanism for
/// signing messages in the future. For the present, only the `wallet_pk` is used.
///
/// Also note that the `wallet_pk` corresponds to a [`PublicKey`](bitcoin::secp256k1::PublicKey)
/// with an even parity as per [BIP 340](https://github.com/bitcoin/bips/blob/master/bip-0340.mediawiki#design).
#[derive(
    Clone, Debug, Eq, PartialEq, Hash, BorshDeserialize, BorshSerialize, Serialize, Deserialize,
)]
pub struct OperatorEntry {
    /// Global operator index.
    idx: OperatorIdx,

    /// Pubkey used to verify signed messages from the operator.
    signing_pk: Buf32,

    /// Wallet pubkey used to compute MuSig2 pubkey from a set of operators.
    wallet_pk: Buf32,
}

impl OperatorEntry {
    pub const fn idx(&self) -> OperatorIdx {
        self.idx
    }

    /// Get pubkey used to verify signed messages from the operator.
    pub const fn signing_pk(&self) -> &Buf32 {
        &self.signing_pk
    }

    /// Get wallet pubkey used to compute MuSig2 pubkey from a set of operators.
    pub const fn wallet_pk(&self) -> &Buf32 {
        &self.wallet_pk
    }
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct OperatorTable {
    /// Next unassigned operator index.
    next_idx: OperatorIdx,

    /// Operator table.
    ///
    /// MUST be sorted by `idx`.
    operators: Vec<OperatorEntry>,
}

impl OperatorTable {
    pub const fn new_empty() -> Self {
        Self {
            next_idx: 0,
            operators: Vec::new(),
        }
    }

    /// Constructs an operator table from a list of operator indexes.
    pub fn from_operator_list(entries: &[OperatorPubkeys]) -> Self {
        Self {
            next_idx: entries.len() as OperatorIdx,
            operators: entries
                .iter()
                .enumerate()
                .map(|(i, e)| OperatorEntry {
                    idx: i as OperatorIdx,
                    signing_pk: *e.signing_pk(),
                    wallet_pk: *e.wallet_pk(),
                })
                .collect(),
        }
    }

    /// Sanity checks the operator table for sensibility.
    #[allow(dead_code)] // FIXME: remove this.
    fn sanity_check(&self) {
        if !self.operators.is_sorted_by_key(|e| e.idx) {
            panic!("bridge_state: operators list not sorted");
        }

        if let Some(e) = self.operators.last() {
            if self.next_idx <= e.idx {
                panic!("bridge_state: operators next_idx before last entry");
            }
        }
    }

    /// Returns the number of operator entries.
    pub fn len(&self) -> u32 {
        self.operators.len() as u32
    }

    /// Returns if the operator table is empty.  This is practically probably
    /// never going to be true.
    pub fn is_empty(&self) -> bool {
        self.operators.is_empty()
    }

    pub fn operators(&self) -> &[OperatorEntry] {
        &self.operators
    }

    /// Inserts a new operator entry.
    pub fn insert(&mut self, signing_pk: Buf32, wallet_pk: Buf32) {
        let entry = OperatorEntry {
            idx: {
                let idx = self.next_idx;
                self.next_idx += 1;
                idx
            },
            signing_pk,
            wallet_pk,
        };
        self.operators.push(entry);
    }

    /// Gets an operator from the table by its idx.
    ///
    /// Does a binary search.
    pub fn get_operator(&self, idx: u32) -> Option<&OperatorEntry> {
        self.operators
            .binary_search_by_key(&idx, |e| e.idx)
            .ok()
            .map(|i| &self.operators[i])
    }

    /// Gets a operator entry by its internal position, *ignoring* the indexes.
    pub fn get_entry_at_pos(&self, pos: u32) -> Option<&OperatorEntry> {
        self.operators.get(pos as usize)
    }

    /// Get all the operator's index
    pub fn indices(&self) -> impl Iterator<Item = OperatorIdx> + '_ {
        self.operators.iter().map(|operator| operator.idx)
    }
}

impl OperatorKeyProvider for OperatorTable {
    fn get_operator_signing_pk(&self, idx: OperatorIdx) -> Option<Buf32> {
        // TODO: use the `signing_pk` here if we decide to use a different signing scheme for
        // signing messages.
        self.get_operator(idx).map(|ent| ent.wallet_pk)
    }
}

impl<'a> arbitrary::Arbitrary<'a> for OperatorTable {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let o0 = OperatorEntry {
            idx: 0,
            signing_pk: Buf32::arbitrary(u)?,
            wallet_pk: Buf32::arbitrary(u)?,
        };

        let o1 = OperatorEntry {
            idx: 1,
            signing_pk: Buf32::arbitrary(u)?,
            wallet_pk: Buf32::arbitrary(u)?,
        };

        Ok(Self {
            next_idx: 2,
            operators: vec![o0, o1],
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct DepositsTable {
    /// Next unassigned deposit index.
    next_idx: u32,

    /// Deposit table.
    ///
    /// MUST be sorted by `deposit_idx`.
    deposits: Vec<DepositEntry>,
}

impl DepositsTable {
    pub const fn new_empty() -> Self {
        Self {
            next_idx: 0,
            deposits: Vec::new(),
        }
    }

    /// Sanity checks the operator table for sensibility.
    #[allow(dead_code)] // FIXME: remove this.
    fn sanity_check(&self) {
        if !self.deposits.is_sorted_by_key(|e| e.deposit_idx) {
            panic!("bridge_state: deposits list not sorted");
        }

        if let Some(e) = self.deposits.last() {
            if self.next_idx <= e.deposit_idx {
                panic!("bridge_state: deposits next_idx before last entry");
            }
        }
    }

    /// Returns the number of deposit entries being tracked.
    pub fn len(&self) -> u32 {
        self.deposits.len() as u32
    }

    /// Returns if the deposit table is empty.  This is practically probably
    /// never going to be true.
    pub fn is_empty(&self) -> bool {
        self.len() > 0
    }

    /// Gets the position in the deposit table of a hypothetical deposit entry
    /// index.
    pub fn get_deposit_entry_pos(&self, idx: u32) -> Result<u32, u32> {
        self.deposits
            .binary_search_by_key(&idx, |e| e.deposit_idx)
            .map(|i| i as u32)
            .map_err(|i| i as u32)
    }

    /// Gets a deposit from the table by its idx.
    ///
    /// Does a binary search.
    pub fn get_deposit(&self, idx: u32) -> Option<&DepositEntry> {
        self.get_deposit_entry_pos(idx)
            .ok()
            .map(|i| &self.deposits[i as usize])
    }

    /// Gets a mut ref to a deposit from the table by its idx.
    ///
    /// Does a binary search.
    pub fn get_deposit_mut(&mut self, idx: u32) -> Option<&mut DepositEntry> {
        self.get_deposit_entry_pos(idx)
            .ok()
            .map(|i| &mut self.deposits[i as usize])
    }

    pub fn get_all_deposits_idxs_iters_iter(&self) -> impl Iterator<Item = u32> + '_ {
        self.deposits.iter().map(|e| e.deposit_idx)
    }

    /// Gets a deposit entry by its internal position, *ignoring* the indexes.
    pub fn get_entry_at_pos(&self, pos: u32) -> Option<&DepositEntry> {
        self.deposits.get(pos as usize)
    }

    /// Adds a new deposit to the table and returns the index of the new deposit.
    pub fn create_next_deposit(
        &mut self,
        tx_ref: OutputRef,
        operators: Vec<OperatorIdx>,
        amt: BitcoinAmount,
    ) -> u32 {
        let idx = self.next_idx();
        let deposit_entry = DepositEntry::new(idx, tx_ref, operators, amt, None);
        self.deposits.push(deposit_entry);
        self.next_idx += 1;
        idx
    }

    /// Tries to create a deposit entry at a specific idx.  If the entry requested if after the
    /// `next_entry`, then updates it to be equal to that.
    ///
    /// Returns if we inserted it successfully.
    pub fn try_create_deposit_at(
        &mut self,
        idx: u32,
        tx_ref: OutputRef,
        operators: Vec<OperatorIdx>,
        amt: BitcoinAmount,
    ) -> bool {
        // Happy case, if we're creating the next entry we can skip the binary
        // search.  This should be most cases, where there isn't concurrent
        // interleaved deposit processing.
        if idx == self.next_idx {
            self.create_next_deposit(tx_ref, operators, amt);
            return true;
        }

        // Slow path.
        match self.get_deposit_entry_pos(idx) {
            Ok(_) => false,
            Err(pos) => {
                let entry = DepositEntry::new(idx, tx_ref, operators, amt, None);
                self.deposits.insert(pos as usize, entry);

                // Tricky bookkeeping.
                if idx >= self.next_idx {
                    self.next_idx = u32::max(self.next_idx, idx) + 1;
                }

                true
            }
        }
    }

    pub const fn next_idx(&self) -> u32 {
        self.next_idx
    }

    pub fn deposits(&self) -> impl Iterator<Item = &DepositEntry> {
        self.deposits.iter()
    }
}

/// Container for the state machine of a deposit factory.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
pub struct DepositEntry {
    deposit_idx: u32,

    /// The outpoint that this deposit entry references.
    output: OutputRef,

    /// List of notary operators, by their indexes.
    // TODO convert this to a windowed bitmap or something
    notary_operators: Vec<OperatorIdx>,

    /// Deposit amount, in the native asset.
    amt: BitcoinAmount,

    /// Deposit state.
    state: DepositState,

    /// Withdrawal request transaction id
    withdrawal_request_txid: Option<Buf32>,
}

impl DepositEntry {
    pub fn new(
        idx: u32,
        output: OutputRef,
        operators: Vec<OperatorIdx>,
        amt: BitcoinAmount,
        withdrawal_request_txid: Option<Buf32>,
    ) -> Self {
        Self {
            deposit_idx: idx,
            output,
            notary_operators: operators,
            amt,
            state: DepositState::Accepted,
            withdrawal_request_txid,
        }
    }

    pub const fn idx(&self) -> u32 {
        self.deposit_idx
    }

    pub const fn output(&self) -> &OutputRef {
        &self.output
    }

    pub fn notary_operators(&self) -> &[OperatorIdx] {
        &self.notary_operators
    }

    pub const fn amt(&self) -> BitcoinAmount {
        self.amt
    }

    pub const fn deposit_state(&self) -> &DepositState {
        &self.state
    }

    pub const fn deposit_state_mut(&mut self) -> &mut DepositState {
        &mut self.state
    }

    pub fn set_state(&mut self, new_state: DepositState) {
        self.state = new_state;
    }

    pub const fn withdrawal_request_txid(&self) -> Option<Buf32> {
        self.withdrawal_request_txid
    }

    pub const fn set_withdrawal_request_txid(&mut self, new_wr_txid: Option<Buf32>) {
        self.withdrawal_request_txid = new_wr_txid;
    }
}

#[cfg(feature = "test_utils")]
impl DepositEntry {
    pub fn with_state(mut self, state: DepositState) -> Self {
        self.state = state;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DepositState {
    /// Deposit utxo has been recognized.
    Created(CreatedState),

    /// Deposit utxo has been accepted.
    Accepted,

    /// Order to send out withdrawal dispatched.
    Dispatched(DispatchedState),

    /// Withdrawal is being processed by the assigned operator.
    Fulfilled(FulfilledState),

    /// Executed state, will be cleaned up.
    Reimbursed,
}

impl DepositState {
    pub const fn is_dispatched_to(&self, operator_idx: u32) -> bool {
        matches!(self, DepositState::Dispatched(s) if s.assignee() == operator_idx)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
pub struct CreatedState {
    /// Destination identifier in EL, probably an encoded address.
    dest_ident: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
pub struct DispatchedState {
    /// Configuration for outputs to be written to.
    cmd: DispatchCommand,

    /// The index of the operator that's fronting the funds for the withdrawal,
    /// and who will be reimbursed by the bridge notaries.
    assignee: OperatorIdx,

    /// L1 block height before which we expect the dispatch command to be
    /// executed and after which this assignment command is no longer valid.
    ///
    /// If a checkpoint is processed for this L1 height and the withdrawal still
    /// goes out it won't be honored.
    exec_deadline: BitcoinBlockHeight,
}

impl DispatchedState {
    pub fn new(
        cmd: DispatchCommand,
        assignee: OperatorIdx,
        exec_deadline: BitcoinBlockHeight,
    ) -> Self {
        Self {
            cmd,
            assignee,
            exec_deadline,
        }
    }

    pub const fn cmd(&self) -> &DispatchCommand {
        &self.cmd
    }

    pub const fn assignee(&self) -> OperatorIdx {
        self.assignee
    }

    pub const fn exec_deadline(&self) -> BitcoinBlockHeight {
        self.exec_deadline
    }

    pub const fn set_assignee(&mut self, assignee_op_idx: OperatorIdx) {
        self.assignee = assignee_op_idx;
    }

    pub const fn set_exec_deadline(&mut self, exec_deadline: BitcoinBlockHeight) {
        self.exec_deadline = exec_deadline;
    }
}

/// Command to operator(s) to initiate the withdrawal.  Describes the set of
/// outputs we're trying to withdraw to.
///
/// May also include future information to deal with fee accounting.
///
/// # Note
///
/// This is mostly here in order to support withdrawal batching (i.e., sub-denomination withdrawal
/// amounts that can be batched and then serviced together). At the moment, the underlying `Vec` of
/// [`WithdrawOutput`] always has a single element.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
pub struct DispatchCommand {
    /// The table of withdrawal outputs.
    withdraw_outputs: Vec<WithdrawOutput>,
}

impl DispatchCommand {
    pub fn new(withdraw_outputs: Vec<WithdrawOutput>) -> Self {
        Self { withdraw_outputs }
    }

    pub fn withdraw_outputs(&self) -> &[WithdrawOutput] {
        &self.withdraw_outputs
    }
}

/// An output constructed from [`crate::bridge_ops::WithdrawalIntent`].
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WithdrawOutput {
    /// BOSD [`Descriptor`].
    destination: Descriptor,

    /// Amount in sats.
    amt: BitcoinAmount,
}

impl WithdrawOutput {
    pub fn new(destination: Descriptor, amt: BitcoinAmount) -> Self {
        Self { destination, amt }
    }

    pub const fn destination(&self) -> &Descriptor {
        &self.destination
    }

    pub const fn amt(&self) -> BitcoinAmount {
        self.amt
    }
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
pub struct FulfilledState {
    /// The index of the operator that has fronted the funds for the withdrawal,
    /// and who will be reimbursed by the bridge notaries.
    assignee: OperatorIdx,

    /// Actual amount sent in withdrawal
    amt: BitcoinAmount,

    /// Corresponding bitcoin transaction id
    txid: Buf32,
}

impl FulfilledState {
    pub fn new(assignee: OperatorIdx, amt: BitcoinAmount, txid: Buf32) -> Self {
        Self {
            assignee,
            amt,
            txid,
        }
    }

    pub const fn assignee(&self) -> OperatorIdx {
        self.assignee
    }

    pub const fn amt(&self) -> BitcoinAmount {
        self.amt
    }
}
