//! Types relating to things we find or generate from Bitcoin blocks/txs/etc.

mod block;
mod btc;
mod errors;
mod genesis;
mod header;
mod inclusion_proof;
mod ops;
mod params;
pub mod payload;
mod proof;
mod tx;
pub mod utils;
mod utils_generate;

pub use block::*;
pub use btc::*;
pub use errors::*;
pub use genesis::*;
pub use header::*;
pub use inclusion_proof::*;
pub use ops::*;
pub use params::*;
pub use proof::*;
pub use tx::*;
pub use utils::*;

/// L1 block height type.
pub type L1Height = u32;

#[rustfmt::skip]
#[cfg(feature = "bitcoin")]
pub use utils_generate::*;
