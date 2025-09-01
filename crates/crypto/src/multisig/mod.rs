// TODO: This needs to be in a different crate. Maybe strata-crypto
pub mod config;
pub mod errors;
pub mod msg;
pub mod vote;

use strata_primitives::buf::{Buf32, Buf64};

use crate::multisig::{errors::VoteValidationError, msg::MultisigPayload};

pub type PubKey = Buf32;
pub type Signature = Buf64;

// FIXME: handle
pub fn aggregate_pubkeys(_keys: &[PubKey]) -> Result<PubKey, VoteValidationError> {
    Ok(PubKey::default())
}

// FIXME: handle
pub fn verify_sig(_pk: &PubKey, _payload: &MultisigPayload, _sig: &Signature) -> bool {
    true
}
