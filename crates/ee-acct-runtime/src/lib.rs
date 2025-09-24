mod exec_processing;
mod process_update;
mod verification_state;

pub use process_update::{
    apply_update_operation_unconditionally, verify_and_apply_update_operation,
};
