#![cfg_attr(test, expect(unused_crate_dependencies, reason = "test weirdness"))]
#![expect(unused, reason = "in-development")]
#![expect(unreachable_pub, reason = "in-development")]

mod exec_processing;
mod private_input;
mod process_update;
mod verification_state;

pub use private_input::SharedPrivateInput;
pub use process_update::{
    apply_update_operation_unconditionally, verify_and_apply_update_operation,
};

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
