//! Receipt persistence and post-prove hooks.
//!
//! Both are opt-in via [`ProverBuilder`](crate::ProverBuilder):
//! - [`ReceiptStore`]: generic byte-keyed persistence. Enables `get_receipt` on the PaaS handle.
//! - [`ReceiptHook`]: domain-specific side effects after proving (e.g. write to ProofDB by epoch).

use std::collections::HashMap;

use async_trait::async_trait;
use parking_lot::RwLock;
use zkaleido::ProofReceiptWithMetadata;

use crate::{error::ProverResult, spec::ProofSpec};

/// Generic receipt persistence keyed by task bytes.
///
/// Implement this for your storage backend (sled, memory, etc.).
/// When provided to the builder, prover-core auto-stores receipts after proving
/// and PaaS exposes `get_receipt(task)` on the handle.
pub trait ReceiptStore: Send + Sync + 'static {
    fn put(&self, key: &[u8], receipt: &ProofReceiptWithMetadata) -> ProverResult<()>;
    fn get(&self, key: &[u8]) -> ProverResult<Option<ProofReceiptWithMetadata>>;
}

/// Domain-specific hook called after a receipt is stored.
///
/// Gets the typed task (not just key bytes), so it can write to domain-specific
/// storage keyed by task identity (e.g. ProofDB keyed by `Epoch`).
///
/// Most consumers don't need this — only use it when you have a secondary
/// storage that needs the domain task for its key.
#[async_trait]
pub trait ReceiptHook<H: ProofSpec>: Send + Sync + 'static {
    async fn on_receipt(
        &self,
        task: &H::Task,
        receipt: &ProofReceiptWithMetadata,
    ) -> ProverResult<()>;
}

/// In-memory receipt store for tests and dev.
#[derive(Debug, Default)]
pub struct InMemoryReceiptStore {
    receipts: RwLock<HashMap<Vec<u8>, ProofReceiptWithMetadata>>,
}

impl InMemoryReceiptStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl ReceiptStore for InMemoryReceiptStore {
    fn put(&self, key: &[u8], receipt: &ProofReceiptWithMetadata) -> ProverResult<()> {
        self.receipts
            .write()
            .insert(key.to_vec(), receipt.clone());
        Ok(())
    }

    fn get(&self, key: &[u8]) -> ProverResult<Option<ProofReceiptWithMetadata>> {
        Ok(self.receipts.read().get(key).cloned())
    }
}
