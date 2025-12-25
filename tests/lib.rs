//! Integration test utilities
//!
//! This module exposes common test utilities to all integration test binaries.

pub mod common;

// Suppress unused extern crate warnings - these are used by test binaries
use borsh as _;
use rand as _;
use rand_chacha as _;
use strata_asm_common as _;
use strata_asm_manifest_types as _;
use strata_asm_proto_administration as _;
use strata_asm_txs_admin as _;
use strata_crypto as _;
use strata_tasks as _;
use strata_test_utils_l2 as _;
