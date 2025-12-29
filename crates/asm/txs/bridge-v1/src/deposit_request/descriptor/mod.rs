//! Deposit Descriptor

mod format;
mod subject_id_bytes;

pub use format::{
    DepositDescriptor, DepositDescriptorError, MAX_DESCRIPTOR_LEN, MAX_SERIAL_VALUE,
    MIN_DESCRIPTOR_LEN,
};
pub use subject_id_bytes::SubjectIdBytes;
