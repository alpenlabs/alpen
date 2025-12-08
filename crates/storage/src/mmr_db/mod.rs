//! MMR Database for Persistent Proof Generation
//!
//! This module provides a database layer for MMR (Merkle Mountain Range) data
//! that enables on-demand proof generation for arbitrary leaf positions.
//!
//! ## Problem
//!
//! `CompactMmr` only stores peak hashes for efficient storage but cannot generate
//! proofs for historical positions. This module solves that by maintaining enough
//! MMR data to generate proofs on-demand.
//!
//! ## Usage
//!
//! ```ignore
//! use strata_storage::mmr_db::{MmrDatabase, InMemoryMmrDb};
//!
//! // Create database
//! let mut db = InMemoryMmrDb::new();
//!
//! // Append leaves
//! let hash1 = [1u8; 32];
//! db.append_leaf(hash1)?;
//!
//! // Generate proof
//! let proof = db.generate_proof(0)?;
//!
//! // Verify proof against root
//! let root = db.root();
//! assert!(verify_proof(&hash1, &proof, &root));
//! ```

mod helpers;
mod memory;
mod sled;
mod types;

pub use memory::InMemoryMmrDb;
pub use sled::SledMmrDb;
pub use types::{MmrDatabase, MmrDbError, MmrDbResult};
