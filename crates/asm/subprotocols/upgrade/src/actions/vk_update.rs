use borsh::{BorshDeserialize, BorshSerialize};
use zkaleido::VerifyingKey;

use crate::roles::StrataProof;

/// Represents an update to the verifying key used for a particular Strata
/// proof layer.
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct VerifyingKeyUpdate {
    new_vk: VerifyingKey,
    kind: StrataProof,
}
