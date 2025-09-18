use strata_primitives::buf::Buf32;

pub type AccountId = Buf32;
pub type AccountVk = Buf32;
pub type AccountSerial = u32;

/// Ledger account state
#[derive(Debug, Clone)]
pub struct AccountState {
    pub serial: AccountSerial,
    /// Account type
    pub ty: u16, // Maybe a separate type
    pub balance: u64, // sats
    pub inner_state: AccountInnerState,
}

/// Account states that correspond to various account types.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum AccountInnerState {
    Snark(SnarkAccountState),
    // add others
}

/// Full update operation description with authentication.
#[derive(Debug, Clone)]
pub struct SnarkAccountUpdate {
    pub data: SnarkAccountUpdateData,
    pub witness: Vec<u8>,
}

/// Description of an update operation, both state changes and conditions.
#[derive(Debug, Clone)]
pub struct SnarkAccountUpdateData {
    pub new_state: SnarkAccountProofState,
    pub seq_no: u64,
    pub processed_msgs: Vec<SnarkAcctMsgProof>,
    pub ledger_refs: LedgerReferences,
    pub outputs: AccountUpdateOutputs,
    pub extra_data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct AccountUpdateOutputs {
    pub output_transfers: Vec<OutputTransfer>,
    pub output_messages: Vec<OutputMessage>,
}

#[derive(Debug, Clone)]
pub struct SnarkAccountProofState {
    pub inner_state_root: Buf32,
    pub next_input_idx: u64,
}

#[derive(Debug, Clone)]
pub struct SnarkAccountState {
    pub update_vk: AccountVk,
    pub proof_state: SnarkAccountProofState,
    pub seq_no: u64,
    // TODO: update this with MMR. This will be accessed via Ledger provider, so just changing the
    // type here should be fine.
    pub input: Vec<SnarkAccountMessageEntry>,
}

/// Message hashed and put into an account's input_mmr.
#[derive(Debug, Clone)]
pub struct SnarkAccountMessageEntry {
    pub source: AccountId,
    pub included_epoch: u64,
    pub data: MessageData,
}

/// Describes a simple value transfer from some account to another.
#[derive(Debug, Clone)]
pub struct OutputTransfer {
    pub destination: AccountId,
    pub transferred_value: u64,
}

/// Container for a message payload with value.
#[derive(Debug, Clone)]
pub struct MessageData {
    pub transferred_value: u64,
    pub payload: Vec<u8>,
}

/// Describes a message with value to be sent from some account to another.
#[derive(Debug, Clone)]
pub struct OutputMessage {
    pub destination: AccountId,
    pub data: MessageData,
}

/// Container for references to other pieces of data in the ledger.
#[derive(Debug, Clone)]
pub struct LedgerReferences {}

/// Observed side effects from an account update.
#[derive(Debug, Clone)]
pub struct SnarkAccountUpdateOutputs {
    pub output_transfers: Option<Vec<OutputTransfer>>,
    pub output_msgs: Option<Vec<OutputMessage>>,
}

/// Proof for an input message in an MMR that can be updated by the OL sequencer
/// without invalidating the transaction's signature.
#[derive(Debug, Clone)]
pub struct SnarkAcctMsgProof<P = () /* This is temporary till we implement mmr */> {
    pub data: SnarkAccountMessageEntry,
    pub proof: P,
}
