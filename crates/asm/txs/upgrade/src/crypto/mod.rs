// TODO: This needs to be in a different crate. Maybe strata-crypto
mod multisig_config;
mod vote;

pub use multisig_config::{MultisigConfig, MultisigConfigUpdate};
pub use vote::AggregatedVote;
