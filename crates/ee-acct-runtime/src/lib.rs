//! Basic EE account runtime framework.

#![cfg_attr(test, expect(unused_crate_dependencies, reason = "test weirdness"))]
#![expect(unused, reason = "lots of stuff being refactored")]

mod block_assembly;
mod commit;
mod ee_program;
mod errors;
mod exec_processing;
mod private_input;
mod update_processing;
mod verification_state;

pub use block_assembly::apply_input_messages;
pub use ee_program::EeSnarkAccountProgram;
pub use private_input::SharedPrivateInput;
pub use update_processing::{process_update_unconditionally, verify_and_process_update};
pub use verification_state::{EeVerificationInput, EeVerificationState};

// Builder utils
//
// These rely on runtime functions, so they have to be in this crate.  Unless we
// decide to reorganize the module hierarchy.

#[cfg(feature = "builders")]
mod builder_errors;
#[cfg(feature = "builders")]
mod chain_segment_builder;
#[cfg(feature = "builders")]
mod update_builder;

#[cfg(feature = "builders")]
mod builder_reexports {
    pub use super::{
        builder_errors::{BuilderError, BuilderResult},
        chain_segment_builder::ChainSegmentBuilder,
        update_builder::UpdateBuilder,
    };
}

#[cfg(feature = "builders")]
pub use builder_reexports::*;
