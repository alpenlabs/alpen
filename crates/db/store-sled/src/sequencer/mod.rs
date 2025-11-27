//! Sequencer-specific database storage.
//!
//! This module provides storage for sequencer-specific data such as exec payloads,
//! which are used for EE consistency recovery on startup.

pub mod db;
pub(crate) mod schemas;
