//! Constants used by EE DA proof verification.

/// Magic bytes in the DA commit transaction marker output.
pub(super) const EE_DA_MAGIC_BYTES: [u8; 4] = *b"ALPN";

/// Current EE DA blob encoding version.
pub(super) const DA_BLOB_VERSION: u32 = 0;
