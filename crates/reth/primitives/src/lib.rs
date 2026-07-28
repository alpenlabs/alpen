//! Primitives for Reth.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use std::mem::size_of;

use alloy_primitives::FixedBytes;
use alloy_sol_types::sol;
use serde::{Deserialize, Serialize};
use strata_acct_types::AccountId;
use strata_identifiers::{SubjectId, SUBJ_ID_LEN};
use strata_ol_bridge_types::OperatorSelection;
use strata_primitives::bitcoin_bosd::Descriptor;

/// Type for withdrawal_intents in rpc.
/// Distinct from `strata_ol_bridge_types::WithdrawalIntent`
/// as this will live in reth repo eventually
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct WithdrawalIntent {
    /// Amount to be withdrawn in sats.
    pub amt: u64,

    /// User's operator selection for withdrawal assignment.
    pub selected_operator: OperatorSelection,

    /// Dynamic-sized bytes BOSD descriptor for the withdrawal destinations in L1.
    pub destination: Descriptor,
}

sol! {
    event WithdrawalIntentEvent(
        /// Withdrawal amount in sats.
        uint64 amount,
        /// Selected operator index. `u32::MAX` means no selection.
        uint32 selectedOperator,
        /// BOSD descriptor for withdrawal destinations in L1.
        bytes destination,
    );
}

/// Structured calldata for the bridge-out withdrawal precompile.
///
/// Wire format: `[4 bytes: operator index (big-endian u32)][BOSD bytes]`
/// - `u32::MAX` (`0xFFFFFFFF`): no operator selection (any operator)
/// - Any other value: specific operator index
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WithdrawalCalldata {
    /// User's operator selection for withdrawal assignment.
    pub selected_operator: OperatorSelection,

    /// Raw BOSD descriptor bytes.
    pub bosd: Vec<u8>,
}

/// Size of the operator index field in calldata (u32 = 4 bytes).
const OPERATOR_INDEX_SIZE: usize = size_of::<u32>();

impl WithdrawalCalldata {
    /// Encodes the calldata to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(OPERATOR_INDEX_SIZE + self.bosd.len());
        buf.extend_from_slice(&self.selected_operator.raw().to_be_bytes());
        buf.extend_from_slice(&self.bosd);
        buf
    }

    /// Decodes calldata from bytes.
    ///
    /// Returns `None` if the data is too short (needs at least 5 bytes: 4 operator + 1 BOSD).
    pub fn decode(data: &[u8]) -> Option<Self> {
        if data.len() <= OPERATOR_INDEX_SIZE {
            return None;
        }

        let (operator_bytes, bosd) = data.split_at(OPERATOR_INDEX_SIZE);
        let raw = u32::from_be_bytes(operator_bytes.try_into().expect("exactly 4 bytes"));

        Some(Self {
            selected_operator: OperatorSelection::from_raw(raw),
            bosd: bosd.to_vec(),
        })
    }
}

/// Structured calldata for the inter-EE subject-transfer precompile.
///
/// Wire format: `[32 bytes: destination account][32 bytes: destination subject][data...]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubjectTransferCalldata {
    /// OL account that should receive the subject-transfer message.
    pub dest_account: AccountId,

    /// Subject that should receive the value in the destination EE.
    pub dest_subject: SubjectId,

    /// Opaque transfer payload delivered with the subject-transfer message.
    pub data: Vec<u8>,
}

/// Size of the fixed destination account and subject fields.
const SUBJECT_TRANSFER_FIXED_FIELDS_SIZE: usize = size_of::<[u8; SUBJ_ID_LEN]>() * 2;

impl SubjectTransferCalldata {
    /// Encodes the calldata to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(SUBJECT_TRANSFER_FIXED_FIELDS_SIZE + self.data.len());
        buf.extend_from_slice(self.dest_account.inner());
        buf.extend_from_slice(self.dest_subject.inner());
        buf.extend_from_slice(&self.data);
        buf
    }

    /// Decodes calldata from bytes.
    ///
    /// Returns `None` if the data is too short to hold the fixed account and subject fields.
    pub fn decode(data: &[u8]) -> Option<Self> {
        if data.len() < SUBJECT_TRANSFER_FIXED_FIELDS_SIZE {
            return None;
        }

        let (dest_account, rest) = data.split_at(SUBJ_ID_LEN);
        let (dest_subject, data) = rest.split_at(SUBJ_ID_LEN);

        Some(Self {
            dest_account: AccountId::new(dest_account.try_into().expect("exactly 32 bytes")),
            dest_subject: SubjectId::new(dest_subject.try_into().expect("exactly 32 bytes")),
            data: data.to_vec(),
        })
    }
}

sol! {
    event SubjectTransferIntentEvent(
        /// Transfer amount in sats.
        uint64 amount,
        /// Source subject derived from the EVM caller.
        bytes32 sourceSubject,
        /// Destination OL account.
        bytes32 destAccount,
        /// Destination subject inside the destination EE.
        bytes32 destSubject,
        /// Opaque transfer payload.
        bytes transferData,
    );
}

/// Converts a 32-byte account ID into a Solidity `bytes32` value.
pub fn account_id_to_bytes32(account_id: &AccountId) -> FixedBytes<32> {
    FixedBytes::from(*account_id.inner())
}

/// Converts a 32-byte subject ID into a Solidity `bytes32` value.
pub fn subject_id_to_bytes32(subject_id: &SubjectId) -> FixedBytes<32> {
    FixedBytes::from(*subject_id.inner())
}
