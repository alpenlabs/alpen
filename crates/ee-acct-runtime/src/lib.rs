mod exec_processing;
mod private_input;
mod process_update;
mod verification_state;

#[cfg(feature = "builders")]
mod builder_errors;
#[cfg(feature = "builders")]
mod chain_segment_builder;
#[cfg(feature = "builders")]
mod update_builder;

#[cfg(test)]
pub mod test_utils;

pub use private_input::SharedPrivateInput;
pub use process_update::{
    apply_update_operation_unconditionally, verify_and_apply_update_operation,
};

#[cfg(feature = "builders")]
pub use builder_errors::{BuilderError, BuilderResult};
#[cfg(feature = "builders")]
pub use chain_segment_builder::ChainSegmentBuilder;
#[cfg(feature = "builders")]
pub use update_builder::UpdateBuilder;
