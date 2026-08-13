//! Building and persisting RBF replacements for writer-published transactions.
//!
//! Replacement is writer work, not a service of its own: the writer is what understands the
//! commit/reveal structure of an envelope and what holds the keys to re-sign one. The broadcaster
//! decides *that* a transaction is stale (see [`crate::broadcaster::fee_bump`]); each writer's
//! existing watcher tick drives the rebuild through [`driver`].

pub(crate) mod build;
pub(crate) mod driver;
