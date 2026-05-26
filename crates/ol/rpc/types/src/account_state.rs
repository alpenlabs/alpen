use serde::{Deserialize, Serialize};
use strata_ledger_types::{IAccountState, ISnarkAccountState};
use strata_ol_state_types::TsnlAccountEntry;
use strata_predicate::PredicateKey;
use strata_primitives::HexBytes32;

/// Snark account state for RPC responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct RpcSnarkAccountState {
    /// Account sequence number.
    seq_no: u64,
    /// Merkle root of the account state.
    inner_state: HexBytes32,
    /// Index of the next inbox message to process.
    next_inbox_msg_idx: u64,
    /// Snark account update verification key.
    #[cfg_attr(feature = "jsonschema", schemars(with = "String"))]
    update_vk: PredicateKey,
}

impl RpcSnarkAccountState {
    /// Creates a new `RpcSnarkAccountState`.
    pub fn new(
        seq_no: u64,
        inner_state: HexBytes32,
        next_inbox_msg_idx: u64,
        update_vk: PredicateKey,
    ) -> Self {
        Self {
            seq_no,
            inner_state,
            next_inbox_msg_idx,
            update_vk,
        }
    }

    /// Returns the account sequence number.
    pub fn seq_no(&self) -> u64 {
        self.seq_no
    }

    /// Returns the state root.
    pub fn inner_state(&self) -> &HexBytes32 {
        &self.inner_state
    }

    /// Returns the next inbox message index.
    pub fn next_inbox_msg_idx(&self) -> u64 {
        self.next_inbox_msg_idx
    }

    /// Returns the update verification key.
    pub fn update_vk(&self) -> &PredicateKey {
        &self.update_vk
    }
}

/// Account list entry returned by `strata_listAccounts`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct RpcAccountEntry {
    /// Account ID.
    id: HexBytes32,
    /// Account state at the queried block.
    state: RpcAccountState,
}

impl RpcAccountEntry {
    pub fn id(&self) -> &HexBytes32 {
        &self.id
    }

    pub fn state(&self) -> &RpcAccountState {
        &self.state
    }
}

impl From<&TsnlAccountEntry> for RpcAccountEntry {
    fn from(entry: &TsnlAccountEntry) -> Self {
        Self {
            id: HexBytes32::from(<[u8; 32]>::from(entry.id())),
            state: RpcAccountState::from(entry.state()),
        }
    }
}

/// Account state at a block.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct RpcAccountState {
    /// Serial assigned at creation.
    serial: u32,
    /// Balance in sats.
    balance_sats: u64,
    /// Account-type discriminator with type-specific fields inline.
    #[serde(flatten)]
    type_data: RpcAccountTypeData,
}

impl RpcAccountState {
    pub fn serial(&self) -> u32 {
        self.serial
    }

    pub fn balance_sats(&self) -> u64 {
        self.balance_sats
    }

    pub fn type_data(&self) -> &RpcAccountTypeData {
        &self.type_data
    }

    /// Convenience accessor: returns the snark summary when the account
    /// is a snark account, `None` otherwise.
    pub fn snark(&self) -> Option<&RpcAccountSnarkSummary> {
        match &self.type_data {
            RpcAccountTypeData::Snark(summary) => Some(summary),
            RpcAccountTypeData::Empty => None,
        }
    }
}

impl<S> From<&S> for RpcAccountState
where
    S: IAccountState,
{
    fn from(state: &S) -> Self {
        let type_data = match state.as_snark_account().ok() {
            Some(snark) => RpcAccountTypeData::Snark(RpcAccountSnarkSummary {
                seq_no: *snark.seqno().inner(),
                inner_state_root: HexBytes32::from(snark.inner_state_root().0),
                next_inbox_msg_idx: snark.next_inbox_msg_idx(),
                update_vk: snark.update_vk().clone(),
            }),
            None => RpcAccountTypeData::Empty,
        };
        Self {
            serial: *state.serial().inner(),
            balance_sats: state.balance().to_sat(),
            type_data,
        }
    }
}

/// Account-type discriminator for [`RpcAccountState`].
///
/// Mirrors the on-chain `OLAccountTypeState` and carries snark-specific
/// summary data inline so the wire format is self-describing.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RpcAccountTypeData {
    Empty,
    Snark(RpcAccountSnarkSummary),
}

/// Snark-account summary fields surfaced in account listings.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct RpcAccountSnarkSummary {
    seq_no: u64,
    inner_state_root: HexBytes32,
    next_inbox_msg_idx: u64,
    #[cfg_attr(feature = "jsonschema", schemars(with = "String"))]
    update_vk: PredicateKey,
}

impl RpcAccountSnarkSummary {
    pub fn seq_no(&self) -> u64 {
        self.seq_no
    }

    pub fn inner_state_root(&self) -> &HexBytes32 {
        &self.inner_state_root
    }

    pub fn next_inbox_msg_idx(&self) -> u64 {
        self.next_inbox_msg_idx
    }

    pub fn update_vk(&self) -> &PredicateKey {
        &self.update_vk
    }
}
