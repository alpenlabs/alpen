mod exec_processing;
mod private_input;
mod process_update;
mod verification_state;

pub use private_input::SharedPrivateInput;
pub use process_update::{
    apply_update_operation_unconditionally, verify_and_apply_update_operation,
};
