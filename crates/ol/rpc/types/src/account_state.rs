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
    /// Serial assigned at creation.
    serial: u32,
    /// Balance in sats.
    balance_sats: u64,
    /// Account-type discriminator with type-specific fields inline.
    kind: RpcAccountKind,
}

impl RpcAccountEntry {
    pub fn id(&self) -> &HexBytes32 {
        &self.id
    }

    pub fn serial(&self) -> u32 {
        self.serial
    }

    pub fn balance_sats(&self) -> u64 {
        self.balance_sats
    }

    pub fn kind(&self) -> &RpcAccountKind {
        &self.kind
    }

    /// Convenience accessor: returns the snark summary when the account
    /// is a snark account, `None` otherwise.
    pub fn snark(&self) -> Option<&RpcAccountSnarkSummary> {
        match &self.kind {
            RpcAccountKind::Snark(summary) => Some(summary),
            RpcAccountKind::Empty => None,
        }
    }
}

impl From<&TsnlAccountEntry> for RpcAccountEntry {
    fn from(entry: &TsnlAccountEntry) -> Self {
        let state = entry.state();
        let kind = match state.as_snark_account().ok() {
            Some(snark) => RpcAccountKind::Snark(RpcAccountSnarkSummary {
                seq_no: *snark.seqno().inner(),
                inner_state_root: HexBytes32::from(snark.inner_state_root().0),
                next_inbox_msg_idx: snark.next_inbox_msg_idx(),
            }),
            None => RpcAccountKind::Empty,
        };
        Self {
            id: HexBytes32::from(<[u8; 32]>::from(entry.id())),
            serial: *state.serial().inner(),
            balance_sats: state.balance().to_sat(),
            kind,
        }
    }
}

/// Account-type discriminator for [`RpcAccountEntry`].
///
/// Mirrors the on-chain `OLAccountTypeState` and carries snark-specific
/// summary data inline so the wire format is self-describing.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RpcAccountKind {
    Empty,
    Snark(RpcAccountSnarkSummary),
}

/// Snark-account summary fields surfaced in account listings.
///
/// Distinct from [`RpcSnarkAccountState`] in that it omits the update verification
/// key, which is not available from the runtime account state.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct RpcAccountSnarkSummary {
    seq_no: u64,
    inner_state_root: HexBytes32,
    next_inbox_msg_idx: u64,
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
}

/// Paginated response for `strata_listAccounts`.
///
/// Wraps a slice of accounts plus pagination metadata so callers can
/// iterate the ledger without forcing the server to materialize every
/// entry in a single response.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct RpcAccountListPage {
    /// Accounts in the requested page, in ascending account-id order.
    entries: Vec<RpcAccountEntry>,
    /// Total number of accounts on the ledger at the queried block.
    total: u64,
    /// Offset to pass for the next page, or `None` if this is the last page.
    next_offset: Option<u64>,
}

impl RpcAccountListPage {
    pub fn new(entries: Vec<RpcAccountEntry>, total: u64, next_offset: Option<u64>) -> Self {
        Self {
            entries,
            total,
            next_offset,
        }
    }

    pub fn entries(&self) -> &[RpcAccountEntry] {
        &self.entries
    }

    pub fn total(&self) -> u64 {
        self.total
    }

    pub fn next_offset(&self) -> Option<u64> {
        self.next_offset
    }
}
