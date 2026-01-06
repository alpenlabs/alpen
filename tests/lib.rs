//! Integration test utilities
//!
//! This module exposes common test utilities to all integration test binaries.

pub mod common;

// Suppress unused extern crate warnings - these are used by test binaries
// This centralized list prevents each test file from needing duplicate suppressions
use anyhow as _;
use bitcoind_async_client as _;
use borsh as _;
use corepc_node as _;
use rand as _;
use rand_chacha as _;
use strata_asm_common as _;
use strata_asm_manifest_types as _;
use strata_asm_proto_administration as _;
use strata_asm_proto_checkpoint_v0 as _;
use strata_asm_txs_admin as _;
use strata_asm_worker as _;
use strata_bridge_types as _;
use strata_btc_types as _;
use strata_crypto as _;
use strata_l1_txfmt as _;
use strata_merkle as _;
use strata_params as _;
use strata_predicate as _;
use strata_state as _;
use strata_tasks as _;
use strata_test_utils_btcio as _;
use strata_test_utils_l2 as _;
