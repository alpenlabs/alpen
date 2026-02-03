//! Global consensus parameters for the rollup.

use bitcoin::{Amount, Network, XOnlyPublicKey};
use serde::{Deserialize, Serialize};
use strata_btc_types::GenesisL1View;
use strata_identifiers::{Buf32, CredRule};
use strata_l1_txfmt::MagicBytes;
use strata_predicate::PredicateKey;
use thiserror::Error;

pub mod serde_helpers;

use serde_helpers::{serde_amount_sat, serde_magic_bytes};

/// Consensus parameters that don't change for the lifetime of the network
/// (unless there's some weird hard fork).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RollupParams {
    /// Rollup name
    #[serde(with = "serde_magic_bytes")]
    pub magic_bytes: MagicBytes,

    /// Block time in milliseconds.
    pub block_time: u64,

    /// Rule we use to decide if a block is correctly signed.
    pub cred_rule: CredRule,

    pub genesis_l1_view: GenesisL1View,

    /// XOnlyPublicKey of the bridge operators at genesis.
    pub operators: Vec<XOnlyPublicKey>,

    /// Hardcoded EL genesis info
    /// TODO: move elsewhere
    pub evm_genesis_block_hash: Buf32,
    pub evm_genesis_block_state_root: Buf32,

    /// Depth after which we consider the L1 block to not reorg
    pub l1_reorg_safe_depth: u32,

    /// target batch size in number of l2 blocks
    pub target_l2_batch_size: u64,

    /// Exact "at-rest" deposit amount, in sats.
    #[serde(with = "serde_amount_sat")]
    pub deposit_amount: Amount,

    /// Number of blocks after Deposit Request Transaction that the depositor can reclaim funds if
    /// operators fail to process the deposit.
    pub recovery_delay: u32,

    /// Predicate to verify the validity of checkpoint
    pub checkpoint_predicate: PredicateKey,

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
        if self.operators.is_empty() {
            return Err(ParamsError::NoOperators);
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

        if self.deposit_amount == Amount::ZERO {
            return Err(ParamsError::ZeroProperty("deposit_amount"));
        }

        if self.recovery_delay == 0 {
            return Err(ParamsError::ZeroProperty("recovery_delay"));
        }

        if self.dispatch_assignment_dur == 0 {
            return Err(ParamsError::ZeroProperty("dispatch_assignment_dur"));
        }

        if self.max_deposits_in_block == 0 {
            return Err(ParamsError::ZeroProperty("max_deposits_in_block"));
        }

        Ok(())
    }

    pub fn checkpoint_predicate(&self) -> &PredicateKey {
        &self.checkpoint_predicate
    }
}

/// Describes how we decide to wait for proofs for checkpoints to generate.
#[derive(
    Clone,
    Debug,
    PartialEq,
    Deserialize,
    Serialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ProofPublishMode {
    /// Timeout in secs after which a blank proof is generated.
    Timeout(u64),

    /// Expect and wait for non-empty proofs.
    Strict,
}

impl ProofPublishMode {
    pub fn allow_empty(&self) -> bool {
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
    pub fn rollup(&self) -> &RollupParams {
        &self.rollup
    }

    pub fn run(&self) -> &SyncParams {
        &self.run
    }

    pub fn network(&self) -> Network {
        self.rollup.network
    }
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
