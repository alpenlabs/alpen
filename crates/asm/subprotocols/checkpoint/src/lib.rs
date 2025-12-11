//! Checkpoint Subprotocol
//!
//! This module implements the checkpoint subprotocol, providing
//! verification of OL STF checkpoints including:
//!
//! - Signature verification using the sequencer credential
//! - State transition validation (epoch, slot, L1 height progression)
//! - Extraction of OL DA & OL Logs
//! - Construction of required public parameters for checkpoint zk proof verification
//! - L1→L2 message range verification (accessed via Auxiliary Data)
//! - ZK proof verification using the checkpoint predicate
//! - Forwarding of withdrawal intents to the bridge subprotocol
//! - Processing sequencer pk and checkpoint predicate updates via inter-protocol messages from the
//!   admin subprotocol
//!
//! The checkpoint subprotocol processes checkpoint transactions (SPS50 tagged tx) from the
//! L1, verifies them using auxiliary data and proof verification,
//! then updates the checkpoint subprotocol state and emits appropriate logs.

// Suppress unused dev-dependency warnings (used only in integration tests)
#[cfg(test)]
mod test_deps {
    use anyhow as _;
    use async_trait as _;
    use bitcoin as _;
    use bitcoind_async_client as _;
    use corepc_node as _;
    use rand as _;
    use ssz as _;
    use strata_asm_spec as _;
    use strata_asm_stf as _;
    use strata_asm_types as _;
    use strata_asm_worker as _;
    use strata_params as _;
    use strata_service as _;
    use strata_state as _;
    use strata_test_utils_btcio as _;
    use strata_test_utils_l2 as _;
    use tokio as _;
}

mod error;
mod handler;
mod msg_handler;
mod state;
mod subprotocol;
mod verification;

pub use state::{CheckpointConfig, CheckpointState};
pub use subprotocol::CheckpointSubprotocol;
