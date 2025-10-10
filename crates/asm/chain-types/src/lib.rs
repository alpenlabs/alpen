//! ASM chain-related types.

mod manifest;

pub use manifest::*;

// Note: L1BlockManifest from strata-asm-types is different from AsmManifest.
// L1BlockManifest is data extracted from L1 Bitcoin blocks.
// AsmManifest is produced from ASM execution and contains encoded data.
