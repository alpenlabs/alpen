//! EE chunk + acct proof generation, backed by paas.
//!
//! Two `ProofSpec`s — one per proof kind — each driven by its own paas
//! `Prover`. A thin [`PaasBatchProver`] wraps both handles and implements
//! [`alpen_ee_common::BatchProver`], the integration seam the existing
//! `batch_lifecycle` task already drives.
//!
//! ```text
//!                 (concurrent per chunk)
//!         ┌────────────────────────────┐
//!         │ Prover<ChunkSpec>          │
//!         │ fetch_input(ChunkId):      │
//!         │   chunk blocks + prev state│
//!         └─────┬──────────────────────┘
//!               │ chunk receipts in shared paas ReceiptStore
//!               │
//!               ▼
//!         ┌────────────────────────────┐
//!         │ Prover<AcctSpec>           │
//!         │ fetch_input(BatchId):      │
//!         │   chunk receipts +         │
//!         │   prev-batch end state     │
//!         └────────────────────────────┘
//!                            │
//!                  hook: write proof to
//!                  EeBatchProofDbManager,
//!                  flip BatchStatus::ProofReady
//! ```

mod batch_prover;
mod hooks;
mod spec_acct;
mod spec_chunk;
mod storage;

pub(crate) use batch_prover::PaasBatchProver;
pub(crate) use hooks::{AcctReceiptHook, ChunkReceiptHook};
pub(crate) use spec_acct::{AcctSpec, BatchTask};
pub(crate) use spec_chunk::{ChunkSpec, ChunkTask, RangeWitnessFn};
pub(crate) use storage::{EeBatchProofDbManager, EeChunkReceiptStore, EeProverTaskDbManager};
