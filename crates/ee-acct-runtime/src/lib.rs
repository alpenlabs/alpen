//! Basic EE account runtime framework.
//!
//! This is expected to be used within the SNARK proof program used as a snark
//! account in order to implement an execution environment host account on the
//! Strata orchestration layer.  There are a collection of utilities in order to
//! help manipulate EE accounts like by building chain segments and updates.

#![cfg_attr(test, expect(unused_crate_dependencies, reason = "test weirdness"))]

mod block_assembly;
mod errors;
mod exec_processing;
mod private_input;
mod traits;
mod update_processing;
mod verification_state;

pub use block_assembly::apply_input_messages;
pub use errors::*;
pub use private_input::SharedPrivateInput;
pub use traits::*;
pub use update_processing::{
    MsgData, MsgMeta, apply_final_update_changes, apply_update_operation_unconditionally,
    verify_and_apply_update_operation,
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
