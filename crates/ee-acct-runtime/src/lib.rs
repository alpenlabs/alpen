mod accumulator;
mod process_update;

pub use process_update::{
    apply_update_operation_unconditionally, verify_and_apply_update_operation,
};
