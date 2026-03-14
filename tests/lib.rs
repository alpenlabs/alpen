//! Integration test utilities
//!
//! This module exposes common test utilities to all integration test binaries.

pub mod harness;

// Suppress unused extern crate warnings - these are used by test binaries
// This centralized list prevents each test file from needing duplicate suppressions
use anyhow as _;
use bitcoin_bosd as _;
use bitcoind_async_client as _;
use borsh as _;
use corepc_node as _;
use rand as _;
use rand_chacha as _;
use ssz as _;
use strata_asm_common as _;
use strata_asm_manifest_types as _;
use strata_asm_proto_administration as _;
use strata_asm_proto_checkpoint_v0 as _;
use strata_asm_txs_admin as _;
use strata_asm_worker as _;
use strata_bridge_types as _;
use strata_btc_types as _;
use strata_checkpoint_types_ssz as _;
use strata_codec_utils as _;
use strata_crypto as _;
use strata_db_types as _;
use strata_identifiers as _;
use strata_l1_txfmt as _;
use strata_merkle as _;
use strata_ol_chain_types_new as _;
use strata_ol_stf as _;
use strata_params as _;
use strata_predicate as _;
use strata_state as _;
use strata_tasks as _;
use strata_test_utils_btcio as _;
use strata_test_utils_l2 as _;
