//! EE Data Availability publisher implementation.
//!
//! This module provides [`EeDaPublisher`], which implements [`BatchDaProvider`]
//! for publishing batch DA using the chunked blob system.
//!
//! # Architecture
//!
//! The publisher bridges the batch lifecycle with the low-level chunked
//! blob publication system:
//!
//! ```text
//! BatchLifecycle → EeDaPublisher → ChunkedBlobPublisher
//!                       ↓
//!                  DaTracker
//! ```
//!
//! - [`BatchDaProvider`] interface is used by the batch lifecycle manager
//! - [`ChunkedBlobPublisher`] is injected for actual L1 publication
//! - [`DaTracker`] manages the BatchId → blob_hash mapping

mod publisher;
mod tracker;

pub use publisher::EeDaPublisher;
pub use tracker::DaTracker;
