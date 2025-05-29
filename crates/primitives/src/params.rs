//! Global consensus parameters for the rollup.

use bitcoin::Network;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    block_credential::CredRule,
    l1::{BitcoinAddress, XOnlyPk},
    operator::OperatorPubkeys,
    prelude::Buf32,
    proof::RollupVerifyingKey,
};

/// Consensus parameters that don't change for the lifetime of the network
/// (unless there's some weird hard fork).
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct RollupParams {
    /// Rollup name
    pub rollup_name: String,

    /// Block time in milliseconds.
    pub block_time: u64,

    /// Tagname used to identify DA envelopes
    pub da_tag: String,

    /// Tagname used to identify Checkpoint envelopes
    pub checkpoint_tag: String,

    /// Rule we use to decide if a block is correctly signed.
    pub cred_rule: CredRule,

    /// Block height from which to watch for L1 transactions
    pub horizon_l1_height: u64,

    /// Block height we'll construct the L2 genesis block from.
    pub genesis_l1_height: u64,

    /// Config for how the genesis operator table is set up.
    pub operator_config: OperatorConfig,

    /// Hardcoded EL genesis info
    /// TODO: move elsewhere
    pub evm_genesis_block_hash: Buf32,
    pub evm_genesis_block_state_root: Buf32,

    /// Depth after which we consider the L1 block to not reorg
    pub l1_reorg_safe_depth: u32,

    /// target batch size in number of l2 blocks
    pub target_l2_batch_size: u64,

    /// Maximum length of an EE address in a deposit.
    // FIXME this should be "max address length"
    pub address_length: u8,

    /// Exact "at-rest" deposit amount, in sats.
    pub deposit_amount: u64,

    /// SP1 verifying key that is used to verify the Groth16 proof posted on Bitcoin
    // FIXME which proof?  should this be `checkpoint_vk`?
    pub rollup_vk: RollupVerifyingKey,

    /// Number of Bitcoin blocks a withdrawal dispatch assignment is valid for.
    pub dispatch_assignment_dur: u32,

    /// Describes how proofs are published
    pub proof_publish_mode: ProofPublishMode,

    /// max number of deposits in a block
    pub max_deposits_in_block: u8,

    /// network the l1 is set on
    pub network: bitcoin::Network,
}

impl RollupParams {
    pub fn check_well_formed(&self) -> Result<(), ParamsError> {
        if self.horizon_l1_height > self.genesis_l1_height {
            return Err(ParamsError::HorizonAfterGenesis(
                self.horizon_l1_height,
                self.genesis_l1_height,
            ));
        }

        if self.rollup_name.is_empty() {
            return Err(ParamsError::EmptyRollupName);
        }

        match &self.operator_config {
            OperatorConfig::Static(optbl) => {
                if optbl.is_empty() {
                    return Err(ParamsError::NoOperators);
                }
            }
        }

        // TODO maybe make all these be a macro?
        if self.block_time == 0 {
            return Err(ParamsError::ZeroProperty("block_time"));
        }

        if self.l1_reorg_safe_depth == 0 {
            return Err(ParamsError::ZeroProperty("l1_reorg_safe_depth"));
        }

        if self.target_l2_batch_size == 0 {
            return Err(ParamsError::ZeroProperty("target_l2_batch_size"));
        }

        if self.address_length == 0 {
            return Err(ParamsError::ZeroProperty("max_address_length"));
        }

        if self.deposit_amount == 0 {
            return Err(ParamsError::ZeroProperty("deposit_amount"));
        }

        if self.dispatch_assignment_dur == 0 {
            return Err(ParamsError::ZeroProperty("dispatch_assignment_dur"));
        }

        if self.max_deposits_in_block == 0 {
            return Err(ParamsError::ZeroProperty("max_deposits_in_block"));
        }

        Ok(())
    }

    pub fn compute_hash(&self) -> Buf32 {
        let raw_bytes = bincode::serialize(&self).expect("rollup params serialization failed");
        crate::hash::raw(&raw_bytes)
    }

    pub const fn rollup_vk(&self) -> RollupVerifyingKey {
        self.rollup_vk
    }
}

/// Configuration common among deposit and deposit request transaction
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Deserialize, Serialize)]
pub struct DepositTxParams {
    /// Magic bytes we use to regonize a deposit with.
    pub magic_bytes: Vec<u8>,

    /// Maximum EE address length.
    // TODO rename to be `max_addr_len`
    pub address_length: u8,

    /// Exact bitcoin amount(sats) in the at-rest deposit.
    pub deposit_amount: u64,

    /// federation address derived from operator entries
    pub address: BitcoinAddress,

    /// Operators' aggregated pubkey(untweaked internal pubkey)
    pub operators_pubkey: XOnlyPk,
}

/// Describes how we decide to wait for proofs for checkpoints to generate.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProofPublishMode {
    /// Timeout in secs after which a blank proof is generated.
    Timeout(u64),

    /// Expect and wait for non-empty proofs
    Strict,
}

impl ProofPublishMode {
    pub const fn allow_empty(&self) -> bool {
        !matches!(self, Self::Strict)
    }
}

/// Client sync parameters that are used to make the network work but don't
/// strictly have to be pre-agreed.  These have to do with grace periods in
/// message delivery and whatnot.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SyncParams {
    /// Number of blocks that we follow the L1 from.
    pub l1_follow_distance: u64,

    /// Number of events after which we checkpoint the client
    pub client_checkpoint_interval: u32,

    /// Max number of recent l2 blocks that can be fetched from RPC
    pub l2_blocks_fetch_limit: u64,
}

/// Combined set of parameters across all the consensus logic.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Params {
    pub rollup: RollupParams,
    pub run: SyncParams,
}

impl Params {
    pub const fn rollup(&self) -> &RollupParams {
        &self.rollup
    }

    pub const fn run(&self) -> &SyncParams {
        &self.run
    }

    pub const fn network(&self) -> Network {
        self.rollup.network
    }
}

/// Describes how we determine the list of operators at genesis.
// TODO improve how this looks when serialized
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperatorConfig {
    /// Use this static list of predetermined operators.
    Static(Vec<OperatorPubkeys>),
}

/// Error that can arise during params validation.
#[derive(Debug, Error)]
pub enum ParamsError {
    #[error("rollup name empty")]
    EmptyRollupName,

    #[error("{0} must not be 0")]
    ZeroProperty(&'static str),

    #[error("horizon block {0} after genesis trigger block {1}")]
    HorizonAfterGenesis(u64, u64),

    #[error("no operators set")]
    NoOperators,
}

impl OperatorConfig {
    #[cfg(test)]
    pub fn get_static_operator_keys(&self) -> &[OperatorPubkeys] {
        match self {
            OperatorConfig::Static(op_keys) => op_keys.as_ref(),
        }
    }
}
