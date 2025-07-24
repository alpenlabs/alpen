//! Transaction handlers for the Core subprotocol
//!
//! This module contains handlers for different transaction types processed by the Core subprotocol.

mod checkpoint;

pub(crate) use checkpoint::handle_checkpoint_transaction;
