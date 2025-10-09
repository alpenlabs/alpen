use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_identifiers::Buf32;

/// Container for operator pubkeys.
#[derive(Clone, Debug, PartialEq, Eq, BorshDeserialize, BorshSerialize, Deserialize, Serialize)]
pub struct OperatorPubkeys {
    signing_pk: Buf32,
    wallet_pk: Buf32,
}

impl OperatorPubkeys {
    pub fn new(signing_pk: Buf32, wallet_pk: Buf32) -> Self {
        Self {
            signing_pk,
            wallet_pk,
        }
    }

    pub fn signing_pk(&self) -> &Buf32 {
        &self.signing_pk
    }

    pub fn wallet_pk(&self) -> &Buf32 {
        &self.wallet_pk
    }

    pub fn into_parts(self) -> (Buf32, Buf32) {
        (self.signing_pk, self.wallet_pk)
    }
}
