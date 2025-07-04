// TODO: This needs to be in a different crate. Maybe strata-crypto
pub mod msg;
pub mod multisig_config;
pub mod vote;

use strata_primitives::buf::{Buf32, Buf64};

use crate::error::VoteValidationError;

pub type PubKey = Buf32;
pub type Signature = Buf64;

// FIXME: handle
pub fn aggregate_pubkeys(_keys: &[PubKey]) -> Result<PubKey, VoteValidationError> {
    Ok(PubKey::default())
}

// FIXME: handle
pub fn verify_sig(_pk: &PubKey, _msg_hash: &Buf32, _sig: &Signature) -> bool {
    true
}
