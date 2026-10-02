pub(crate) mod codec;
pub mod db;
pub mod schemas;

use typed_sled::Schema;

pub use db::*;

/// Sled tables removed when resetting ASM for replay; excludes L1 manifests and MMR data.
pub const ASM_REPLAY_TABLE_NAMES: [&str; 3] = [
    <schemas::AsmStateSchema as Schema>::TREE_NAME.0,
    <schemas::AsmLogSchema as Schema>::TREE_NAME.0,
    <schemas::AsmAuxDataSchema as Schema>::TREE_NAME.0,
];
