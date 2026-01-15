//! Deposit Descriptor

mod format;

pub use format::{
    DepositDescriptor, DepositDescriptorError, MAX_DESCRIPTOR_LEN, MAX_SERIAL_VALUE,
    MIN_DESCRIPTOR_LEN,
};
