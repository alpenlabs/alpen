//! Types relating to things we find or generate from Bitcoin blocks/txs/etc.

mod block;
mod btc;
mod errors;
mod genesis;
mod header;
mod ops;
mod params;
pub mod payload;
mod proof;

pub use block::*;
pub use btc::*;
pub use errors::*;
pub use genesis::*;
pub use header::*;
pub use ops::*;
pub use params::*;
pub use proof::*;

/// L1 block height type.
pub type L1Height = u32;
