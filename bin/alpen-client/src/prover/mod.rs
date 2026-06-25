//! EE chunk + acct proof generation, backed by paas.
//!
//! Two `ProofSpec`s — one per proof kind — each driven by its own paas
//! `Prover`. A thin [`PaasEeProver`] wraps both handles and implements
//! chunk and acct/batch proof submission interfaces.
//!
//! ```text
//!          chunk proof lifecycle
//!                  │
//!                  │ sealed ChunkId
//!                  ▼
//!         ┌────────────────────────────┐
//!         │ Prover<ChunkSpec>          │
//!         │ fetch_input(ChunkId):      │
//!         │   chunk blocks + prev state│
//!         └─────┬──────────────────────┘
//!               │ chunk receipts in shared paas ReceiptStore
//!               │ hook: flip ChunkStatus::ProofReady
//!               │
//!          batch lifecycle asks PaasEeProver for acct proof
//!               │
//!               ▼
//!         PaasEeProver checks acct input readiness
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

mod ee_prover;
mod hooks;
mod spec_acct;
mod spec_chunk;
mod storage;

pub(crate) use ee_prover::PaasEeProver;
pub(crate) use hooks::{AcctReceiptHook, ChunkReceiptHook};
pub(crate) use spec_acct::{AcctRangeWitnessFn, AcctSpec, BatchTask};
pub(crate) use spec_chunk::{ChunkSpec, ChunkTask};
pub(crate) use storage::{EeBatchProofDbManager, EeChunkReceiptStore, EeProverTaskDbManager};
