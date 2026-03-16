//! Wraps the ASM STF with MohoProgram

mod input;
mod program;

#[allow(
    clippy::all,
    unreachable_pub,
    clippy::allow_attributes,
    clippy::absolute_paths,
    reason = "generated code"
)]
mod ssz_generated {
    include!(concat!(env!("OUT_DIR"), "/generated.rs"));
}

pub use input::L1Block;
pub use moho_runtime_interface::MohoProgram;
pub use program::AsmStfProgram;
pub use ssz_generated::ssz::input::{AsmStepInput, AsmStepInputRef, AuxDataBytes, BlockBytes};
