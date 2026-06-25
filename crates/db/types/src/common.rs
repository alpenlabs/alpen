//! Shared database identifier aliases used across multiple trait families.

use strata_identifiers::RBuf32;

/// Bitcoin transaction ID displayed in Bitcoin byte order.
pub type L1TxId = RBuf32;

/// Bitcoin witness transaction ID displayed in Bitcoin byte order.
pub type L1WtxId = RBuf32;

/// Index into the L1 payload intent store.
pub type L1PayloadIntentIndex = u64;
