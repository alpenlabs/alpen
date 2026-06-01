//! Global consensus parameters for the rollup.

use bitcoin::{Amount, Network, XOnlyPublicKey};
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_btc_types::GenesisL1View;
use strata_identifiers::{Buf32, L1Height};
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
    pub recovery_delay: u16,

    /// Predicate to verify the validity of checkpoint
    pub checkpoint_predicate: PredicateKey,

    /// Number of Bitcoin blocks a withdrawal dispatch assignment is valid for.
    pub dispatch_assignment_dur: u16,

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
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, BorshSerialize, BorshDeserialize)]
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

/// Rule we use to decide how to identify if an L2 block is correctly signed.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, BorshSerialize, BorshDeserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredRule {
    /// Any block gets accepted, unconditionally.
    Unchecked,

    /// Just sign every block with a static BIP340 schnorr pubkey.
    SchnorrKey(Buf32),
}

impl CredRule {
    /// Returns the Schnorr key if the variant is [`CredRule::SchnorrKey`], otherwise `None`.
    pub fn schnorr_key(&self) -> Option<&Buf32> {
        match self {
            Self::SchnorrKey(key) => Some(key),
            Self::Unchecked => None,
        }
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

/// Number of confirmations an L1 block has under `current_tip`, counting the
/// block itself as one confirmation.
///
/// A block at the tip has 1 confirmation; one block below tip has 2; etc.
/// Observation heights above the tip saturate to 0.
pub fn l1_confirmations(observed_height: L1Height, current_tip: L1Height) -> u32 {
    if observed_height > current_tip {
        return 0;
    }
    current_tip
        .saturating_sub(observed_height)
        .saturating_add(1)
}

/// A single computation logic for whether an L1 block at `observed_height` is buried deep enough
/// under `current_tip` to be considered reorg-safe.
pub fn is_l1_reorg_safe(
    observed_height: L1Height,
    current_tip: L1Height,
    l1_reorg_safe_depth: u32,
) -> bool {
    l1_confirmations(observed_height, current_tip) >= l1_reorg_safe_depth.max(1)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirmations_at_tip_is_one() {
        assert_eq!(l1_confirmations(100, 100), 1);
    }

    #[test]
    fn confirmations_grows_with_burial() {
        assert_eq!(l1_confirmations(98, 100), 3);
    }

    #[test]
    fn confirmations_above_tip_is_zero() {
        assert_eq!(l1_confirmations(101, 100), 0);
    }

    #[test]
    fn reorg_safe_exactly_at_threshold() {
        // depth=3 means: need >= 3 confirmations. tip=102, obs=100 => 3 confs.
        assert!(is_l1_reorg_safe(100, 102, 3));
    }

    #[test]
    fn reorg_safe_one_below_threshold() {
        // tip=101, obs=100 => 2 confs, depth=3 not satisfied.
        assert!(!is_l1_reorg_safe(100, 101, 3));
    }

    #[test]
    fn reorg_safe_depth_zero_clamped_to_one() {
        // depth=0 must not mark the tip block trivially safe.
        // tip=100, obs=100 => 1 conf, clamped depth=1 satisfied.
        assert!(is_l1_reorg_safe(100, 100, 0));
        // But obs above tip never qualifies.
        assert!(!is_l1_reorg_safe(101, 100, 0));
    }
}
